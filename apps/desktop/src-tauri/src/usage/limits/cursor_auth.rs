//! Independent Cursor Desktop and CLI logins; credential values stay in memory.
use super::auth::{cursor_account, cursor_login, Credentials};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::{io::Read, path::PathBuf};

const MAX_CREDENTIAL_BYTES: usize = 1024 * 1024;
const MISSING: &str = "No usable Cursor Desktop or CLI login was found. Sign in to Cursor or run agent login, then Refresh.";

#[derive(Clone, Copy)]
enum CliStore {
    Keychain,
    File,
    Memory,
}

pub fn load(directories: &[PathBuf], native_defaults: bool) -> Result<Credentials, String> {
    // The current CLI does not restore a file when its default Mac Keychain login
    // is absent. File storage is an explicit mode; honoring this prevents logout
    // from reviving an old auth.json left by a different credential store.
    let mode = if !native_defaults || !cfg!(target_os = "macos") {
        CliStore::File
    } else {
        match std::env::var("AGENT_CLI_CREDENTIAL_STORE").as_deref() {
            Ok("file") => CliStore::File,
            Ok("memory") => CliStore::Memory,
            _ => CliStore::Keychain,
        }
    };
    load_with(directories, mode, keychain)
}

fn usable(token: &str) -> Result<Credentials, String> {
    let credentials = cursor_login(token)?;
    cursor_account(&credentials)?;
    Ok(credentials)
}

fn desktop(directory: &std::path::Path) -> Result<Option<String>, String> {
    let path = directory.join("User/globalStorage/state.vscdb");
    if !path.exists() {
        return Ok(None);
    }
    let database = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| "Cursor Desktop login database could not be read.".to_owned())?;
    database
        .query_row(
            "SELECT value FROM ItemTable WHERE key='cursorAuth/accessToken' LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| "Cursor Desktop login database could not be inspected.".to_owned())
}

