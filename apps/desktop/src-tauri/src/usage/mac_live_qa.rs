//! Explicit real-account diagnostics. Keep provider data and credentials out of test output.
use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "Opt-in real local collection and provider API check; use DEVCLASH_MAC_LIVE_QA=1"]
fn actual_mac_collector_and_provider_quotas() {
    assert_eq!(std::env::var("DEVCLASH_MAC_LIVE_QA").as_deref(), Ok("1"));
    for variable in [
        "DEVCLASH_QUOTAS_DISABLED",
        "DEVCLASH_CURSOR_FETCH_DISABLED",
        "DEVCLASH_SOURCE_SETTINGS",
        "DEVCLASH_CLAUDE_DIRS",
        "DEVCLASH_CODEX_DIRS",
        "DEVCLASH_CURSOR_DIRS",
        "DEVCLASH_OPENCODE_DIRS",
    ] {
        assert!(
            std::env::var_os(variable).is_none(),
            "Run with automatic source discovery and live APIs enabled"
        );
    }
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let directory = Scratch(std::env::temp_dir().join(format!(
        "devclash-mac-live-{}-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )));
    // A daily refresh reuses verified native file metadata. Avoid rereading tens of GB
    // of unchanged prompt-bearing logs just to repeat a real-account smoke check.
    std::fs::create_dir_all(&directory.0).unwrap();
    let native_cache = dirs::data_local_dir()
        .unwrap()
        .join("ai.devclash.desktop/usage-cache.json");
    let reused_native_file_cache = native_cache.is_file();
    if reused_native_file_cache {
        std::fs::copy(native_cache, directory.0.join("usage-cache.json")).unwrap();
    }
    let store = UsageStore::load(directory.0.clone());
    let snapshot = store
        .refresh_forced()
        .expect("Actual metadata collection must finish");
    let since = Utc::now() - Duration::minutes(15);
    // The UI snapshot is daily totals; use the collector's original metadata timestamps here.
    let raw = store.cloud_events.lock().unwrap();
    let providers = ["claude", "codex", "cursor", "opencode"].map(|provider| {
        let recent = raw.iter().any(|event| event.provider == provider && event.tokens > 0 &&
            DateTime::parse_from_rfc3339(&event.timestamp).is_ok_and(|date| date >= since));
        (provider, json!({"measured_history":snapshot.events.iter().any(|event| event.provider==provider && event.tokens>0),
            "recent_measured_usage":recent,"quota_available":snapshot.quotas.contains_key(provider),
            "keychain_permission_required":snapshot.quota_errors.get(provider).is_some_and(|reason|reason.contains("Keychain access is required")),
            "usage_error":snapshot.provider_errors.contains_key(provider)}))
    }).into_iter().collect::<std::collections::BTreeMap<_,_>>();
    println!(
        "DEVCLASH_MAC_COLLECTOR_RESULT={}",
        json!({"providers":providers,"cloud_sync_invoked":false,"reused_native_file_cache":reused_native_file_cache})
    );
}
