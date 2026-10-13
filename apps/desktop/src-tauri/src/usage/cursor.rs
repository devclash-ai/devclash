//! Measured dashboard history, fetched read-only using Cursor's existing local login.
use super::{limits::auth, scan::Record, UsageEvent};
use crate::{cloud::records::hash, pricing::TokenBreakdown};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use reqwest::header::{ACCEPT, COOKIE, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, io::Read, time::Instant};

pub const SOURCE: &str = "cursor-account-api";
const URL: &str = "https://cursor.com/api/dashboard/export-usage-events-csv";
const MAX_BODY: usize = 10 * 1024 * 1024;
const MAX_ROWS: usize = 100_000;
const MAX_TOKENS: u64 = 9_007_199_254_740_991;
const INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);
const COLUMNS: [&str; 6] = [
    "Date",
    "Model",
    "Input (w/ Cache Write)",
    "Input (w/o Cache Write)",
    "Cache Read",
    "Output Tokens",
];
struct FetchError {
    message: String,
    unauthenticated: bool,
}
impl From<String> for FetchError {
    fn from(message: String) -> Self {
        Self {
            message,
            unauthenticated: false,
        }
    }
}
impl From<&str> for FetchError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedUsage {
    pub source: String,
    #[serde(default)]
    pub account_hash: String,
    pub fetched_at: String,
    pub records: Vec<Record>,
}
impl VerifiedUsage {
    pub fn valid(&self) -> bool {
        self.source == SOURCE
            && self.account_hash.len() == 64
            && self
                .account_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            && DateTime::parse_from_rfc3339(&self.fetched_at).is_ok()
            && self.records.len() <= MAX_ROWS
            && self.records.iter().all(|record| {
                record.event.provider == "cursor"
                    && record.event.id.starts_with("cursor-api:")
                    && DateTime::parse_from_rfc3339(&record.event.timestamp).is_ok()
                    && record.breakdown.is_some_and(|buckets| {
                        [
                            buckets.input,
                            buckets.cache_write_5m,
                            buckets.cache_read,
                            buckets.output,
                        ]
                        .into_iter()
                        .try_fold(0_u64, u64::checked_add)
                            == Some(record.event.tokens)
                    })
            })
    }
}
// Missing, expired, or changed logins must not expose cached rows as current account usage.
pub fn discard_unusable_account(cached: &mut Option<VerifiedUsage>) -> Option<String> {
    if cached.is_none() || std::env::var("DEVCLASH_CURSOR_FETCH_DISABLED").as_deref() == Ok("1") {
        return None;
    }
    let error =
        match auth::load("cursor").and_then(|credentials| auth::cursor_account(&credentials)) {
            Ok(account)
                if cached
                    .as_ref()
                    .is_some_and(|usage| usage.account_hash == hash(&account)) =>
            {
                None
            }
            Ok(_) => Some("Cursor account changed. Refresh to load its usage.".into()),
            Err(error) => Some(error),
        };
    if let Some(error) = &error {
        eprintln!("Cursor usage history unavailable: {error}");
        *cached = None;
    }
    error
}

