//! Select a borrowed access token from externally owned Claude Desktop caches.
//! Never decode refresh tokens into credentials or include cache contents in errors.
use serde_json::Value;
use std::collections::{HashMap, HashSet};

const HOST: &str = "https://api.anthropic.com";
const PRODUCTION_CLIENT: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const MAX_ENTRIES: usize = 4096;
const MAX_KEY_BYTES: usize = 4096;
const MAX_TOKEN_BYTES: usize = 16 * 1024;
const EXPIRY_MARGIN_MS: f64 = 120_000.;
const INVALID: &str = "Claude Desktop login could not be read. Open Claude Desktop, then Refresh.";
const STALE: &str = "Claude Desktop login is stale. Open Claude Desktop, then Refresh.";
const NOT_FOUND: &str =
    "No usable Claude Desktop subscription login was found. Open Claude Desktop, then Refresh.";
const UNKNOWN_ORGANIZATION: &str = "Claude Desktop organization could not be determined. Open Claude Desktop and Refresh, or sign in with Claude Code.";

#[derive(Clone, Eq, Hash, PartialEq)]
struct Key {
    client: String,
    organization: String,
    scopes: Vec<String>,
}

pub(super) fn normalized_uuid(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    (bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        }))
    .then(|| raw.to_ascii_lowercase())
}

fn parse_key(raw: &str) -> Option<Key> {
    let marker = format!(":{HOST}:");
    let (prefix, scopes) = raw.split_once(&marker)?;
    let (client, organization) = prefix.split_once(':')?;
    Some(Key {
        client: normalized_uuid(client)?,
        organization: normalized_uuid(organization)?,
        scopes: scopes.split_whitespace().map(str::to_owned).collect(),
    })
}

// Scoped aliases, including null deletion markers, supersede legacy aliases.
// Foreign accounts cannot supply entries or suppress a current account's V1 token.
fn normalized_cache<'a>(
    cache: Option<&'a Value>,
    active_account: Option<&str>,
    scoped_only: bool,
) -> Result<HashMap<Key, &'a Value>, String> {
    let Some(cache) = cache else {
        return Ok(HashMap::new());
    };
    let entries = cache.as_object().ok_or(INVALID)?;
    if entries.len() > MAX_ENTRIES || entries.keys().any(|key| key.len() > MAX_KEY_BYTES) {
        return Err(INVALID.into());
    }
    let active_account = active_account.and_then(normalized_uuid);
    let mut legacy = HashMap::new();
    let mut scoped = HashMap::new();
    let mut sorted = entries.iter().collect::<Vec<_>>();
    sorted.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (raw, entry) in sorted {
        let (key, is_scoped) = if let Some(raw) = raw.strip_prefix("acct:") {
            let Some((owner, key)) = raw.split_once('|') else {
                continue;
            };
            let Some(owner) = normalized_uuid(owner) else {
                continue;
            };
            if active_account.as_deref() != Some(owner.as_str()) {
                continue;
            }
            (key, true)
        } else {
            if scoped_only {
                continue;
            }
            (raw.as_str(), false)
        };
        let Some(key) = parse_key(key) else {
            continue;
        };
        if is_scoped {
            scoped.insert(key, entry);
        } else {
            legacy.insert(key, entry);
        }
    }
    legacy.extend(scoped);
    Ok(legacy)
}

#[derive(Default)]
struct Candidates<'a> {
    best: Option<((bool, bool, usize, f64), &'a str)>,
    stale: bool,
    invalid: bool,
}

