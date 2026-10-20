//! Explicit, local-only Claude/Codex diagnostic export. Never serialize collector/cache
//! structs here: they contain paths and free-form strings. This is a positive allowlist.
use super::{
    scan::{self, CachedFile, Record},
    UsageStore,
};
use crate::{cloud::records::hash, pricing::TokenBreakdown};
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use serde::Serialize;
use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
};
use tauri::Manager;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticEvent {
    provider: &'static str,
    // Exactly the identifier sent to the backend, so support can match without raw IDs.
    cloud_event_id: String,
    model_hash: String,
    occurred_at: String,
    tokens: u64,
    cost_usd: Option<f64>,
    token_breakdown: Option<TokenBreakdown>,
    message_hash: Option<String>,
    session_hash: Option<String>,
    // Codex's identity suffix is normally cumulative tokens, but can be a line number.
    identity_counter: Option<u64>,
    sidechain: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_label: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    schema_version: u8,
    app_version: &'static str,
    platform: &'static str,
    generated_at: String,
    from_utc: String,
    through_utc: String,
    scan_warning_count: usize,
    source_file_count: usize,
    occurrences_before_dedup: usize,
    events_after_dedup: usize,
    tokens_after_dedup: u64,
    // Both arrays contain only the requested UTC date range, not all retained history.
    occurrences: Vec<DiagnosticEvent>,
    events: Vec<DiagnosticEvent>,
}

struct DateRange {
    from: NaiveDate,
    through: NaiveDate,
}
impl DateRange {
    fn parse(from: &str, through: &str) -> Result<Self, String> {
        let parse = |value: &str| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .filter(|date| date.to_string() == value)
        };
        let (Some(from), Some(through)) = (parse(from), parse(through)) else {
            return Err("Choose valid UTC dates for the export.".into());
        };
        if through < from || (through - from).num_days() >= 90 || through > Utc::now().date_naive()
        {
            return Err("Choose a range of up to 90 days ending today or earlier.".into());
        }
        Ok(Self { from, through })
    }
    fn timestamp(&self, value: &str) -> Option<String> {
        let utc = DateTime::parse_from_rfc3339(value)
            .ok()?
            .with_timezone(&Utc);
        (utc.date_naive() >= self.from && utc.date_naive() <= self.through)
            .then(|| utc.to_rfc3339_opts(SecondsFormat::Millis, true))
    }
}

fn metadata(
    record: &Record,
    range: &DateRange,
    file_label: Option<String>,
) -> Option<DiagnosticEvent> {
    let event = &record.event;
    let provider = match event.provider.as_str() {
        "claude" => "Claude",
        "codex" => "Codex",
        _ => return None,
    };
    let occurred_at = range.timestamp(&event.timestamp)?;
    let identity = if provider == "Codex" {
        event.id.rsplit_once(':').and_then(|(prefix, counter)| {
            prefix
                .strip_suffix(&format!(":{}", event.timestamp))
                .map(|session| {
                    (
                        hash(&format!("Codex:{session}")),
                        counter.parse::<u64>().ok(),
                    )
                })
        })
    } else {
        None
    };
    Some(DiagnosticEvent {
        provider,
        cloud_event_id: hash(&format!("{provider}:{}", event.id)),
        // A custom model string could contain private text. Hash it instead of guessing.
        model_hash: hash(&event.model),
        occurred_at,
        tokens: event.tokens,
        cost_usd: event
            .cost_usd
            .filter(|cost| cost.is_finite() && *cost >= 0.),
        token_breakdown: record.breakdown,
        message_hash: record
            .message
            .as_ref()
            .map(|id| hash(&format!("{provider}:{id}"))),
        session_hash: identity.as_ref().map(|(session, _)| session.clone()),
        identity_counter: identity.and_then(|(_, counter)| counter),
        sidechain: record.sidechain,
        file_label,
    })
}

