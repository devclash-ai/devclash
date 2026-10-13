use super::super::CostSource;
use super::*;
use crate::pricing::ModelPricing;
use serde_json::json;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const CREATED: i64 = 1_790_812_800_000; // 2026-10-01 UTC
fn since() -> DateTime<Utc> {
    DateTime::from_timestamp_millis(CREATED - 86_400_000).unwrap()
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "devclash-opencode-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn database(&self, name: &str) -> Connection {
        let db = Connection::open(self.0.join(name)).unwrap();
        db.execute_batch("CREATE TABLE message (id TEXT PRIMARY KEY, time_created INTEGER, data TEXT);
            CREATE TABLE session_message (id TEXT PRIMARY KEY, type TEXT, time_created INTEGER, data TEXT);").unwrap();
        db
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn v1(db: &Connection, id: &str, data: Value) {
    db.execute(
        "INSERT INTO message VALUES (?1,?2,?3)",
        rusqlite::params![id, CREATED, data.to_string()],
    )
    .unwrap();
}
fn v2(db: &Connection, id: &str, kind: &str, data: Value) {
    db.execute(
        "INSERT INTO session_message VALUES (?1,?2,?3,?4)",
        rusqlite::params![id, kind, CREATED, data.to_string()],
    )
    .unwrap();
}
fn message(provider: &str, cost: Value) -> Value {
    json!({"role":"assistant","providerID":provider,"modelID":"custom-model","cost":cost,
        "tokens":{"input":100,"output":20,"reasoning":5,"cache":{"read":30,"write":40}},
        "time":{"completed":CREATED+1000},"content":"PRIVATE_RESPONSE_MUST_NOT_BE_RETAINED"})
}

#[test]
fn reads_all_providers_preserves_recorded_costs_and_omits_private_content() {
    let fixture = Fixture::new();
    let db = fixture.database("opencode.db");
    v1(&db, "go", message("opencode-go", json!(1.5)));
    v1(&db, "free", message("opencode", json!(0)));
    v1(&db, "custom", message("routeway", json!(0.02)));
    let result = scan_directory(&fixture.0, since());
    assert!(result.available);
    assert!(result.errors.is_empty());
    assert_eq!(result.records.len(), 3);
    assert_eq!(
        result.records.iter().map(|r| r.event.tokens).sum::<u64>(),
        585
    );
    let pricing = ModelPricing::bundled();
    for record in &result.records {
        assert_eq!(record.event.provider, "opencode");
        assert_eq!(
            record.priced(&pricing).cost_source,
            Some(CostSource::Recorded)
        );
        assert!(!serde_json::to_string(record)
            .unwrap()
            .contains("PRIVATE_RESPONSE"));
    }
    assert_eq!(
        result
            .records
            .iter()
            .find(|r| r.event.id == "free")
            .unwrap()
            .priced(&pricing)
            .cost_usd,
        Some(0.)
    );
}

#[test]
fn deduplicates_migrated_messages_and_channel_copies_and_counts_completed_compaction() {
    let fixture = Fixture::new();
    let stable = fixture.database("opencode.db");
    v1(&stable, "copied", message("opencode-go", json!(0.1)));
    v2(
        &stable,
        "copied",
        "assistant",
        json!({"model":{"id":"new-model","providerID":"opencode-go"},
        "cost":0.2,"tokens":{"input":200,"output":50}}),
    );
    v2(
        &stable,
        "done",
        "compaction",
        json!({"status":"completed","model":{"id":"summary-model"},"cost":0.01,"tokens":{"total":10}}),
    );
    v2(
        &stable,
        "running",
        "compaction",
        json!({"status":"pending","tokens":{"total":1000},"cost":1}),
    );
    v2(
        &stable,
        "user",
        "user",
        json!({"tokens":{"total":1000},"cost":1}),
    );
    let preview = fixture.database("opencode-next.db");
    v1(&preview, "copied", message("opencode-go", json!(0.1)));
    let result = scan_directory(&fixture.0, since());
    assert!(result.errors.is_empty());
    assert_eq!(result.records.len(), 2);
    assert_eq!(
        result.records.iter().map(|r| r.event.tokens).sum::<u64>(),
        260
    );
    let copied = result
        .records
        .iter()
        .find(|r| r.event.id == "copied")
        .unwrap();
    assert_eq!(copied.event.model, "new-model");
    assert_eq!(copied.event.cost_usd, Some(0.2));
}

#[test]
fn total_fallback_pricing_and_unknown_models_do_not_lose_usage() {
    let fixture = Fixture::new();
    let db = fixture.database("opencode.db");
    let mut known = message("anthropic", Value::Null);
    known["modelID"] = json!("claude-sonnet-4-6");
    known["tokens"]["total"] = json!(300);
    v1(&db, "priced", known);
    let mut unknown = message("custom", Value::Null);
    unknown["tokens"]["total"] = json!(400);
    v1(&db, "unknown", unknown);
    v1(
        &db,
        "total-only",
        json!({"role":"assistant","modelID":"claude-sonnet-4-6","tokens":{"total":500}}),
    );
    let result = scan_directory(&fixture.0, since());
    let pricing = ModelPricing::bundled();
    let events = result
        .records
        .iter()
        .map(|r| r.priced(&pricing))
        .collect::<Vec<_>>();
    assert_eq!(events.iter().map(|e| e.tokens).sum::<u64>(), 1200);
    let priced = events.iter().find(|e| e.id == "priced").unwrap();
    assert_eq!(priced.cost_source, Some(CostSource::Estimated));
    assert!(priced.cost_usd.unwrap() > 0.);
    assert_eq!(
        events
            .iter()
            .find(|e| e.id == "unknown")
            .unwrap()
            .unpriced_tokens,
        400
    );
    assert_eq!(
        events
            .iter()
            .find(|e| e.id == "total-only")
            .unwrap()
            .cost_usd,
        None
    );
}

#[test]
fn reads_v2_only_databases_and_rejects_old_user_and_malformed_rows() {
    let fixture = Fixture::new();
    let db = fixture.database("opencode.db");
    db.execute_batch("DROP TABLE message").unwrap();
    v2(
        &db,
        "valid",
        "assistant",
        json!({"model":{"id":"v2-model"},"cost":0,"tokens":{"total":42}}),
    );
    db.execute(
        "INSERT INTO session_message VALUES ('invalid','assistant',?1,'{broken')",
        [CREATED],
    )
    .unwrap();
    db.execute(
        "INSERT INTO session_message VALUES ('old','assistant',?1,?2)",
        rusqlite::params![
            CREATED - 172_800_000,
            message("opencode", json!(1)).to_string()
        ],
    )
    .unwrap();
    v2(&db, "user", "user", message("opencode", json!(1)));
    let result = scan_directory(&fixture.0, since());
    assert!(result.available);
    assert!(result.errors.is_empty());
    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].event.model, "v2-model");
    assert_eq!(result.records[0].event.tokens, 42);
}

