//! Read the same local login stores as OpenUsage. Never print or modify provider credentials.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
pub struct Credentials {
    pub token: String,
    pub account: Option<String>,
}
pub fn jwt_claims(token: &str) -> Option<Value> {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(token.split('.').nth(1)?).ok()?).ok()
}
pub fn cursor_account(credentials: &Credentials) -> Result<String, String> {
    let subject = jwt_claims(&credentials.token)
        .and_then(|claims| claims["sub"].as_str().map(str::to_owned))
        .ok_or("Cursor login is missing its account identifier. Sign in to Cursor again.")?;
    // Match OpenUsage's dashboard session construction without persisting the cookie.
    let user = subject.split('|').nth(1).unwrap_or(&subject);
    if user.is_empty()
        || user.len() > 256
        || user
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'-'))
    {
        return Err(
            "Cursor login has an invalid account identifier. Sign in to Cursor again.".into(),
        );
    }
    Ok(user.into())
}
pub fn cursor_cookie(credentials: &Credentials) -> Result<reqwest::header::HeaderValue, String> {
    let user = cursor_account(credentials)?;
    let cookie = format!("WorkosCursorSessionToken={user}%3A%3A{}", credentials.token);
    let mut header = reqwest::header::HeaderValue::from_str(&cookie)
        .map_err(|_| "Cursor credentials are not usable. Sign in to Cursor again.")?;
    header.set_sensitive(true);
    Ok(header)
}
pub(super) fn credentials(token: &str, account: Option<String>) -> Result<Credentials, String> {
    let token = token.trim();
    if token.is_empty() || token.len() > 16 * 1024 {
        return Err("No usable provider login was found.".into());
    }
    if jwt_claims(token)
        .and_then(|claims| claims["exp"].as_i64())
        .is_some_and(|expires| expires <= Utc::now().timestamp())
    {
        return Err(
            "The provider login has expired. Sign in to the provider again, then Refresh.".into(),
        );
    }
    Ok(Credentials {
        token: token.into(),
        account,
    })
}
fn json(path: &Path) -> Option<Value> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= 1024 * 1024)
        .then(|| serde_json::from_slice(&bytes).ok())
        .flatten()
}
fn claude(value: &Value) -> Result<Credentials, String> {
    let oauth = &value["claudeAiOauth"];
    if oauth["expiresAt"]
        .as_i64()
        .is_some_and(|expires| expires <= Utc::now().timestamp_millis())
    {
        return Err("Claude login has expired. Run claude login, then Refresh.".into());
    }
    if oauth["scopes"].as_array().is_some_and(|scopes| {
        !scopes.is_empty() && !scopes.iter().any(|scope| scope == "user:profile")
    }) {
        return Err(
            "Claude login has no quota permission. Run claude login to use a subscription login."
                .into(),
        );
    }
    credentials(oauth["accessToken"].as_str().unwrap_or(""), None)
}
fn codex(value: &Value) -> Result<Credentials, String> {
    let tokens = &value["tokens"];
    if tokens["access_token"]
        .as_str()
        .is_none_or(|token| token.trim().is_empty())
        && value["OPENAI_API_KEY"].as_str().is_some()
    {
        return Err("Codex API-key login has no subscription quota. Sign in to Codex with ChatGPT to show quota.".into());
    }
    credentials(
        tokens["access_token"].as_str().unwrap_or(""),
        tokens["account_id"].as_str().map(str::to_owned),
    )
}
#[cfg(target_os = "macos")]
fn keychain(service: &str) -> Result<Option<Value>, String> {
    super::mac_keychain::password(service, None)?
        .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| "The provider's Keychain login has an unsupported format. Sign in to the provider again.".into()))
        .transpose()
}
#[cfg(not(target_os = "macos"))]
fn keychain(_service: &str) -> Result<Option<Value>, String> {
    Ok(None)
}
fn codex_keyring(directory: &Path) -> Result<Option<Value>, String> {
    let canonical = directory
        .canonicalize()
        .unwrap_or_else(|_| directory.into());
    let key = format!(
        "cli|{:x}",
        Sha256::digest(canonical.to_string_lossy().as_bytes())
    );
    let account = &key[..20];
    #[cfg(target_os = "macos")]
    {
        return super::mac_keychain::password("Codex Auth", Some(account))?
            .map(|bytes| {
                serde_json::from_slice(&bytes).map_err(|_| {
                    "Codex Keychain login has an unsupported format. Sign in to Codex again.".into()
                })
            })
            .transpose();
    }
    #[cfg(not(target_os = "macos"))]
    Ok(keyring::Entry::new("Codex Auth", account)
        .ok()
        .and_then(|entry| entry.get_password().ok())
        .and_then(|text| serde_json::from_str(&text).ok()))
}

