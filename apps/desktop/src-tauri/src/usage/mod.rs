mod claude;
#[cfg(any(target_os = "macos", test))]
mod claude_roots;
mod codex;
mod cursor;
mod daily;
mod limits;
#[cfg(all(test, target_os = "macos"))]
mod mac_live_qa;
#[cfg(target_os = "macos")]
pub fn handle_keychain_helper() -> bool {
    limits::handle_keychain_helper()
}
mod opencode;
mod project;
#[cfg(test)]
mod project_tests;
pub use project::Project;
mod scan;
mod synara_roots;

use crate::cloud::records::{normalize, CloudRecord};
use crate::pricing::PricingStore;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::Mutex};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageEvent {
    pub id: String,
    pub provider: String,
    pub timestamp: String,
    pub model: String,
    pub tokens: u64,
    #[serde(rename = "costUSD", alias = "costUsd")]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub unpriced_tokens: u64,
    #[serde(default)]
    pub cost_source: Option<CostSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<Project>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usd_acronym_matches_the_frontend_and_older_caches_still_load() {
        let event = UsageEvent {
            id: "test".into(),
            provider: "codex".into(),
            timestamp: "2026-09-30T12:00:00Z".into(),
            model: "gpt-6-sol".into(),
            tokens: 100,
            cost_usd: Some(0.01),
            unpriced_tokens: 0,
            cost_source: Some(CostSource::Estimated),
            project: None,
        };
        let mut value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["costUSD"], 0.01);
        assert!(value.get("costUsd").is_none());
        assert_eq!(value["unpricedTokens"], 0);
        assert_eq!(value["costSource"], "estimated");
        let old = value.as_object_mut().unwrap().remove("costUSD").unwrap();
        value["costUsd"] = old;
        let decoded: UsageEvent = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.cost_usd, Some(0.01));
    }
    fn record(provider: &str, id: &str) -> scan::Record {
        scan::Record::plain(UsageEvent {
            id: id.into(),
            provider: provider.into(),
            timestamp: "2026-10-01T12:00:00Z".into(),
            model: "composer-2.5".into(),
            tokens: 10,
            cost_usd: None,
            unpriced_tokens: 0,
            cost_source: None,
            project: None,
        })
        .with_breakdown(crate::pricing::TokenBreakdown {
            input: 10,
            ..Default::default()
        })
    }
    #[test]
    fn legacy_manual_import_fields_and_snapshot_rows_are_discarded_during_migration() {
        for version in 1..=3 {
            let imported = record("cursor", "cursor-csv:manual-import");
            let codex = record("codex", "preserved-codex");
            let snapshot = Snapshot {
                events: vec![imported.event.clone(), codex.event.clone()],
                available_providers: vec!["cursor".into(), "codex".into()],
                ..Default::default()
            };
            let value = serde_json::json!({
                "version": version, "files": {}, "snapshot": snapshot,
                "cursor": [imported.event], "cursor_records": [imported],
                "cursor_imported_at": "2026-10-01T12:00:00Z",
                "cursor_api": {"source": "cursor-account-api", "accountHash": crate::cloud::records::hash("synthetic-account"),
                    "fetchedAt": "2026-10-01T12:00:00Z", "records": [record("cursor", "cursor-api:old-cache-claim")]}
            });
            let mut cache: Cache = serde_json::from_value(value).unwrap();
            cache.migrate();
            assert_eq!(cache.version, 5);
            assert!(cache.cursor_api.is_none());
            assert_eq!(cache.snapshot.events.len(), 1);
            assert_eq!(cache.snapshot.events[0].id, "preserved-codex");
            assert_eq!(cache.snapshot.available_providers, vec!["codex"]);
            let migrated = serde_json::to_value(&cache).unwrap();
            for key in ["cursor", "cursor_records", "cursor_imported_at"] {
                assert!(migrated.get(key).is_none());
            }
            assert!(!migrated.to_string().contains("manual-import"));
        }
    }
    #[test]
    fn only_new_schema_api_provenance_and_billable_buckets_survive_cache_reload() {
        let api = cursor::VerifiedUsage {
            source: "cursor-account-api".into(),
            account_hash: crate::cloud::records::hash("synthetic-account"),
            fetched_at: "2026-10-01T12:00:00Z".into(),
            records: vec![record("cursor", "cursor-api:verified")],
        };
        let mut cache = Cache {
            version: 4,
            cursor_api: Some(api.clone()),
            ..Default::default()
        };
        cache.migrate();
        assert!(cache.cursor_api.is_some());
        let mut imported_claim = api;
        imported_claim.records[0].event.id = "cursor-csv:legacy".into();
        cache.cursor_api = Some(imported_claim);
        cache.migrate();
        assert!(cache.cursor_api.is_none());
    }
    #[test]
    fn one_cursor_fetch_uploads_the_complete_same_revision_at_zero_and_two_minutes() {
        let mut recent = record("cursor", "cursor-api:recent").event;
        recent.timestamp = "2026-10-01T12:01:55Z".into();
        let mut earlier = record("cursor", "cursor-api:earlier").event;
        earlier.timestamp = "2026-10-01T11:57:00Z".into();
        let mut append = record("codex", "recent-append").event;
        append.timestamp = recent.timestamp.clone();
        let events = vec![recent, earlier, append];
        let now = DateTime::parse_from_rfc3339("2026-10-01T12:02:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let revision = Some("2026-10-01T12:02:00Z");
        let initial = normalize(&syncable_events(&events, now), revision);
        let later = normalize(
            &syncable_events(&events, now + Duration::minutes(2)),
            revision,
        );
        assert_eq!(initial.len(), 1);
        assert_eq!(initial[0].tokens, 20);
        assert_eq!(initial[0].source_id.as_deref(), Some("cursor-account-api"));
        let later_cursor = later
            .iter()
            .find(|record| record.provider == "Cursor")
            .unwrap();
        assert_eq!(
            serde_json::to_value(&initial[0]).unwrap(),
            serde_json::to_value(later_cursor).unwrap()
        );
        assert_eq!(initial[0].acknowledgement(), later_cursor.acknowledgement());
        assert!(later.iter().any(|record| record.provider == "Codex"));
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CostSource {
    Recorded,
    Estimated,
    Mixed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quota {
    pub label: String,
    pub used: f64,
    pub resets_at: Option<String>,
    pub reset_label: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub events: Vec<UsageEvent>,
    pub fetched_at: Option<String>,
    pub source: String,
    pub errors: Vec<String>,
    pub available_providers: Vec<String>,
    pub quotas: HashMap<String, Quota>,
    #[serde(default)]
    pub quota_errors: HashMap<String, String>,
    #[serde(default)]
    pub provider_errors: HashMap<String, String>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            fetched_at: None,
            source: "local".into(),
            errors: Vec::new(),
            available_providers: Vec::new(),
            quotas: HashMap::new(),
            quota_errors: HashMap::new(),
            provider_errors: HashMap::new(),
        }
    }
}
#[derive(Default, Serialize, Deserialize)]
struct Cache {
    version: u8,
    files: HashMap<PathBuf, scan::CachedFile>,
    snapshot: Snapshot,
    #[serde(default)]
    cursor_api: Option<cursor::VerifiedUsage>,
    #[serde(skip)]
    cursor_refresh: cursor::FetchState,
}
impl Cache {
    fn migrate(&mut self) {
        // Reparse unchanged local source files once to recover explicit workspace metadata.
        // Verified Cursor API history is independent of this file cache and stays intact.
        if self.version < 5 {
            self.files.clear();
        }
        // A legacy import or daily aggregate is never proof of an automatic API fetch.
        if self.version < 4 || self.cursor_api.as_ref().is_some_and(|usage| !usage.valid()) {
            self.cursor_api = None;
        }
        self.snapshot
            .events
            .retain(|event| event.provider != "cursor");
        self.snapshot
            .available_providers
            .retain(|provider| provider != "cursor");
        self.snapshot.provider_errors.remove("cursor");
        self.version = 5;
    }
}
pub struct UsageStore {
    path: PathBuf,
    cache: Mutex<Cache>,
    snapshot: Mutex<Snapshot>,
    pricing: PricingStore,
    cloud_events: Mutex<Vec<UsageEvent>>,
}

pub fn authorize_login(source: &str) -> Result<(), String> {
    limits::authorize_login(source)
}
impl UsageStore {
    pub fn load(directory: PathBuf) -> Self {
        let path = directory.join("usage-cache.json");
        let mut cache = crate::storage::read_json::<Cache>(&path)
            .ok()
            .filter(|cache| matches!(cache.version, 1..=5))
            .unwrap_or_default();
        cache.migrate();
        if let Some(error) = cursor::discard_unusable_account(&mut cache.cursor_api) {
            cache
                .snapshot
                .provider_errors
                .insert("cursor".into(), error);
            cache.snapshot.quotas.remove("cursor");
        }
        let pricing = PricingStore::load(directory.join("pricing"));
        if let Some(verified) = &cache.cursor_api {
            cache.snapshot.events.extend(
                verified
                    .records
                    .iter()
                    .map(|record| record.priced(&pricing.current())),
            );
            cache.snapshot.available_providers.push("cursor".into());
        }
        cache.snapshot.events = daily::totals(std::mem::take(&mut cache.snapshot.events));
        let snapshot = Mutex::new(cache.snapshot.clone());
        let store = Self {
            path,
            cache: Mutex::new(cache),
            snapshot,
            pricing,
            cloud_events: Mutex::new(Vec::new()),
        };
        // Persist migration now, including when damaged source settings prevent a later scan.
        if let Ok(mut cache) = store.cache.lock() {
            if let Err(error) = store.save(&cache) {
                eprintln!("Usage cache migration save failed: {error}");
                cache
                    .snapshot
                    .errors
                    .push("Saved usage could not be updated on this device. Try Refresh.".into());
                if let Ok(mut snapshot) = store.snapshot.lock() {
                    *snapshot = cache.snapshot.clone();
                }
            }
        }
        store
    }
    pub fn read(&self) -> Result<Snapshot, String> {
        let mut cache = self.cache.lock().map_err(|_| "Usage cache unavailable")?;
        self.discard_unusable_cursor(&mut cache)?;
        Ok(self
            .snapshot
            .lock()
            .map_err(|_| "Usage cache unavailable")?
            .clone())
    }
    pub fn refresh(&self) -> Result<Snapshot, String> {
        self.refresh_with(false)
    }
    pub fn refresh_forced(&self) -> Result<Snapshot, String> {
        self.refresh_with(true)
    }
    fn refresh_with(&self, force: bool) -> Result<Snapshot, String> {
        let mut cache = self.cache.lock().map_err(|_| "Usage cache unavailable")?;
        self.discard_unusable_cursor(&mut cache)?;
        if let Some(reason) = crate::paths::configuration_error() {
            let mut snapshot = cache.snapshot.clone();
            if !snapshot.errors.contains(&reason) {
                snapshot.errors.push(reason);
            }
            *self
                .snapshot
                .lock()
                .map_err(|_| "Usage cache unavailable")? = snapshot.clone();
            return Ok(snapshot);
        }
        let pricing = self.pricing.current();
        let mut snapshot = scan::scan(&mut cache.files, &pricing);
        let Cache {
            cursor_api,
            cursor_refresh,
            ..
        } = &mut *cache;
        if let Some(error) = cursor_refresh.refresh(cursor_api, force) {
            snapshot.provider_errors.insert("cursor".into(), error);
        }
        if let Some(verified) = &cache.cursor_api {
            snapshot.events.extend(
                verified
                    .records
                    .iter()
                    .map(|record| record.priced(&pricing)),
            );
            snapshot.available_providers.push("cursor".into());
        }
        *self
            .cloud_events
            .lock()
            .map_err(|_| "Cloud usage unavailable")? = snapshot.events.clone();
        snapshot.events = daily::totals(snapshot.events);
        let limits = limits::read();
        snapshot.quotas = limits.quotas;
        snapshot.quota_errors = limits.errors;
        snapshot.fetched_at = Some(Utc::now().to_rfc3339());
        snapshot.errors.extend(self.pricing.warnings());
        cache.version = 5;
        cache.snapshot = snapshot.clone();
        if let Err(error) = self.save(&cache) {
            eprintln!("Usage cache save failed: {error}");
            snapshot
                .errors
                .push("Usage is available, but it could not be saved on this device.".into());
            cache.snapshot.errors = snapshot.errors.clone();
        }
        *self
            .snapshot
            .lock()
            .map_err(|_| "Usage cache unavailable")? = snapshot.clone();
        Ok(snapshot)
    }
    pub fn cloud_records(&self) -> Result<Vec<CloudRecord>, String> {
        let mut cache = self.cache.lock().map_err(|_| "Usage cache unavailable")?;
        self.discard_unusable_cursor(&mut cache)?;
        let events = self
            .cloud_events
            .lock()
            .map_err(|_| "Cloud usage unavailable")?;
        let mature = syncable_events(&events, Utc::now());
        Ok(normalize(
            &mature,
            cache
                .cursor_api
                .as_ref()
                .map(|usage| usage.fetched_at.as_str()),
        ))
    }
    fn discard_unusable_cursor(&self, cache: &mut Cache) -> Result<(), String> {
        if let Some(error) = cursor::discard_unusable_account(&mut cache.cursor_api) {
            self.cloud_events
                .lock()
                .map_err(|_| "Cloud usage unavailable")?
                .retain(|event| event.provider != "cursor");
            cache
                .snapshot
                .events
                .retain(|event| event.provider != "cursor");
            cache
                .snapshot
                .available_providers
                .retain(|provider| provider != "cursor");
            cache.snapshot.quotas.remove("cursor");
            cache
                .snapshot
                .provider_errors
                .insert("cursor".into(), error);
            *self
                .snapshot
                .lock()
                .map_err(|_| "Usage cache unavailable")? = cache.snapshot.clone();
            self.save(cache)?;
        }
        Ok(())
    }
    fn save(&self, cache: &Cache) -> Result<(), String> {
        std::fs::create_dir_all(self.path.parent().ok_or("Invalid cache directory")?)
            .map_err(|error| error.to_string())?;
        crate::storage::write_json(&self.path, cache)
    }
}
fn syncable_events(events: &[UsageEvent], now: DateTime<Utc>) -> Vec<UsageEvent> {
    // Append-based source messages may still be settling. A verified API response is complete
    // for its fetch revision and must not gain additional rows just because time passes.
    events
        .iter()
        .filter(|event| {
            event.provider == "cursor"
                || DateTime::parse_from_rfc3339(&event.timestamp)
                    .is_ok_and(|date| date <= now - Duration::minutes(2))
        })
        .cloned()
        .collect()
}