#[test]
fn a_bad_channel_reports_an_error_without_hiding_valid_usage() {
    let fixture = Fixture::new();
    let db = fixture.database("opencode.db");
    v1(&db, "valid", message("opencode", json!(0)));
    fs::write(fixture.0.join("opencode-broken.db"), "not SQLite").unwrap();
    fs::write(fixture.0.join("other.db"), "must be ignored").unwrap();
    let result = scan_directory(&fixture.0, since());
    assert!(result.available);
    assert_eq!(result.records.len(), 1);
    assert_eq!(result.errors.len(), 1);
    assert!(!scan_directory(&fixture.0.join("missing"), since()).available);
}

#[test]
fn read_only_scans_see_wal_updates_and_never_create_source_databases() {
    let fixture = Fixture::new();
    let db = fixture.database("opencode.db");
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    v1(&db, "first", message("opencode", json!(0)));
    assert_eq!(scan_directory(&fixture.0, since()).records.len(), 1);
    let before = fs::metadata(fixture.0.join("opencode.db")).unwrap().len();
    v1(&db, "second", message("opencode", json!(0)));
    assert_eq!(
        fs::metadata(fixture.0.join("opencode.db")).unwrap().len(),
        before
    );
    assert_eq!(scan_directory(&fixture.0, since()).records.len(), 2);
    let missing = fixture.0.join("opencode-missing.db");
    assert!(read_database(&missing, since()).is_err());
    assert!(!missing.exists());
    let ro = Connection::open_with_flags(
        fixture.0.join("opencode.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert!(ro.execute("DELETE FROM message", []).is_err());
}

#[test]
fn custom_and_xdg_paths_match_openusage_precedence() {
    let home = Path::new("home");
    assert_eq!(
        data_directory(home, Some(" ~/custom "), Some("xdg")),
        home.join("custom")
    );
    assert_eq!(
        data_directory(home, Some(" "), Some("xdg")),
        PathBuf::from("xdg/opencode")
    );
    assert_eq!(
        data_directory(home, None, None),
        home.join(".local/share/opencode")
    );
}

#[test]
fn desktop_only_and_cli_only_stores_collect_usage_without_credentials_or_executables() {
    for (name, provider) in [
        ("opencode.db", "opencode-go"),
        ("opencode-local.db", "anthropic"),
    ] {
        let fixture = Fixture::new();
        let db = fixture.database(name);
        v1(&db, "usage", message(provider, json!(0.1)));
        let result = scan_directory(&fixture.0, since());
        assert!(result.available);
        assert!(result.errors.is_empty());
        assert_eq!(result.records.len(), 1);
        assert_eq!(result.records[0].event.provider, "opencode");
        assert_eq!(result.records[0].event.tokens, 195);
        assert!(!fixture.0.join("auth.json").exists());
    }
}

#[test]
fn shared_desktop_cli_history_is_unique_and_survives_local_logout() {
    let fixture = Fixture::new();
    let db = fixture.database("opencode.db");
    db.execute_batch(
        "CREATE TABLE credential(id TEXT, integration_id TEXT, active INTEGER, value TEXT);",
    )
    .unwrap();
    db.execute(
        "INSERT INTO credential VALUES ('login','opencode-go',1,?1)",
        [r#"{"type":"api","key":"SYNTHETIC_PRIVATE_KEY"}"#],
    )
    .unwrap();
    v1(&db, "desktop", message("opencode-go", json!(0.1)));
    v1(&db, "cli", message("openai", json!(0.2)));
    let channel = fixture.database("opencode-preview.db");
    v1(&channel, "desktop", message("opencode-go", json!(0.1)));
    let before = scan_directory(&fixture.0, since());
    assert_eq!(before.records.len(), 2);
    db.execute("DELETE FROM credential", []).unwrap();
    let after = scan_directory(&fixture.0, since());
    assert!(after.available);
    assert!(after.errors.is_empty());
    assert_eq!(after.records.len(), 2);
    assert_eq!(
        after
            .records
            .iter()
            .map(|record| record.event.tokens)
            .sum::<u64>(),
        390
    );
    let stored = serde_json::to_string(&after.records).unwrap();
    assert!(!stored.contains("SYNTHETIC_PRIVATE_KEY"));
    assert!(!stored.contains("PRIVATE_RESPONSE"));
    assert!(after
        .records
        .iter()
        .all(|record| record.event.provider == "opencode"));
}

#[path = "project_tests.rs"]
mod project_tests;