pub(super) fn claude_service(directory: &Path) -> String {
    let native = dirs::home_dir().map(|home| home.join(".claude"));
    if native.as_deref() == Some(directory) {
        "Claude Code-credentials".into()
    } else {
        format!(
            "Claude Code-credentials-{}",
            &format!(
                "{:x}",
                Sha256::digest(directory.to_string_lossy().as_bytes())
            )[..8]
        )
    }
}
#[cfg(target_os = "macos")]
pub(super) fn authorize_claude_code() -> Result<(), String> {
    for directory in crate::paths::configured("claude") {
        if super::mac_keychain::read(&claude_service(&directory), None, true)?.is_some() {
            return Ok(());
        }
    }
    Err("No Claude Code Keychain login was found. Sign in with Claude Code first.".into())
}

#[cfg(target_os = "macos")]
pub(super) fn authorize_codex() -> Result<(), String> {
    for directory in crate::paths::configured("codex") {
        let canonical = directory.canonicalize().unwrap_or(directory);
        let key = format!(
            "cli|{:x}",
            Sha256::digest(canonical.to_string_lossy().as_bytes())
        );
        if super::mac_keychain::read("Codex Auth", Some(&key[..20]), true)?.is_some() {
            return Ok(());
        }
    }
    Err("No Codex Keychain login was found. Sign in to Codex with ChatGPT first.".into())
}
pub fn cursor_login(token: &str) -> Result<Credentials, String> {
    credentials(token, None)
}
pub fn load(provider: &str) -> Result<Credentials, String> {
    if provider == "cursor" {
        let directories = crate::paths::configured(provider);
        let native_defaults = directories
            == crate::paths::defaults(
                provider,
                &dirs::home_dir().unwrap_or_default(),
                std::env::consts::OS,
            );
        return super::cursor_auth::load(&directories, native_defaults);
    }
    let primary = load_local(provider);
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    if provider == "claude"
        && (primary.is_ok()
            || crate::paths::configured(provider)
                == crate::paths::defaults(
                    provider,
                    &dirs::home_dir().unwrap_or_default(),
                    std::env::consts::OS,
                ))
    {
        return with_claude_desktop(primary, super::claude_desktop::load);
    }
    primary
}
#[cfg(any(target_os = "windows", target_os = "macos", test))]
fn with_claude_desktop(
    primary: Result<Credentials, String>,
    desktop: impl FnOnce() -> Result<Option<Credentials>, String>,
) -> Result<Credentials, String> {
    match primary {
        Ok(credentials) => Ok(credentials),
        Err(reason) => desktop()?.ok_or(reason),
    }
}
fn load_local(provider: &str) -> Result<Credentials, String> {
    let mut reason=match provider {
        "claude" if std::env::var_os("CLAUDE_CODE_OAUTH_TOKEN").is_some()=>"Claude setup-token supports inference only. Run claude login to show subscription quota.",
        "claude"=>claude_missing_login(),
        "codex"=>"No Codex subscription login was found. Sign in to Codex with ChatGPT, then Refresh.",
        "cursor"=>"No Cursor login was found. Sign in to Cursor, then Refresh.",
        _=>"OpenCode has no global quota. Sign in to OpenCode Go to show its plan quota.",
    }.to_owned();
    for directory in crate::paths::configured(provider) {
        let result = match provider {
            "claude" => {
                let service = claude_service(&directory);
                match keychain(&service) {
                    Ok(stored) => stored
                        .or_else(|| json(&directory.join(".credentials.json")))
                        .map(|value| claude(&value)),
                    Err(error) => Some(
                        json(&directory.join(".credentials.json"))
                            .map_or(Err(error), |value| claude(&value)),
                    ),
                }
            }
            "codex" => match json(&directory.join("auth.json")) {
                Some(value) => Some(codex(&value)),
                None => match codex_keyring(&directory) {
                    Ok(value) => value.map(|value| codex(&value)),
                    Err(error) => Some(Err(error)),
                },
            },
            "opencode" => match super::opencode_auth::load(&directory) {
                Ok(Some(auth)) => Some(Ok(auth)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            },
            _ => None,
        };
        match result {
            Some(Ok(credentials)) => return Ok(credentials),
            Some(Err(error)) => reason = error,
            None => {}
        }
    }
    Err(reason)
}
fn claude_missing_login() -> &'static str {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        "No usable Claude Code or Claude Desktop login was found. Open Claude Desktop or sign in with Claude Code, then Refresh."
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        "No Claude Code subscription login was found. Run claude login, then Refresh."
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Opt-in read-only real-account check; use DEVCLASH_MAC_LIVE_QA=1"]
    fn actual_mac_app_and_cli_login_sources() {
        assert_eq!(std::env::var("DEVCLASH_MAC_LIVE_QA").as_deref(), Ok("1"));
        let home = dirs::home_dir().unwrap();
        let status = |auth: Result<Credentials, String>| match auth {
            Ok(_) => json!({"usable":true}),
            Err(error) => {
                json!({"usable":false,"permission_required":error.contains("Keychain access is required")})
            }
        };
        let desktop = super::super::claude_desktop::load();
        let desktop_status = match desktop {
            Ok(auth) => json!({"usable":auth.is_some()}),
            Err(error) => {
                json!({"usable":false,"permission_required":error.contains("Keychain access is required")})
            }
        };
        let result = json!({
            "claude":{"app_only":desktop_status,"cli_only":status(load_local("claude")),"both":status(load("claude"))},
            "cursor":{"app_only":status(super::super::cursor_auth::load(&[home.join("Library/Application Support/Cursor")],false)),
                "cli_only":status(super::super::cursor_auth::load(&[home.join(".cursor")],true)),"both":status(load("cursor"))},
            "codex":{"shared_app_cli_store":status(load("codex"))},
            "opencode":{"shared_app_cli_store":status(load("opencode"))}
        });
        println!("DEVCLASH_MAC_LOGIN_RESULT={result}");
    }
    #[test]
    fn usable_claude_code_login_never_probes_desktop_and_missing_fallback_keeps_reason() {
        let auth = with_claude_desktop(
            Ok(Credentials {
                token: "synthetic-cli".into(),
                account: None,
            }),
            || panic!("Desktop must not be probed for a usable CLI login"),
        )
        .unwrap();
        assert_eq!(auth.token, "synthetic-cli");
        assert_eq!(
            with_claude_desktop(Err("CLI expired".into()), || Ok(None))
                .err()
                .unwrap(),
            "CLI expired"
        );
        let auth = with_claude_desktop(Err("CLI missing".into()), || {
            Ok(Some(Credentials {
                token: "synthetic-desktop".into(),
                account: None,
            }))
        })
        .unwrap();
        assert_eq!(auth.token, "synthetic-desktop");
    }
    #[test]
    fn rejects_expired_and_inference_only_credentials() {
        assert!(
            claude(&json!({"claudeAiOauth":{"accessToken":"test","expiresAt":1}}))
                .err()
                .unwrap()
                .contains("expired")
        );
        assert!(claude(
            &json!({"claudeAiOauth":{"accessToken":"test","scopes":["user:inference"]}})
        )
        .err()
        .unwrap()
        .contains("permission"));
        assert!(codex(&json!({"OPENAI_API_KEY":"test"}))
            .err()
            .unwrap()
            .contains("API-key"));
    }
    #[test]
    fn valid_oauth_account_stays_private() {
        let auth =
            codex(&json!({"tokens":{"access_token":"test","account_id":"private-id"}})).unwrap();
        assert_eq!(auth.account.as_deref(), Some("private-id"));
    }
    #[test]
    fn cursor_dashboard_cookie_matches_upstream_and_marks_credentials_sensitive() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"sub":"auth|account-a|unused"}"#);
        let auth = Credentials {
            token: format!("header.{payload}.signature"),
            account: None,
        };
        assert_eq!(cursor_account(&auth).unwrap(), "account-a");
        let header = cursor_cookie(&auth).unwrap();
        assert!(header.is_sensitive());
        assert_eq!(
            header.to_str().unwrap(),
            format!("WorkosCursorSessionToken=account-a%3A%3A{}", auth.token)
        );
        let empty = URL_SAFE_NO_PAD.encode(br#"{"sub":"auth|"}"#);
        assert!(cursor_cookie(&Credentials {
            token: format!("header.{empty}.signature"),
            account: None
        })
        .is_err());
    }
}