fn file(directory: &std::path::Path) -> Result<Option<String>, String> {
    let path = directory.join("auth.json");
    let mut bytes = Vec::new();
    let source = match std::fs::File::open(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Cursor CLI login file could not be read.".into()),
    };
    source
        .take(MAX_CREDENTIAL_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cursor CLI login file could not be read.".to_owned())?;
    if bytes.len() > MAX_CREDENTIAL_BYTES {
        return Err("Cursor CLI login file exceeded its size limit.".into());
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Cursor CLI login file has an unsupported format.".to_owned())?;
    if let Some(token) = value["accessToken"].as_str() {
        return Ok(Some(token.to_owned()));
    }
    if value["apiKey"].as_str().is_some() {
        return Err("Cursor API-key login has no stored dashboard session. Run agent login to show account usage.".into());
    }
    Ok(None)
}

fn load_with(
    directories: &[PathBuf],
    cli_store: CliStore,
    keychain: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<Credentials, String> {
    let mut reason = MISSING.to_owned();
    for directory in directories {
        match desktop(directory).and_then(|token| token.map(|token| usable(&token)).transpose()) {
            Ok(Some(credentials)) => return Ok(credentials),
            Ok(None) => {}
            Err(error) => reason = error,
        }
    }
    match cli_store {
        CliStore::Keychain => match keychain()? {
            Some(token) => usable(&token),
            None => Err(reason),
        },
        CliStore::File => {
            for directory in directories {
                match file(directory)
                    .and_then(|token| token.map(|token| usable(&token)).transpose())
                {
                    Ok(Some(credentials)) => return Ok(credentials),
                    Ok(None) => {}
                    Err(error) => reason = error,
                }
            }
            Err(reason)
        }
        CliStore::Memory => Err(reason),
    }
}

#[cfg(target_os = "macos")]
fn keychain() -> Result<Option<String>, String> {
    super::mac_keychain::password("cursor-access-token", Some("cursor-user"))?
        .map(|bytes| {
            if bytes.len() > MAX_CREDENTIAL_BYTES {
                return Err("Cursor CLI Keychain login exceeded its size limit.".into());
            }
            std::str::from_utf8(&bytes)
                .map(str::to_owned)
                .map_err(|_| "Cursor CLI Keychain login has an unsupported format.".into())
        })
        .transpose()
}

#[cfg(not(target_os = "macos"))]
fn keychain() -> Result<Option<String>, String> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "devclash-cursor-auth-{}-{}-{}",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&directory).unwrap();
            Self(directory)
        }
        fn desktop(&self, token: Option<&str>) {
            let directory = self.0.join("User/globalStorage");
            std::fs::create_dir_all(&directory).unwrap();
            let database = Connection::open(directory.join("state.vscdb")).unwrap();
            database
                .execute_batch("CREATE TABLE ItemTable(key TEXT PRIMARY KEY,value TEXT)")
                .unwrap();
            if let Some(token) = token {
                database
                    .execute(
                        "INSERT INTO ItemTable VALUES('cursorAuth/accessToken',?)",
                        [token],
                    )
                    .unwrap();
            }
        }
        fn file(&self, token: &str) {
            std::fs::write(
                self.0.join("auth.json"),
                serde_json::json!({"accessToken":token,"refreshToken":"synthetic-refresh"})
                    .to_string(),
            )
            .unwrap();
        }
        fn roots(&self) -> Vec<PathBuf> {
            vec![self.0.clone()]
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn token(account: &str, expired: bool) -> String {
        let expiration = if expired { 1 } else { 4_102_444_800_i64 };
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::json!({"sub":format!("auth|{account}"),"exp":expiration}).to_string(),
        );
        format!("header.{payload}.signature")
    }

    #[test]
    fn desktop_only_and_both_keep_the_usable_desktop_login_without_probing_cli() {
        for mode in [CliStore::Keychain, CliStore::File] {
            let fixture = Fixture::new();
            let desktop = token("desktop-account", false);
            fixture.desktop(Some(&desktop));
            fixture.file(&token("different-cli-account", false));
            let credentials =
                load_with(&fixture.roots(), mode, || panic!("Desktop usable")).unwrap();
            assert_eq!(credentials.token, desktop);
        }
    }
    #[test]
    fn cli_only_and_expired_desktop_use_independent_keychain_login() {
        for desktop in [
            None,
            Some(token("old-desktop-account", true)),
            Some(String::new()),
        ] {
            let fixture = Fixture::new();
            if let Some(desktop) = &desktop {
                fixture.desktop(Some(desktop));
            }
            let cli = token("cli-account", false);
            let credentials = load_with(&fixture.roots(), CliStore::Keychain, || {
                Ok(Some(cli.clone()))
            })
            .unwrap();
            assert_eq!(credentials.token, cli);
        }
    }
    #[test]
    fn keychain_logout_denial_expiry_and_empty_value_never_restore_stale_file() {
        let fixture = Fixture::new();
        fixture.desktop(None);
        fixture.file(&token("stale-file-account", false));
        for result in [
            Ok(None),
            Err("Keychain denied".into()),
            Ok(Some(String::new())),
            Ok(Some(token("expired-cli", true))),
        ] {
            assert!(load_with(&fixture.roots(), CliStore::Keychain, || result).is_err());
        }
    }
    #[test]
    fn explicit_file_store_works_without_keychain_and_logout_stays_logged_out() {
        let fixture = Fixture::new();
        let cli = token("file-cli-account", false);
        fixture.file(&cli);
        assert_eq!(
            load_with(&fixture.roots(), CliStore::File, || panic!("File mode"))
                .unwrap()
                .token,
            cli
        );
        assert_eq!(load(&fixture.roots(), false).unwrap().token, cli);
        std::fs::write(fixture.0.join("auth.json"), r#"{"bedrockCredentials":{}}"#).unwrap();
        assert!(load_with(&fixture.roots(), CliStore::File, || panic!("File mode")).is_err());
        assert!(load(&fixture.roots(), false).is_err());
        std::fs::remove_file(fixture.0.join("auth.json")).unwrap();
        assert!(load_with(&fixture.roots(), CliStore::File, || panic!("File mode")).is_err());
    }
    #[test]
    fn unreadable_oversized_api_key_and_missing_subject_are_friendly_errors() {
        let fixture = Fixture::new();
        for value in [
            "{".to_owned(),
            "x".repeat(MAX_CREDENTIAL_BYTES + 1),
            r#"{"apiKey":"synthetic-api-key"}"#.to_owned(),
            r#"{"accessToken":"opaque-token"}"#.to_owned(),
        ] {
            std::fs::write(fixture.0.join("auth.json"), value).unwrap();
            assert!(load_with(&fixture.roots(), CliStore::File, || panic!("File mode")).is_err());
        }
    }
}
