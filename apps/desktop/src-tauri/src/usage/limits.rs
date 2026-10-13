//! Direct provider quotas; credentials stay on this device and are never refreshed or rewritten.
pub(super) mod auth;
#[cfg(any(target_os = "windows", target_os = "macos", test))]
mod claude_desktop;
#[cfg(any(target_os = "macos", test))]
mod claude_retry;
mod cursor_auth;
#[cfg(target_os = "macos")]
mod mac_keychain;
#[cfg(target_os = "macos")]
pub(super) fn handle_keychain_helper() -> bool {
    mac_keychain::handle_helper()
}
mod opencode_auth;
#[cfg(test)]
mod tests;
use super::Quota;
use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, COOKIE, USER_AGENT};
use serde_json::Value;
use std::{collections::HashMap, io::Read, time::Duration};

#[derive(Default)]
pub struct Limits {
    pub quotas: HashMap<String, Quota>,
    pub errors: HashMap<String, String>,
}

pub(super) fn authorize_login(source: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let (provider, service, account) = match source {
            "claude-code" | "claude-retry" => ("claude", "Claude Code-credentials", None),
            "claude-desktop" => ("claude", "Claude Safe Storage", Some("Claude Key")),
            "cursor-cli" => ("cursor", "cursor-access-token", Some("cursor-user")),
            "codex" => ("codex", "", None),
            _ => return Err("This provider authorization is not supported.".into()),
        };
        let home = dirs::home_dir().ok_or("Your home folder could not be found.")?;
        require_automatic_discovery(
            &crate::paths::configured(provider),
            &crate::paths::defaults(provider, &home, "macos"),
        )?;
        if source == "claude-retry" {
            return claude_retry::authorize();
        }
        if source == "claude-desktop" {
            return claude_desktop::authorize();
        }
        if source == "claude-code" {
            return auth::authorize_claude_code();
        }
        if source == "codex" {
            return auth::authorize_codex();
        }
        mac_keychain::read(service, account, true)?
            .ok_or("No saved login was found. Sign in to the provider first.")?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = source;
        Err("Keychain authorization is available on macOS only.".into())
    }
}

#[cfg(any(target_os = "macos", test))]
fn require_automatic_discovery(
    configured: &[std::path::PathBuf],
    defaults: &[std::path::PathBuf],
) -> Result<(), String> {
    if configured != defaults {
        return Err(
            "Native Keychain authorization requires Automatic Discovery for this provider.".into(),
        );
    }
    Ok(())
}