#[derive(Default)]
pub struct FetchState {
    last_attempt: Option<Instant>,
    error: Option<String>,
}
impl FetchState {
    pub fn refresh(&mut self, cached: &mut Option<VerifiedUsage>, force: bool) -> Option<String> {
        if std::env::var("DEVCLASH_CURSOR_FETCH_DISABLED").as_deref() == Ok("1") {
            return None;
        }
        self.refresh_login(
            cached,
            force,
            auth::load("cursor"),
            |credentials, account| fetch(Utc::now(), credentials, account),
        )
    }
    fn refresh_login(
        &mut self,
        cached: &mut Option<VerifiedUsage>,
        force: bool,
        login: Result<auth::Credentials, String>,
        fetch: impl FnOnce(&auth::Credentials, &str) -> Result<VerifiedUsage, FetchError>,
    ) -> Option<String> {
        let login = login.and_then(|credentials| {
            let account = auth::cursor_account(&credentials)?;
            Ok((credentials, hash(&account)))
        });
        let (credentials, account) = match login {
            Ok(login) => login,
            Err(error) => {
                eprintln!("Cursor usage history unavailable: {error}");
                *cached = None;
                self.last_attempt = None;
                self.error = Some(error);
                return self.error.clone();
            }
        };
        self.refresh_with(cached, force, Instant::now(), &account, || {
            fetch(&credentials, &account)
        })
    }
    fn refresh_with(
        &mut self,
        cached: &mut Option<VerifiedUsage>,
        force: bool,
        now: Instant,
        account: &str,
        fetch: impl FnOnce() -> Result<VerifiedUsage, FetchError>,
    ) -> Option<String> {
        if cached
            .as_ref()
            .is_some_and(|usage| usage.account_hash != account)
        {
            *cached = None;
            self.last_attempt = None;
        }
        if !force
            && self
                .last_attempt
                .is_some_and(|last| now.duration_since(last) < INTERVAL)
        {
            return self.error.clone();
        }
        self.last_attempt = Some(now);
        match fetch() {
            Ok(verified) => {
                *cached = Some(verified);
                self.error = None;
            }
            Err(error) => {
                if error.unauthenticated {
                    *cached = None;
                }
                eprintln!("Cursor usage history unavailable: {}", error.message);
                self.error = Some(error.message);
            }
        }
        self.error.clone()
    }
}

fn fetch(
    end: DateTime<Utc>,
    credentials: &auth::Credentials,
    account: &str,
) -> Result<VerifiedUsage, FetchError> {
    // Include 90 UTC calendar days, including today's current measured total.
    let start = end.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc() - Duration::days(89);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Cursor history connection could not be created.")?;
    let url = format!(
        "{URL}?startDate={}&endDate={}&strategy=tokens",
        start.timestamp_millis(),
        end.timestamp_millis()
    );
    let response = client
        .get(url)
        .header(COOKIE, auth::cursor_cookie(credentials)?)
        .header(ACCEPT, "text/csv")
        .header(USER_AGENT, "DevClash")
        .send()
        .map_err(|_| {
            "Could not reach Cursor usage history. Check your connection and try Refresh."
        })?;
    let records = match response.status().as_u16() {
        204 => Vec::new(),
        200 => {
            if response
                .content_length()
                .is_some_and(|size| size > MAX_BODY as u64)
            {
                return Err("Cursor usage history exceeded the 10 MB response limit.".into());
            }
            let mut bytes = Vec::new();
            response
                .take(MAX_BODY as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "Cursor usage history could not be read. Try Refresh.")?;
            if bytes.len() > MAX_BODY {
                return Err("Cursor usage history exceeded the 10 MB response limit.".into());
            }
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| "Cursor returned unreadable usage history. Try Refresh.")?;
            parse(text)?
                .into_iter()
                .filter(|record| {
                    DateTime::parse_from_rfc3339(&record.event.timestamp)
                        .is_ok_and(|date| date >= start && date <= end)
                })
                .collect()
        }
        401 | 403 => {
            return Err(FetchError {
                message: "Cursor refused this login. Sign in to Cursor again, then Refresh.".into(),
                unauthenticated: true,
            })
        }
        429 => {
            return Err("Cursor is limiting history requests. Try Refresh in a few minutes.".into())
        }
        status => {
            return Err(format!("Cursor history returned HTTP {status}. Try Refresh later.").into())
        }
    };
    Ok(VerifiedUsage {
        source: SOURCE.into(),
        account_hash: account.into(),
        fetched_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        records,
    })
}

fn count(raw: &str, row: usize) -> Result<u64, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(0);
    }
    let groups = value.split(',').collect::<Vec<_>>();
    if groups
        .iter()
        .any(|group| group.is_empty() || !group.bytes().all(|byte| byte.is_ascii_digit()))
        || groups.len() > 1
            && (groups[0].len() > 3 || groups[1..].iter().any(|group| group.len() != 3))
    {
        return Err(format!(
            "Cursor history row {row} has an invalid token count."
        ));
    }
    groups
        .concat()
        .parse::<u64>()
        .ok()
        .filter(|count| *count <= MAX_TOKENS)
        .ok_or_else(|| format!("Cursor history row {row} has an invalid token count."))
}

