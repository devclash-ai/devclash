//! Borrow a live Claude Desktop access token without refreshing or changing its login.
mod cache;
#[cfg(any(target_os = "windows", test))]
mod crypto;
#[cfg(any(target_os = "macos", test))]
mod mac_crypto;
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
use zeroize::Zeroizing;

const LIMIT: usize = 1024 * 1024;
const UNREADABLE: &str =
    "Claude Desktop login could not be read. Open Claude Desktop, then Refresh.";

#[derive(Debug)]
enum CookieError {
    Unavailable,
    Invalid(String),
}
fn cookie_error(error: rusqlite::Error) -> CookieError {
    if matches!(&error, rusqlite::Error::SqliteFailure(failure, _) if matches!(failure.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked | rusqlite::ErrorCode::CannotOpen))
    {
        CookieError::Unavailable
    } else {
        CookieError::Invalid(UNREADABLE.into())
    }
}

fn read_json(path: &Path) -> Result<Value, String> {
    let mut bytes = Zeroizing::new(Vec::new());
    std::fs::File::open(path)
        .map_err(|_| UNREADABLE)?
        .take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| UNREADABLE)?;
    if bytes.len() > LIMIT {
        return Err(UNREADABLE.into());
    }
    serde_json::from_slice(&bytes).map_err(|_| UNREADABLE.into())
}

type Decrypt = fn(&[u8], &[u8]) -> Result<Vec<u8>, String>;
fn decode_cache(
    config: &Value,
    field: &str,
    key: &[u8],
    decrypt: Decrypt,
) -> Result<Option<Value>, String> {
    let Some(value) = config.get(field).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let encoded = value
        .as_str()
        .filter(|text| text.len() <= LIMIT * 4 / 3 + 4)
        .ok_or(UNREADABLE)?;
    let cipher = STANDARD.decode(encoded).map_err(|_| UNREADABLE)?;
    let clear = Zeroizing::new(decrypt(&cipher, key)?);
    serde_json::from_slice(&clear)
        .map(Some)
        .map_err(|_| UNREADABLE.into())
}

// Only look in Claude's native or MSIX-redirected app data, never browser profiles.
#[cfg(any(target_os = "windows", test))]
fn roots(roaming: &Path, local: &Path) -> Vec<PathBuf> {
    let mut roots = vec![roaming.join("Claude")];
    if let Ok(entries) = std::fs::read_dir(local.join("Packages")) {
        let mut packages = entries
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("Claude_") && name.len() < 128)
            })
            .map(|entry| entry.path())
            .take(32)
            .collect::<Vec<_>>();
        packages.sort();
        roots.extend(
            packages
                .into_iter()
                .map(|path| path.join("LocalCache/Roaming/Claude")),
        );
    }
    roots
}

fn cookie_path(root: &Path) -> Option<PathBuf> {
    [root.join("Network/Cookies"), root.join("Cookies")]
        .into_iter()
        .find(|path| path.is_file())
}
fn has_active_cookie(root: &Path) -> Result<bool, CookieError> {
    let Some(path) = cookie_path(root) else {
        return Ok(false);
    };
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(cookie_error)?;
    db.busy_timeout(Duration::from_millis(300))
        .map_err(cookie_error)?;
    db.query_row("SELECT 1 FROM cookies WHERE name='lastActiveOrg' AND host_key IN ('claude.ai','.claude.ai') LIMIT 1", [], |_| Ok(true))
        .optional().map(|value| value.unwrap_or(false)).map_err(cookie_error)
}
fn active_org(root: &Path, key: &[u8], decrypt: Decrypt) -> Result<Option<String>, CookieError> {
    let path = cookie_path(root);
    let Some(path) = path else {
        return Ok(None);
    };
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(cookie_error)?;
    db.busy_timeout(Duration::from_millis(300))
        .map_err(cookie_error)?;
    let row = db.query_row(
        "SELECT host_key, substr(value,1,129), substr(encrypted_value,1,1025), length(value), length(encrypted_value) FROM cookies WHERE name='lastActiveOrg' AND host_key IN ('claude.ai','.claude.ai') ORDER BY last_update_utc DESC LIMIT 1",
        [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Vec<u8>>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?))
    ).optional().map_err(cookie_error)?;
    let Some((host, value, encrypted, value_length, encrypted_length)) = row else {
        return Ok(None);
    };
    if value_length > 128 || encrypted_length > 1024 {
        return Err(CookieError::Invalid(UNREADABLE.into()));
    }
    if !value.is_empty() {
        return cache::normalized_uuid(&value)
            .map(Some)
            .ok_or_else(|| CookieError::Invalid(UNREADABLE.into()));
    }
    let clear = Zeroizing::new(decrypt(&encrypted, key).map_err(CookieError::Invalid)?);
    // Chromium cookies bind decrypted values to their host starting with schema version 24.
    let version: Option<String> = db
        .query_row(
            "SELECT substr(value,1,20) FROM meta WHERE key='version'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(cookie_error)?;
    let version = version
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| CookieError::Invalid(UNREADABLE.into()))?;
    let value = if version >= 24 {
        let digest = Sha256::digest(host.as_bytes());
        if !clear.starts_with(&digest) {
            return Err(CookieError::Invalid(UNREADABLE.into()));
        }
        &clear[32..]
    } else {
        clear.as_slice()
    };
    let value = std::str::from_utf8(value).map_err(|_| CookieError::Invalid(UNREADABLE.into()))?;
    cache::normalized_uuid(value)
        .map(Some)
        .ok_or_else(|| CookieError::Invalid(UNREADABLE.into()))
}

