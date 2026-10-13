//! OpenUsage's OpenCode database approach, extended to include every OpenCode provider.
//! Only usage and selected directory columns leave SQLite; message content and credentials do not.
use super::{scan::Record, Project, UsageEvent};
use crate::pricing::TokenBreakdown;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(test)]
mod tests;

#[derive(Default)]
pub struct Scan {
    pub records: Vec<Record>,
    pub available: bool,
    pub errors: Vec<String>,
}

#[cfg(test)]
fn expand(value: &str, home: &Path) -> PathBuf {
    if value == "~" {
        home.into()
    } else if let Some(suffix) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        home.join(suffix)
    } else {
        value.into()
    }
}

#[cfg(test)]
fn data_directory(home: &Path, override_dir: Option<&str>, xdg: Option<&str>) -> PathBuf {
    fn nonempty(value: Option<&str>) -> Option<&str> {
        value.map(str::trim).filter(|value| !value.is_empty())
    }
    if let Some(value) = nonempty(override_dir) {
        expand(value, home)
    } else if let Some(value) = nonempty(xdg) {
        expand(value, home).join("opencode")
    } else {
        home.join(".local").join("share").join("opencode")
    }
}

pub fn scan(since: DateTime<Utc>) -> Scan {
    let mut combined = Scan::default();
    for directory in crate::paths::configured("opencode") {
        let result = scan_directory(&directory, since);
        combined.available |= result.available;
        combined.records.extend(result.records);
        combined.errors.extend(result.errors);
    }
    combined
}

fn database_paths(directory: &Path) -> io::Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        result => result?,
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("opencode") && name.ends_with(".db") && entry.path().is_file() {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

fn scan_directory(directory: &Path, since: DateTime<Utc>) -> Scan {
    let mut result = Scan::default();
    let paths = match database_paths(directory) {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("OpenCode database discovery failed: {error}");
            result.errors.push(
                "OpenCode usage could not be read. Check access to its data directory.".into(),
            );
            return result;
        }
    };
    let mut failed = 0;
    for path in paths {
        match read_database(&path, since) {
            Ok(Some(records)) => {
                result.available = true;
                result.records.extend(records);
            }
            Ok(None) => {} // A channel without either message table contains no usage yet.
            Err(error) => {
                failed += 1;
                eprintln!(
                    "OpenCode usage query failed for {}: {error}",
                    path.display()
                );
            }
        }
    }
    if failed > 0 {
        result.errors.push("Some OpenCode usage databases could not be read. Other available usage is still shown.".into());
    }
    // Migration and channel copies share message IDs. Reuse the collector's reconciliation.
    result.records = super::scan::dedup(result.records);
    result
}

// Known table names only. Optional relations vary across OpenCode database generations.
fn columns(connection: &Connection, table: &str) -> rusqlite::Result<HashSet<String>> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    Ok(names)
}
fn directory_query(connection: &Connection, table: &str) -> rusqlite::Result<(String, String)> {
    let mut joins = String::new();
    let mut session_directories = Vec::new();
    let mut project_directories = Vec::new();
    if columns(connection, table)?.contains("session_id") {
        let candidates = if table == "session_message" {
            &["session", "session_v2"][..]
        } else {
            &["session"][..]
        };
        for (index, session) in candidates.iter().enumerate() {
            let schema = columns(connection, session)?;
            if !schema.contains("id") {
                continue;
            }
            joins.push_str(&format!(
                " LEFT JOIN {session} s{index} ON s{index}.id=m.session_id"
            ));
            if schema.contains("directory") {
                session_directories.push(format!("s{index}.directory"));
            }
            if schema.contains("project_id") {
                let project_schema = columns(connection, "project")?;
                if project_schema.contains("id") && project_schema.contains("worktree") {
                    joins.push_str(&format!(
                        " LEFT JOIN project p{index} ON p{index}.id=s{index}.project_id"
                    ));
                    project_directories.push(format!("p{index}.worktree"));
                }
            }
        }
    }
    let mut selected = vec![
        "json_extract(m.data,'$.path.cwd')".into(),
        "json_extract(m.data,'$.path.root')".into(),
    ];
    selected.extend(session_directories);
    selected.extend(project_directories);
    Ok((format!("json_array({})", selected.join(",")), joins))
}