fn parse(text: &str) -> Result<Vec<Record>, String> {
    if text.len() > MAX_BODY {
        return Err("Cursor usage history exceeded the 10 MB response limit.".into());
    }
    if text.bytes().filter(|byte| *byte == b'"').count() % 2 != 0 {
        return Err("Cursor returned incomplete usage history. Try Refresh.".into());
    }
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(text.trim_start_matches('\u{feff}').as_bytes());
    let headers = reader
        .headers()
        .map_err(|_| "Cursor usage history could not be read.")?
        .clone();
    if headers.len() > 64 || headers.iter().collect::<HashSet<_>>().len() != headers.len() {
        return Err("Cursor usage history has invalid column names.".into());
    }
    let columns = COLUMNS
        .iter()
        .map(|name| {
            headers
                .iter()
                .position(|header| header == *name)
                .ok_or_else(|| format!("Cursor usage history is missing the {name} column."))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let total_column = headers.iter().position(|header| header == "Total Tokens");
    let origin_columns = ["Cloud Agent ID", "Automation ID"]
        .map(|name| headers.iter().position(|header| header == name));
    let mut seen = HashSet::new();
    let mut events = Vec::new();
    let mut all_tokens = 0_u64;
    for (index, record) in reader.records().enumerate() {
        if index >= MAX_ROWS {
            return Err("Cursor usage history has too many rows.".into());
        }
        let row_number = index + 2;
        let row = record.map_err(|_| format!("Cursor history row {row_number} is malformed."))?;
        // The API supplies explicit UTC offsets. Never infer a timezone from the local device.
        let timestamp = DateTime::parse_from_rfc3339(&row[columns[0]])
            .map(|date| {
                date.with_timezone(&Utc)
                    .to_rfc3339_opts(SecondsFormat::Millis, true)
            })
            .map_err(|_| format!("Cursor history row {row_number} has no valid UTC timestamp."))?;
        let model = row[columns[1]].trim();
        if model.is_empty() || model.len() > 128 || model.chars().any(char::is_control) {
            return Err(format!(
                "Cursor history row {row_number} has an invalid model."
            ));
        }
        let counts = columns[2..]
            .iter()
            .map(|column| count(&row[*column], row_number))
            .collect::<Result<Vec<_>, _>>()?;
        let tokens = counts
            .iter()
            .try_fold(0_u64, |total, count| total.checked_add(*count))
            .filter(|total| *total <= MAX_TOKENS)
            .ok_or("Cursor history token count is too large.")?;
        if let Some(column) = total_column {
            if !row[column].trim().is_empty() && count(&row[column], row_number)? != tokens {
                return Err(format!(
                    "Cursor history row {row_number} has inconsistent token totals."
                ));
            }
        }
        let origins = origin_columns.map(|column| column.map(|column| &row[column]).unwrap_or(""));
        let identity = serde_json::to_string(&(&timestamp, model, &counts, origins)).unwrap();
        let id = format!("cursor-api:{}", hash(&identity));
        if tokens == 0 || !seen.insert(id.clone()) {
            continue;
        }
        all_tokens = all_tokens
            .checked_add(tokens)
            .filter(|total| *total <= MAX_TOKENS)
            .ok_or("Cursor history token total is too large.")?;
        events.push(
            Record::plain(UsageEvent {
                id,
                provider: "cursor".into(),
                timestamp,
                model: model.into(),
                tokens,
                cost_usd: None,
                unpriced_tokens: 0,
                cost_source: None,
                project: None,
            })
            .with_breakdown(TokenBreakdown {
                cache_write_5m: counts[0],
                input: counts[1],
                cache_read: counts[2],
                output: counts[3],
                ..TokenBreakdown::default()
            }),
        );
    }
    events.sort_by(|a, b| {
        a.event
            .timestamp
            .cmp(&b.event.timestamp)
            .then(a.event.id.cmp(&b.event.id))
    });
    Ok(events)
}

#[cfg(test)]
mod tests;