pub fn read() -> Limits {
    if std::env::var_os("DEVCLASH_QUOTAS_DISABLED").is_some() {
        return Limits::default();
    }
    let mut result = Limits::default();
    std::thread::scope(|scope| {
        let workers = ["claude", "codex", "cursor", "opencode"]
            .map(|provider| (provider, scope.spawn(move || provider_quota(provider))));
        for (provider, worker) in workers {
            match worker
                .join()
                .unwrap_or_else(|_| Err("Quota collection could not finish. Try Refresh.".into()))
            {
                Ok(quota) => {
                    result.quotas.insert(provider.into(), quota);
                }
                Err(reason) => {
                    eprintln!("{provider} quota unavailable: {reason}");
                    result.errors.insert(provider.into(), reason);
                }
            }
        }
    });
    result
}
fn provider_quota(provider: &str) -> Result<Quota, String> {
    let credentials = auth::load(provider)?;
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(USER_AGENT, HeaderValue::from_static("DevClash"));
    if provider == "cursor" {
        headers.insert(COOKIE, auth::cursor_cookie(&credentials)?);
    } else {
        let mut authorization = HeaderValue::from_str(&format!("Bearer {}", credentials.token))
            .map_err(|_| "Provider credentials are not usable. Sign in to the provider again.")?;
        authorization.set_sensitive(true);
        headers.insert(AUTHORIZATION, authorization);
    }
    let url = match provider {
        "claude" => {
            headers.insert(
                "anthropic-beta",
                HeaderValue::from_static("oauth-2025-04-20"),
            );
            headers.insert(
                USER_AGENT,
                HeaderValue::from_static("claude-cli/2.1.280 (external, cli)"),
            );
            "https://api.anthropic.com/api/oauth/usage"
        }
        "codex" => {
            if let Some(account) = credentials.account {
                headers.insert(
                    "ChatGPT-Account-Id",
                    HeaderValue::from_str(&account).map_err(|_| {
                        "Codex account identifier is not usable. Sign in to Codex again."
                    })?,
                );
            }
            "https://chatgpt.com/backend-api/wham/usage"
        }
        "cursor" => "https://cursor.com/api/usage-summary",
        "opencode" => "https://opencode.ai/zen/go/v1/usage",
        _ => return Err("This provider does not expose a supported quota API.".into()),
    };
    parse(provider,&fetch(url,headers)?,Utc::now()).ok_or_else(||"The provider returned no supported quota window. Local token totals are still available.".into())
}
fn fetch(url: &str, headers: HeaderMap) -> Result<Value, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Quota connection could not be created.".to_owned())?;
    let response = client.get(url).headers(headers).send().map_err(|_| {
        "Could not reach the quota service. Check your connection and try Refresh.".to_owned()
    })?;
    match response.status().as_u16() {
        200 => {}
        401 | 403 => {
            return Err(
                "The provider refused this login. Sign in to the provider again, then Refresh."
                    .into(),
            )
        }
        429 => return Err("The provider is limiting quota requests. Try Refresh later.".into()),
        _ => {
            return Err(format!(
                "The quota service returned HTTP {}. Try Refresh later.",
                response.status().as_u16()
            ))
        }
    }
    let mut bytes = vec![];
    response
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Quota response could not be read.".to_owned())?;
    if bytes.len() > 1024 * 1024 {
        return Err("The quota response was too large.".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "The provider returned an unreadable quota response.".into())
}
fn quota(label: &str, percent: &Value, reset: Option<String>, now: DateTime<Utc>) -> Option<Quota> {
    let used = percent
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.)?;
    let reset_label = reset
        .as_ref()
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|date| {
            let minutes = date.signed_duration_since(now).num_minutes().max(0);
            format!("Resets in {}h {}m", minutes / 60, minutes % 60)
        });
    Some(Quota {
        label: label.into(),
        used: (used * 10.).round() / 10.,
        resets_at: reset,
        reset_label,
    })
}
fn reset(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|date| date.to_rfc3339());
    }
    value
        .as_i64()
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
        .map(|date| date.to_rfc3339())
}
fn parse(provider: &str, value: &Value, now: DateTime<Utc>) -> Option<Quota> {
    match provider {
        "claude" => [("five_hour", "Session"), ("seven_day", "Weekly")]
            .iter()
            .find_map(|(key, label)| {
                quota(
                    label,
                    &value[*key]["utilization"],
                    reset(&value[*key]["resets_at"]),
                    now,
                )
            }),
        "codex" => {
            let mut windows = vec![];
            for (key, fallback) in [
                ("primary_window", "Session"),
                ("secondary_window", "Weekly"),
            ] {
                let window = &value["rate_limit"][key];
                let label = match window["limit_window_seconds"].as_i64() {
                    Some(604800) => "Weekly",
                    Some(18000) => "Session",
                    _ => fallback,
                };
                let resets = reset(&window["reset_at"]).or_else(|| {
                    window["reset_after_seconds"]
                        .as_i64()
                        .filter(|seconds| *seconds >= 0)
                        .map(|seconds| (now + chrono::Duration::seconds(seconds)).to_rfc3339())
                });
                if let Some(quota) = quota(label, &window["used_percent"], resets, now) {
                    windows.push(quota);
                }
            }
            windows.sort_by_key(|window| if window.label == "Session" { 0 } else { 1 });
            windows.into_iter().next()
        }
        "cursor" => quota(
            "Plan",
            &value["individualUsage"]["plan"]["totalPercentUsed"],
            reset(&value["billingCycleEnd"]),
            now,
        ),
        "opencode" => [
            ("rolling", "Go Session"),
            ("weekly", "Go Weekly"),
            ("monthly", "Go Monthly"),
        ]
        .iter()
        .find_map(|(key, label)| {
            quota(
                label,
                &value["usage"][*key]["percent"],
                reset(&value["usage"][*key]["resetsAt"]),
                now,
            )
        }),
        _ => None,
    }
}
