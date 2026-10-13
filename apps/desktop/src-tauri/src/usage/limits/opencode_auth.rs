//! Borrow OpenCode Go logins without modifying either authentication store.
use super::auth::{self, Credentials};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Deserialize;
use std::{fs, io::Read, path::Path, time::Duration};
use zeroize::{Zeroize, Zeroizing};

#[cfg(test)]
mod tests;

const IMPORT_MIGRATION: &str = "20260805200742_import_legacy_credentials";
const MAX_JSON: usize = 1024 * 1024;
const MAX_KEY: usize = 16 * 1024;

fn error(message: &str) -> String {
    // SQLite/JSON errors may contain credential material. Log only the fixed reason.
    eprintln!("{message}");
    message.into()
}

fn validate_key(kind: &str, key: &str, legacy: bool) -> Result<Credentials, String> {
    if kind != if legacy { "api" } else { "key" } {
        return Err(error("OpenCode Go login has an unsupported credential type. Sign in to OpenCode Go again, then Refresh."));
    }
    let token = key.trim();
    if token.is_empty()
        || token.len() > MAX_KEY
        || token
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(error("OpenCode Go login contains an invalid key. Sign in to OpenCode Go again, then Refresh."));
    }
    auth::credentials(token, None)
}

#[derive(Deserialize)]
struct LegacyFile {
    #[serde(rename = "opencode-go")]
    go: Option<LegacyCredential>,
}
#[derive(Deserialize)]
struct LegacyCredential {
    #[serde(rename = "type")]
    kind: String,
    key: String,
}
impl Drop for LegacyCredential {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

fn legacy(directory: &Path) -> Result<Option<Credentials>, String> {
    let file = match fs::File::open(directory.join("auth.json")) {
        Ok(file) => file,
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(error(
                "OpenCode login file could not be read. Check access to its data directory.",
            ))
        }
    };
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_JSON as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            error("OpenCode login file could not be read. Check access to its data directory.")
        })?;
    if bytes.len() > MAX_JSON {
        return Err(error(
            "OpenCode login file is too large. Sign in to OpenCode Go again, then Refresh.",
        ));
    }
    let value: LegacyFile = serde_json::from_slice(&bytes)
        .map_err(|_| error("OpenCode login file has an unsupported format. Sign in to OpenCode Go again, then Refresh."))?;
    value
        .go
        .as_ref()
        .map(|go| validate_key(&go.kind, &go.key, true))
        .transpose()
}

struct DatabaseState {
    authoritative: bool,
    login: Option<Credentials>,
}

fn database(path: &Path) -> Result<DatabaseState, String> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| {
        error("OpenCode login database could not be read. Check access to its data directory.")
    })?;
    db.busy_timeout(Duration::from_millis(300)).map_err(|_| {
        error("OpenCode login database could not be read. Check access to its data directory.")
    })?;
    let table = |name: &str| -> Result<bool, String> {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [name],
            |row| row.get(0),
        )
        .map_err(|_| error("OpenCode login database could not be inspected."))
    };
    // This journal remains after v2 deletes the final credential. Merely creating
    // the credential table does not migrate a legacy installation's JSON login.
    let migrated = if table("migration")? {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM migration WHERE id=?1)",
            [IMPORT_MIGRATION],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|_| error("OpenCode credential migration state could not be inspected."))?
    } else {
        false
    };
    if !table("credential")? {
        if migrated {
            return Err(error("OpenCode migrated login database is missing its credential table. Open OpenCode to repair its login, then Refresh."));
        }
        return Ok(DatabaseState {
            authoritative: false,
            login: None,
        });
    }
    let has_rows = db
        .query_row("SELECT EXISTS(SELECT 1 FROM credential)", [], |row| {
            row.get::<_, bool>(0)
        })
        .map_err(|_| error("OpenCode login database could not be inspected."))?;
    let authoritative = migrated || has_rows;
    if !has_rows {
        return Ok(DatabaseState {
            authoritative,
            login: None,
        });
    }
    let columns = db
        .prepare("PRAGMA table_info(credential)")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| error("OpenCode Go login format is not supported by this build."))?;
    if !["id", "integration_id", "value", "time_created"]
        .iter()
        .all(|name| columns.iter().any(|column| column == name))
    {
        return Err(error(
            "OpenCode Go login format is not supported by this build.",
        ));
    }
    let has_active = columns.iter().any(|column| column == "active");
    let selection = if has_active {
        "active DESC, time_created DESC, id DESC"
    } else {
        "time_created DESC, id DESC"
    };
    // Select only the current Go row, even if invalid. Never revive an older row.
    // Do not load other integrations, refresh tokens, account IDs or entire JSON.
    let sql = format!("SELECT length(CAST(value AS BLOB)),
        CASE WHEN length(CAST(value AS BLOB))<={MAX_JSON} AND json_valid(value) THEN json_extract(value,'$.type') END,
        CASE WHEN length(CAST(value AS BLOB))<={MAX_JSON} AND json_valid(value) AND json_type(value,'$.key')='text'
             AND length(CAST(json_extract(value,'$.key') AS BLOB))<={MAX_KEY} THEN json_extract(value,'$.key') END
        FROM credential WHERE integration_id='opencode-go' ORDER BY {selection} LIMIT 1");
    let selected = db
        .query_row(&sql, [], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?.map(Zeroizing::new),
            ))
        })
        .optional()
        .map_err(|_| error("OpenCode Go login format is not supported by this build."))?;
    let Some((size, kind, key)) = selected else {
        return Ok(DatabaseState {
            authoritative,
            login: None,
        });
    };
    if size < 0 || size as usize > MAX_JSON || kind.is_none() || key.is_none() {
        return Err(error("OpenCode Go login contains invalid or oversized credential data. Sign in to OpenCode Go again, then Refresh."));
    }
    let login = validate_key(kind.as_deref().unwrap(), key.as_deref().unwrap(), false)?;
    Ok(DatabaseState {
        authoritative,
        login: Some(login),
    })
}

pub fn load(directory: &Path) -> Result<Option<Credentials>, String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(error(
                "OpenCode data directory could not be read. Check access to its login files.",
            ))
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| error("OpenCode login database discovery failed."))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("opencode") && name.ends_with(".db") && entry.path().is_file() {
            paths.push(entry.path());
        }
        if paths.len() > 64 {
            return Err(error("OpenCode has too many login databases to inspect. Choose its current data folder in Usage Sources."));
        }
    }
    paths.sort();
    let mut authoritative = false;
    let mut selected = None;
    for path in paths {
        let result = database(&path)?;
        authoritative |= result.authoritative;
        // Channels are independent: logout in one cannot revoke a valid Go login
        // in another. Lexical filename order makes the choice deterministic.
        if selected.is_none() {
            selected = result.login;
        }
    }
    if selected.is_some() || authoritative {
        Ok(selected)
    } else {
        legacy(directory)
    }
}