fn resolve_organization(
    cookie: Result<Option<String>, CookieError>,
    v2: Option<&Value>,
    v1: Option<&Value>,
    account: Option<&str>,
) -> Result<String, String> {
    match cookie {
        Ok(Some(organization)) => Ok(organization),
        Err(CookieError::Invalid(error)) => Err(error),
        Ok(None) => Err("No active Claude Desktop subscription login was found. Sign in to Claude Desktop, then Refresh.".into()),
        Err(CookieError::Unavailable) => {
            eprintln!("Claude Desktop organization cookie is unavailable; checking unique account organization");
            cache::single_organization(v2, v1, account)
        }
    }
}

#[cfg(target_os = "windows")]
pub(super) fn load() -> Result<Option<super::auth::Credentials>, String> {
    let Some(home) = dirs::home_dir() else {
        return Ok(None);
    };
    let roaming = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData/Roaming"));
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData/Local"));
    // A recent logout must not revive an older installation's cached account.
    let Some(root) = roots(&roaming, &local)
        .into_iter()
        .filter(|root| root.join("config.json").is_file())
        .max_by_key(|root| {
            std::fs::metadata(root.join("config.json"))
                .and_then(|metadata| metadata.modified())
                .ok()
        })
    else {
        return Ok(None);
    };
    let Some(config) = signed_in_config(&root)? else {
        return Ok(None);
    };
    let state = read_json(&root.join("Local State"))?;
    let encoded = state["os_crypt"]["encrypted_key"].as_str().filter(|text| text.len() <= 16 * 1024)
        .ok_or("Claude Desktop uses an unsupported login format. Sign in with Claude Code to show quota.")?;
    let protected = STANDARD.decode(encoded).map_err(|_| UNREADABLE)?;
    let key = Zeroizing::new(crypto::unprotect_key(&protected)?);
    load_with_key(&root, config, &key, crypto::decrypt)
}

fn signed_in_config(root: &Path) -> Result<Option<Value>, String> {
    if !root.join("config.json").is_file() {
        return Ok(None);
    }
    let config = read_json(&root.join("config.json"))?;
    if !["oauth:tokenCacheV2", "oauth:tokenCache"]
        .iter()
        .any(|field| config.get(field).is_some_and(|value| !value.is_null()))
    {
        return Ok(None);
    }
    // Desktop retains cache/account metadata after logout. A readable empty cookie store is final.
    match has_active_cookie(root) {
        Ok(false) => return Ok(None),
        Err(CookieError::Invalid(error)) => return Err(error),
        Ok(true) | Err(CookieError::Unavailable) => {}
    }
    Ok(Some(config))
}

#[cfg(target_os = "macos")]
pub(super) fn load() -> Result<Option<super::auth::Credentials>, String> {
    let Some(home) = dirs::home_dir() else {
        return Ok(None);
    };
    let root = home.join("Library/Application Support/Claude");
    let Some(config) = signed_in_config(&root)? else {
        return Ok(None);
    };
    let password = super::mac_keychain::password("Claude Safe Storage", Some("Claude Key"))?
        .ok_or("Claude Desktop encryption key was not found. Open Claude Desktop, then Refresh.")?;
    let key = mac_crypto::derive_key(&password)?;
    load_with_key(&root, config, key.as_slice(), mac_crypto::decrypt)
}

#[cfg(target_os = "macos")]
pub(super) fn authorize() -> Result<(), String> {
    let home = dirs::home_dir().ok_or("Your home folder could not be found.")?;
    if signed_in_config(&home.join("Library/Application Support/Claude"))?.is_none() {
        return Err("Sign in to Claude Desktop before authorizing its quota access.".into());
    }
    super::mac_keychain::read("Claude Safe Storage", Some("Claude Key"), true)?
        .ok_or("Claude Desktop encryption key was not found. Open Claude Desktop first.")?;
    Ok(())
}

fn load_with_key(
    root: &Path,
    config: Value,
    key: &[u8],
    decrypt: Decrypt,
) -> Result<Option<super::auth::Credentials>, String> {
    let v2 = decode_cache(&config, "oauth:tokenCacheV2", key, decrypt)?;
    let v1 = decode_cache(&config, "oauth:tokenCache", key, decrypt)?;
    let account = config["lastKnownAccountUuid"].as_str();
    let organization = resolve_organization(
        active_org(root, key, decrypt),
        v2.as_ref(),
        v1.as_ref(),
        account,
    )?;
    let token = cache::select_scoped(
        v2.as_ref(),
        v1.as_ref(),
        account,
        &organization,
        chrono::Utc::now().timestamp_millis() as f64,
    )?;
    let latest = read_json(&root.join("config.json"))?;
    if latest.get("lastKnownAccountUuid") != config.get("lastKnownAccountUuid")
        || latest.get("oauth:tokenCacheV2") != config.get("oauth:tokenCacheV2")
        || latest.get("oauth:tokenCache") != config.get("oauth:tokenCache")
    {
        return Err("Claude Desktop login changed during Refresh. Try Refresh again.".into());
    }
    match active_org(root, key, decrypt) {
        Ok(Some(current)) if current == organization => {}
        Err(CookieError::Unavailable) => {}
        _ => {
            return Err(
                "Claude Desktop account or organization changed during Refresh. Try Refresh again."
                    .into(),
            )
        }
    }
    Ok(Some(super::auth::Credentials {
        token,
        account: None,
    }))
}

#[cfg(test)]
mod mac_tests;
#[cfg(test)]
mod tests;