fn report(
    files: &HashMap<PathBuf, CachedFile>,
    range: &DateRange,
    pricing: &crate::pricing::ModelPricing,
    warnings: usize,
) -> Report {
    let mut files = files.iter().collect::<Vec<_>>();
    files.sort_by_key(|(path, _)| *path);
    let mut records = Vec::new();
    let mut occurrences = Vec::new();
    let mut source_file_count = 0;
    for (_, file) in files {
        let label = format!("file-{}", source_file_count + 1);
        let start = occurrences.len();
        for record in &file.records {
            let mut priced = record.clone();
            priced.event = record.priced(pricing);
            if let Some(row) = metadata(&priced, range, Some(label.clone())) {
                occurrences.push(row);
            }
            records.push(record.clone());
        }
        if occurrences.len() > start {
            source_file_count += 1;
        }
    }
    // Deduplicate the complete scan first, exactly as collection does, then limit dates.
    let events = scan::dedup(records)
        .iter()
        .filter_map(|record| {
            let mut priced = record.clone();
            priced.event = record.priced(pricing);
            metadata(&priced, range, None)
        })
        .collect::<Vec<_>>();
    Report {
        schema_version: 1,
        app_version: env!("CARGO_PKG_VERSION"),
        platform: std::env::consts::OS,
        generated_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        from_utc: range.from.to_string(),
        through_utc: range.through.to_string(),
        scan_warning_count: warnings,
        source_file_count,
        occurrences_before_dedup: occurrences.len(),
        events_after_dedup: events.len(),
        tokens_after_dedup: events.iter().map(|event| event.tokens).sum(),
        occurrences,
        events,
    }
}

fn save(report: &Report, directory: &Path) -> Result<String, String> {
    // No caller-supplied paths, no overwrites, and private permissions on Unix.
    let name = format!(
        "devclash-usage-diagnostics-{}.json",
        Utc::now().format("%Y%m%dT%H%M%S%.9fZ")
    );
    let path = directory.join(&name);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|error| {
        eprintln!("Diagnostic export file creation failed: {error}");
        "Could not save diagnostics to Downloads. Check folder access and free space.".to_string()
    })?;
    let written = serde_json::to_writer(&mut file, report)
        .map_err(|error| error.to_string())
        .and_then(|_| {
            file.flush()
                .and_then(|_| file.sync_all())
                .map_err(|error| error.to_string())
        });
    drop(file);
    if let Err(error) = written {
        eprintln!("Diagnostic export write failed: {error}");
        let _ = std::fs::remove_file(path);
        return Err("Diagnostics could not be saved. Check free space and try again.".into());
    }
    Ok(name)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReceipt {
    filename: String,
    event_count: usize,
    warning_count: usize,
}

#[tauri::command]
pub async fn export_usage_diagnostics(
    app: tauri::AppHandle,
    from: String,
    through: String,
) -> Result<ExportReceipt, String> {
    let range = DateRange::parse(&from, &through)?;
    tauri::async_runtime::spawn_blocking(move || {
        if crate::paths::configuration_error().is_some() {
            return Err(
                "Usage source settings could not be read. Fix Usage Sources before exporting."
                    .into(),
            );
        }
        let pricing = app.state::<UsageStore>().pricing.cached()?;
        let mut files = HashMap::new();
        let snapshot = scan::diagnostic_scan(&mut files, &pricing);
        let report = report(&files, &range, &pricing, snapshot.errors.len());
        let directory = dirs::download_dir().ok_or("Downloads folder could not be located.")?;
        let filename = save(&report, &directory)?;
        Ok(ExportReceipt {
            filename,
            event_count: report.events_after_dedup,
            warning_count: report.scan_warning_count,
        })
    })
    .await
    .map_err(|error| {
        eprintln!("Diagnostic export worker failed: {error}");
        "Diagnostics could not be prepared. Try again.".to_string()
    })?
}

#[tauri::command]
pub async fn open_diagnostics_folder() -> Result<(), String> {
    let directory = dirs::download_dir().ok_or("Downloads folder could not be located.")?;
    open::that(directory).map_err(|error| {
        eprintln!("Diagnostic folder open failed: {error}");
        "Could not open Downloads. Open it from your file manager.".into()
    })
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
