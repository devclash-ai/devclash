use super::{CostSource, Snapshot, UsageEvent};
use crate::pricing::{ModelPricing, TokenBreakdown};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use walkdir::WalkDir;

#[derive(Clone, Serialize, Deserialize)]
pub struct Record {
    pub event: UsageEvent,
    pub sidechain: bool,
    pub message: Option<String>,
    #[serde(default)]
    pub breakdown: Option<TokenBreakdown>,
}
impl Record {
    pub fn plain(event: UsageEvent) -> Self {
        Self {
            event,
            sidechain: false,
            message: None,
            breakdown: None,
        }
    }
    pub fn with_breakdown(mut self, breakdown: TokenBreakdown) -> Self {
        self.breakdown = Some(breakdown);
        self
    }
    pub fn priced(&self, pricing: &ModelPricing) -> UsageEvent {
        let mut event = self.event.clone();
        if event.cost_usd.is_some() {
            event.cost_source = Some(CostSource::Recorded);
        } else {
            event.cost_usd = self.breakdown.and_then(|tokens| {
                pricing.estimate(&event.provider, &event.model, tokens, &event.timestamp)
            });
            event.cost_source = event.cost_usd.map(|_| CostSource::Estimated);
        }
        event.unpriced_tokens = if event.cost_usd.is_none() {
            event.tokens
        } else {
            0
        };
        event
    }
}
#[derive(Serialize, Deserialize)]
pub struct CachedFile {
    size: u64,
    modified: u128,
    pub(super) records: Vec<Record>,
}