fn read_database(path: &Path, since: DateTime<Utc>) -> rusqlite::Result<Option<Vec<Record>>> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_millis(500))?;
    let mut tables = connection.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name IN ('message','session_message') ORDER BY name")?;
    let tables = tables
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if tables.is_empty() {
        return Ok(None);
    }
    let mut records = Vec::new();
    for table in tables {
        let (directories, joins) = directory_query(&connection, &table)?;
        let role = if table == "message" {
            "json_extract(m.data,'$.role')='assistant'"
        } else {
            "(m.type='assistant' OR (m.type='compaction' AND json_extract(m.data,'$.status')='completed'))"
        };
        let sql = format!("SELECT CAST(m.id AS TEXT), m.time_created,
            CASE WHEN json_type(m.data,'$.time.completed') IN ('integer','real')
                THEN CAST(json_extract(m.data,'$.time.completed') AS INTEGER) ELSE m.time_created END,
            json_object('model',COALESCE(json_extract(m.data,'$.model.id'),json_extract(m.data,'$.modelID')),
                'cost',json_extract(m.data,'$.cost'), 'total',json_extract(m.data,'$.tokens.total'),
                'input',json_extract(m.data,'$.tokens.input'), 'output',json_extract(m.data,'$.tokens.output'),
                'reasoning',json_extract(m.data,'$.tokens.reasoning'),
                'cacheRead',json_extract(m.data,'$.tokens.cache.read'),
                'cacheWrite',json_extract(m.data,'$.tokens.cache.write')), {directories}
            FROM {table} m {joins} WHERE m.time_created >= ?1
                AND CASE WHEN json_valid(m.data) THEN ({role} AND json_type(m.data,'$.tokens')='object') ELSE 0 END
            ORDER BY m.time_created, m.id");
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map([since.timestamp_millis()], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        for (index, row) in rows.enumerate() {
            let (id, created, completed, usage, directories) = row?;
            let metadata: Value = serde_json::from_str(&usage)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            let id = id
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| {
                    format!(
                        "{}:{table}:{created}:{index}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    )
                });
            let directory_metadata: Value = serde_json::from_str(&directories)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            let project = directory_metadata.as_array().and_then(|directories| {
                directories
                    .iter()
                    .find_map(|directory| directory.as_str().and_then(Project::from_directory))
            });
            if let Some(mut record) = parse_record(id, completed, &metadata).filter(|record| {
                DateTime::parse_from_rfc3339(&record.event.timestamp)
                    .is_ok_and(|date| date >= since)
            }) {
                record.event.project = project;
                records.push(record);
            }
        }
    }
    Ok(Some(records))
}

fn count(value: &Value) -> Option<u64> {
    value
        .as_f64()
        .filter(|number| number.is_finite() && *number >= 0. && number.fract() == 0.)
        .map(|number| number.min(1_000_000_000_000_000.) as u64)
}

fn parse_record(id: String, milliseconds: i64, value: &Value) -> Option<Record> {
    let date = DateTime::from_timestamp_millis(milliseconds)?;
    let breakdown = TokenBreakdown {
        input: count(&value["input"]).unwrap_or(0),
        output: count(&value["output"]).unwrap_or(0) + count(&value["reasoning"]).unwrap_or(0),
        cache_read: count(&value["cacheRead"]).unwrap_or(0),
        cache_write_5m: count(&value["cacheWrite"]).unwrap_or(0),
        ..TokenBreakdown::default()
    };
    let bucket_total = breakdown.total()?;
    let tokens = count(&value["total"]).unwrap_or(bucket_total);
    let cost = value["cost"]
        .as_f64()
        .filter(|cost| cost.is_finite() && *cost >= 0.);
    if tokens == 0 && cost.is_none() {
        return None;
    }
    let model = value["model"]
        .as_str()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or("Unknown Model");
    let mut record = Record::plain(UsageEvent {
        id,
        provider: "opencode".into(),
        timestamp: date.to_rfc3339(),
        model: model.into(),
        tokens,
        cost_usd: cost,
        unpriced_tokens: 0,
        cost_source: None,
        project: None,
    });
    // OpenCode's per-message cost is authoritative, including free models with a zero cost.
    // Missing costs use the shared pricing engine only when billable buckets are available.
    if bucket_total > 0 {
        record.breakdown = Some(breakdown)
    }
    Some(record)
}