fn candidates<'a>(
    entries: impl Iterator<Item = (&'a Key, &'a Value)>,
    organization: &str,
    now_ms: f64,
) -> Candidates<'a> {
    let mut result = Candidates::default();
    for (key, entry) in entries {
        if key.organization != organization
            || !key.scopes.iter().any(|scope| scope == "user:profile")
        {
            continue;
        }
        if entry.is_null() {
            continue;
        }
        let token = entry.get("token").and_then(Value::as_str);
        let expiry = entry.get("expiresAt").and_then(Value::as_f64);
        let (Some(token), Some(expiry)) = (token, expiry) else {
            result.invalid = true;
            continue;
        };
        if token.trim().is_empty() || token.len() > MAX_TOKEN_BYTES || !expiry.is_finite() {
            result.invalid = true;
            continue;
        }
        if expiry <= now_ms + EXPIRY_MARGIN_MS {
            result.stale = true;
            continue;
        }
        let full_scope = key.scopes.iter().any(|scope| scope == "user:inference");
        let rank = (
            key.client == PRODUCTION_CLIENT && full_scope,
            full_scope,
            key.scopes.len(),
            expiry,
        );
        if result.best.as_ref().is_none_or(|(best, _)| rank > *best) {
            result.best = Some((rank, token));
        }
    }
    result
}

#[cfg(test)]
pub(super) fn select(
    v2: Option<&Value>,
    v1: Option<&Value>,
    active_account: Option<&str>,
    active_org: &str,
    now_ms: f64,
) -> Result<String, String> {
    select_mode(v2, v1, active_account, active_org, now_ms, false)
}

/// Modern Windows caches must prove the selected token belongs to the current account.
/// A shared organization alone cannot establish ownership of a legacy unscoped token.
pub(super) fn select_scoped(
    v2: Option<&Value>,
    v1: Option<&Value>,
    active_account: Option<&str>,
    active_org: &str,
    now_ms: f64,
) -> Result<String, String> {
    let account = active_account.and_then(normalized_uuid).ok_or(INVALID)?;
    select_mode(v2, v1, Some(&account), active_org, now_ms, true)
}

fn select_mode(
    v2: Option<&Value>,
    v1: Option<&Value>,
    active_account: Option<&str>,
    active_org: &str,
    now_ms: f64,
    scoped_only: bool,
) -> Result<String, String> {
    let organization = normalized_uuid(active_org).ok_or(INVALID)?;
    if !now_ms.is_finite() {
        return Err(INVALID.into());
    }
    let v2 = normalized_cache(v2, active_account, scoped_only)?;
    let v1 = normalized_cache(v1, active_account, scoped_only)?;
    let current = candidates(
        v2.iter().map(|(key, value)| (key, *value)),
        &organization,
        now_ms,
    );
    if let Some((_, token)) = current.best {
        return Ok(token.into());
    }
    // Presence, rather than validity, prevents resurrecting deleted/expired V2 entries.
    let legacy = candidates(
        v1.iter()
            .filter(|(key, _)| !v2.contains_key(*key))
            .map(|(key, value)| (key, *value)),
        &organization,
        now_ms,
    );
    if let Some((_, token)) = legacy.best {
        return Ok(token.into());
    }
    Err(if current.stale || legacy.stale {
        STALE
    } else if current.invalid || legacy.invalid {
        INVALID
    } else {
        NOT_FOUND
    }
    .into())
}

/// A locked cookie database can only fall back to one provable account-scoped org.
/// Include tombstones and stale entries: another org must never be selected by expiry.
pub(super) fn single_organization(
    v2: Option<&Value>,
    v1: Option<&Value>,
    active_account: Option<&str>,
) -> Result<String, String> {
    let account = active_account
        .and_then(normalized_uuid)
        .ok_or(UNKNOWN_ORGANIZATION)?;
    let v2 = normalized_cache(v2, Some(&account), true)?;
    let v1 = normalized_cache(v1, Some(&account), true)?;
    let organizations = v2
        .keys()
        .chain(v1.keys().filter(|key| !v2.contains_key(*key)))
        .filter(|key| key.scopes.iter().any(|scope| scope == "user:profile"))
        .map(|key| key.organization.as_str())
        .collect::<HashSet<_>>();
    if organizations.len() != 1 {
        return Err(UNKNOWN_ORGANIZATION.into());
    }
    Ok(organizations.into_iter().next().unwrap().into())
}

#[cfg(test)]
#[path = "cache/tests.rs"]
mod tests;