fn roots() -> (Vec<(&'static str, PathBuf)>, Vec<String>) {
    let mut result = Vec::new();
    let (configs, claude_auto) = crate::paths::configured_with_discovery("claude");
    let (codex_configs, codex_auto) = crate::paths::configured_with_discovery("codex");
    #[cfg(target_os = "macos")]
    let (configs, errors) = {
        let home = dirs::home_dir().unwrap_or_default();
        super::claude_roots::discover(&home, &configs, claude_auto)
    };
    #[cfg(not(target_os = "macos"))]
    let errors = Vec::new();
    for root in configs {
        result.push((
            "claude",
            if root.file_name().is_some_and(|name| name == "projects") {
                root
            } else {
                root.join("projects")
            },
        ));
    }
    for root in codex_configs {
        result.push(("codex", root.join("sessions")));
        result.push(("codex", root.join("archived_sessions")));
    }
    let mut errors = errors;
    if let Some(home) = dirs::home_dir() {
        let discovered = super::synara_roots::discover(
            &home,
            std::env::var("SYNARA_HOME").ok().as_deref(),
            claude_auto,
            codex_auto,
        );
        result.extend(discovered.roots);
        errors.extend(discovered.errors);
    }
    (result, errors)
}
// Export uses the same local log discovery/parsers without quotas, credentials or API calls.
pub(super) fn diagnostic_scan(
    cache: &mut HashMap<PathBuf, CachedFile>,
    pricing: &ModelPricing,
) -> Snapshot {
    let (roots, errors) = roots();
    scan_roots(cache, pricing, roots, errors, false)
}

pub fn scan(cache: &mut HashMap<PathBuf, CachedFile>, pricing: &ModelPricing) -> Snapshot {
    let (roots, errors) = roots();
    scan_roots(cache, pricing, roots, errors, true)
}

pub(super) fn scan_roots(
    cache: &mut HashMap<PathBuf, CachedFile>,
    pricing: &ModelPricing,
    roots: Vec<super::synara_roots::Root>,
    mut errors: Vec<String>,
    include_opencode: bool,
) -> Snapshot {
    let roots = super::synara_roots::canonical_roots(roots, &mut errors);
    let mut snapshot = Snapshot::default();
    let mut seen = HashSet::new();
    let mut ordered_paths = Vec::new();
    let since = Utc::now() - Duration::days(90);
    snapshot.errors.extend(errors);
    for (provider, root) in roots {
        if !root.exists() {
            continue;
        }
        if !snapshot
            .available_providers
            .iter()
            .any(|value| value == provider)
        {
            snapshot.available_providers.push(provider.into());
        }
        let mut failed = 0;
        let mut files = WalkDir::new(root)
            .max_depth(12)
            .follow_links(false)
            .into_iter()
            .filter_map(|entry| match entry {
                Ok(entry) => Some(entry),
                Err(_) => {
                    failed += 1;
                    None
                }
            })
            .filter(|entry| {
                entry.file_type().is_file()
                    && entry.path().extension().is_some_and(|ext| ext == "jsonl")
            })
            .map(|entry| entry.into_path())
            .collect::<Vec<_>>();
        files.sort();
        for path in files {
            if !seen.insert(path.clone()) {
                continue;
            }
            let Ok(metadata) = path.metadata() else {
                failed += 1;
                continue;
            };
            let modified = metadata
                .modified()
                .ok()
                .and_then(|date| date.duration_since(UNIX_EPOCH).ok())
                .map(|value| value.as_nanos())
                .unwrap_or(0);
            if modified / 1_000_000_000 < since.timestamp().max(0) as u128 {
                seen.remove(&path);
                continue;
            }
            let changed = cache
                .get(&path)
                .is_none_or(|old| old.size != metadata.len() || old.modified != modified);
            if changed {
                let parsed = if provider == "claude" {
                    super::claude::parse(&path)
                } else {
                    super::codex::parse(&path)
                };
                match parsed {
                    Ok(parsed) => {
                        let mut projects = HashSet::new();
                        let recent = parsed
                            .into_iter()
                            .filter(|record| {
                                DateTime::parse_from_rfc3339(&record.event.timestamp)
                                    .is_ok_and(|date| date >= since)
                            })
                            .map(|mut record| {
                                if let Some(project) = &mut record.event.project {
                                    project.share(&mut projects);
                                }
                                record
                            })
                            .collect();
                        cache.insert(
                            path.clone(),
                            CachedFile {
                                size: metadata.len(),
                                modified,
                                records: recent,
                            },
                        );
                    }
                    Err(_) => failed += 1,
                }
            }
            ordered_paths.push(path);
        }
        if failed > 0 {
            eprintln!("{provider}: {failed} local usage files could not be read");
            snapshot.errors.push(format!(
                "Some {provider} usage files could not be read. Check access to your local logs."
            ));
        }
    }
    cache.retain(|path, _| seen.contains(path));
    let mut opencode_records = Vec::new();
    if include_opencode {
        let opencode = super::opencode::scan(since);
        if opencode.available {
            snapshot.available_providers.push("opencode".into());
        }
        snapshot.errors.extend(opencode.errors);
        opencode_records = opencode.records;
    }
    let records = ordered_paths
        .iter()
        .filter_map(|path| cache.get(path))
        .flat_map(|file| file.records.iter())
        .chain(opencode_records.iter());
    snapshot.events = dedup_refs(records)
        .into_iter()
        .map(|record| record.priced(pricing))
        .collect();
    snapshot
}

pub fn dedup(records: Vec<Record>) -> Vec<Record> {
    let mut by_id: HashMap<(String, String), Record> = HashMap::new();
    for record in records {
        let key = (record.event.provider.clone(), record.event.id.clone());
        match by_id.get_mut(&key) {
            Some(old) if preferred(&record, old) => *old = record,
            None => {
                by_id.insert(key, record);
            }
            _ => {}
        }
    }
    let mut by_message: HashMap<String, Record> = HashMap::new();
    let mut result = Vec::new();
    let mut ordered = by_id.into_values().collect::<Vec<_>>();
    ordered.sort_by(|a, b| a.event.id.cmp(&b.event.id));
    for record in ordered {
        if record.event.provider == "claude" {
            if let Some(message) = &record.message {
                match by_message.get_mut(message) {
                    Some(old) if old.sidechain || record.sidechain => {
                        if preferred(&record, old) {
                            *old = record;
                        }
                    }
                    Some(_) => result.push(record),
                    None => {
                        by_message.insert(message.clone(), record);
                    }
                }
                continue;
            }
        }
        result.push(record);
    }
    result.extend(by_message.into_values());
    result.sort_by(|a, b| {
        a.event
            .timestamp
            .cmp(&b.event.timestamp)
            .then(a.event.id.cmp(&b.event.id))
    });
    result
}
// Borrow the parsed cache; only the winning records become priced events.
pub(super) fn dedup_refs<'a>(records: impl IntoIterator<Item = &'a Record>) -> Vec<&'a Record> {
    let mut by_id: HashMap<(&str, &str), &Record> = HashMap::new();
    for record in records {
        let key = (record.event.provider.as_str(), record.event.id.as_str());
        match by_id.get_mut(&key) {
            Some(old) if preferred(&record, old) => *old = record,
            None => {
                by_id.insert(key, record);
            }
            _ => {}
        }
    }
    let mut by_message: HashMap<&str, &Record> = HashMap::new();
    let mut result = Vec::new();
    let mut ordered = by_id.into_values().collect::<Vec<_>>();
    ordered.sort_by(|a, b| a.event.id.cmp(&b.event.id));
    for record in ordered {
        if record.event.provider == "claude" {
            if let Some(message) = &record.message {
                match by_message.get_mut(message.as_str()) {
                    Some(old) if old.sidechain || record.sidechain => {
                        if preferred(&record, old) {
                            *old = record;
                        }
                    }
                    Some(_) => result.push(record),
                    None => {
                        by_message.insert(message.as_str(), record);
                    }
                }
                continue;
            }
        }
        result.push(record);
    }
    result.extend(by_message.into_values());
    result.sort_by(|a, b| {
        a.event
            .timestamp
            .cmp(&b.event.timestamp)
            .then(a.event.id.cmp(&b.event.id))
    });
    result
}
pub(super) fn preferred(new: &Record, old: &Record) -> bool {
    let buckets = |record: &Record| {
        record.breakdown.map(|b| {
            (
                b.input,
                b.output,
                b.cache_read,
                b.cache_write_5m,
                b.cache_write_1h,
                b.is_fast,
                b.is_ultrafast,
            )
        })
    };
    (!new.sidechain && old.sidechain)
        || (new.sidechain == old.sidechain && new.event.tokens > old.event.tokens)
        || (new.sidechain == old.sidechain
            && new.event.tokens == old.event.tokens
            && (new.event.cost_usd.is_some() && old.event.cost_usd.is_none()
                || new.event.cost_usd.is_some() == old.event.cost_usd.is_some()
                    && new
                        .breakdown
                        .is_some_and(|tokens| tokens.is_fast || tokens.is_ultrafast)
                    && !old
                        .breakdown
                        .is_some_and(|tokens| tokens.is_fast || tokens.is_ultrafast)))
        // Equivalent copies may differ only in explicit directory metadata. Keep that
        // metadata while leaving token, recorded-cost and pricing-bucket precedence intact.
        || (new.sidechain == old.sidechain
            && new.event.tokens == old.event.tokens
            && new.event.timestamp == old.event.timestamp
            && new.event.model == old.event.model
            && new.event.cost_usd == old.event.cost_usd
            && buckets(new) == buckets(old)
            && new.event.project.is_some() && old.event.project.is_none())
}

// Read one line at a time, retaining at most 4 MiB. Oversized prompt records are skipped.
pub fn read_lines(path: &Path, mut consume: impl FnMut(usize, &str)) -> io::Result<()> {
    let mut reader = BufReader::with_capacity(64 * 1024, File::open(path)?);
    let mut line = Vec::new();
    let mut number = 0;
    let mut oversized = false;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            if !line.is_empty() && !oversized {
                if let Ok(text) = std::str::from_utf8(&line) {
                    consume(number, text);
                }
            }
            break;
        }
        let length = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |index| index + 1);
        let finished = buffer[length - 1] == b'\n';
        if !oversized && line.len() + length <= 4 * 1024 * 1024 {
            line.extend_from_slice(&buffer[..length]);
        } else {
            oversized = true;
            line.clear();
        }
        reader.consume(length);
        if finished {
            if !oversized {
                if let Ok(text) = std::str::from_utf8(&line) {
                    consume(number, text);
                }
            }
            number += 1;
            line.clear();
            oversized = false;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(id: &str, tokens: u64, sidechain: bool) -> Record {
        Record {
            event: UsageEvent {
                id: id.into(),
                provider: "claude".into(),
                timestamp: "2026-09-30T12:00:00Z".into(),
                model: "unpriced-model".into(),
                tokens,
                cost_usd: None,
                unpriced_tokens: 0,
                cost_source: None,
                project: None,
            },
            message: Some("message-a".into()),
            sidechain,
            breakdown: None,
        }
    }
    #[test]
    fn sidechain_replays_do_not_inflate_usage() {
        let result = dedup(vec![
            record("primary", 10, false),
            record("replayed", 100, true),
            record("primary", 20, false),
        ]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event.tokens, 20);
    }
    #[test]
    fn unknown_pricing_preserves_tokens() {
        let result = dedup(vec![record("primary", 10, false)]);
        assert_eq!(result[0].event.tokens, 10);
        assert_eq!(result[0].event.cost_usd, None);
    }
    #[test]
    fn a_fast_duplicate_does_not_discard_an_already_recorded_cost() {
        let mut recorded = record("primary", 100, false);
        recorded.event.cost_usd = Some(1.25);
        let fast = record("primary", 100, false).with_breakdown(TokenBreakdown {
            input: 100,
            is_fast: true,
            ..TokenBreakdown::default()
        });
        let result = dedup(vec![recorded, fast]);
        assert_eq!(result[0].event.cost_usd, Some(1.25));
    }
    #[test]
    fn equivalent_replay_copies_prefer_real_directory_metadata_without_repricing_or_recounting() {
        let unassigned = record("same", 100, false);
        let mut assigned = unassigned.clone();
        assigned.event.project = super::super::Project::from_directory("/synthetic/app");
        for copies in [
            vec![unassigned.clone(), assigned.clone()],
            vec![assigned.clone(), unassigned.clone()],
        ] {
            let result = dedup(copies);
            assert_eq!(result.len(), 1);
            assert_eq!(result[0].event.tokens, 100);
            assert_eq!(result[0].event.project, assigned.event.project);
        }
        let mut recorded = unassigned;
        recorded.event.cost_usd = Some(0.5);
        let result = dedup(vec![recorded, assigned]);
        assert_eq!(result[0].event.cost_usd, Some(0.5));
        // A weaker replay cannot override the authoritative recorded-cost source.
        assert!(result[0].event.project.is_none());
    }
}
