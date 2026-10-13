use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "devclash-go-auth-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn db(&self, name: &str) -> Connection {
        let db = Connection::open(self.0.join(name)).unwrap();
        db.execute_batch("CREATE TABLE credential(id TEXT, integration_id TEXT, active INTEGER, time_created INTEGER, time_updated INTEGER, value TEXT);
            CREATE TABLE migration(id TEXT PRIMARY KEY, time_completed INTEGER);").unwrap();
        db
    }
    fn legacy(&self) {
        fs::write(self.0.join("auth.json"), r#"{"opencode-go":{"type":"api","key":"synthetic-legacy"},"other":{"access":"private-unused-oauth"}}"#).unwrap();
    }
    fn migrate(db: &Connection) {
        db.execute("INSERT INTO migration VALUES (?1,1)", [IMPORT_MIGRATION])
            .unwrap();
    }
    fn row(
        db: &Connection,
        id: &str,
        provider: &str,
        active: Option<i64>,
        created: i64,
        updated: i64,
        value: &str,
    ) {
        db.execute(
            "INSERT INTO credential VALUES (?1,?2,?3,?4,?5,?6)",
            rusqlite::params![id, provider, active, created, updated, value],
        )
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn legacy_login_survives_an_empty_unmigrated_experimental_database() {
    let fixture = Fixture::new();
    fixture.legacy();
    let db = fixture.db("opencode.db");
    db.execute(
        "INSERT INTO migration VALUES ('20260611035744_credential',1)",
        [],
    )
    .unwrap();
    fixture.db("opencode-local.db");
    assert_eq!(load(&fixture.0).unwrap().unwrap().token, "synthetic-legacy");
    fs::write(fixture.0.join("auth.json"), "{}").unwrap();
    assert!(load(&fixture.0).unwrap().is_none());
}

#[test]
fn completed_migration_remains_authoritative_after_final_logout() {
    let fixture = Fixture::new();
    fixture.legacy();
    let db = fixture.db("opencode.db");
    Fixture::migrate(&db);
    Fixture::row(
        &db,
        "current",
        "opencode-go",
        Some(1),
        1,
        1,
        r#"{"type":"key","key":"synthetic-current"}"#,
    );
    assert_eq!(
        load(&fixture.0).unwrap().unwrap().token,
        "synthetic-current"
    );
    db.execute("DELETE FROM credential", []).unwrap();
    assert!(load(&fixture.0).unwrap().is_none());
    assert!(load(&fixture.0).unwrap().is_none()); // persisted state, not a process-local latch
    assert!(fixture.0.join("auth.json").exists());
}

#[test]
fn inactive_go_remains_usable_and_other_provider_rows_never_restore_legacy_json() {
    for provider in ["opencode-go", "openai"] {
        let fixture = Fixture::new();
        fixture.legacy();
        let db = fixture.db("opencode.db");
        Fixture::row(
            &db,
            "saved",
            provider,
            Some(0),
            1,
            1,
            r#"{"type":"key","key":"synthetic-inactive"}"#,
        );
        let login = load(&fixture.0).unwrap();
        if provider == "opencode-go" {
            assert_eq!(login.unwrap().token, "synthetic-inactive");
        } else {
            assert!(login.is_none());
        }
    }
}

#[test]
fn selection_matches_active_then_creation_not_last_refresh_and_rejects_invalid_current() {
    let fixture = Fixture::new();
    fixture.legacy();
    let db = fixture.db("opencode.db");
    Fixture::row(
        &db,
        "old-refreshed",
        "opencode-go",
        None,
        1,
        999,
        r#"{"type":"key","key":"synthetic-old"}"#,
    );
    Fixture::row(
        &db,
        "new",
        "opencode-go",
        None,
        2,
        2,
        r#"{"type":"key","key":"synthetic-new"}"#,
    );
    assert_eq!(load(&fixture.0).unwrap().unwrap().token, "synthetic-new");
    Fixture::row(
        &db,
        "selected",
        "opencode-go",
        Some(1),
        0,
        0,
        r#"{"type":"key","key":"synthetic-selected"}"#,
    );
    assert_eq!(
        load(&fixture.0).unwrap().unwrap().token,
        "synthetic-selected"
    );
    db.execute(
        "UPDATE credential SET value='{broken' WHERE id='selected'",
        [],
    )
    .unwrap();
    assert!(load(&fixture.0).is_err());
}

#[test]
fn schema_without_active_column_uses_newest_credential() {
    let fixture = Fixture::new();
    let db = Connection::open(fixture.0.join("opencode.db")).unwrap();
    db.execute_batch("CREATE TABLE credential(id TEXT,integration_id TEXT,time_created INTEGER,value TEXT);
        INSERT INTO credential VALUES ('old','opencode-go',1,'{\"type\":\"key\",\"key\":\"synthetic-old\"}');
        INSERT INTO credential VALUES ('new','opencode-go',2,'{\"type\":\"key\",\"key\":\"synthetic-new\"}');").unwrap();
    assert_eq!(load(&fixture.0).unwrap().unwrap().token, "synthetic-new");
}

#[test]
fn independent_channels_are_deterministic_and_logout_never_restores_json() {
    let fixture = Fixture::new();
    fixture.legacy();
    let stable = fixture.db("opencode.db");
    Fixture::migrate(&stable);
    let preview = fixture.db("opencode-preview.db");
    Fixture::migrate(&preview);
    Fixture::row(
        &stable,
        "stable",
        "opencode-go",
        Some(1),
        1,
        1,
        r#"{"type":"key","key":"synthetic-stable"}"#,
    );
    Fixture::row(
        &preview,
        "preview",
        "opencode-go",
        Some(1),
        2,
        2,
        r#"{"type":"key","key":"synthetic-preview"}"#,
    );
    assert_eq!(
        load(&fixture.0).unwrap().unwrap().token,
        "synthetic-preview"
    );
    preview.execute("DELETE FROM credential", []).unwrap();
    assert_eq!(load(&fixture.0).unwrap().unwrap().token, "synthetic-stable");
    stable.execute("DELETE FROM credential", []).unwrap();
    assert!(load(&fixture.0).unwrap().is_none());
}

#[test]
fn malformed_oversized_wrong_type_and_nonstring_keys_fail_without_exposing_material() {
    for value in [
        r#"{"type":"oauth","key":"PRIVATE_SENTINEL"}"#.into(),
        r#"{"type":"key","key":123}"#.into(),
        format!(r#"{{"type":"key","key":"{}"}}"#, "x".repeat(MAX_KEY + 1)),
        r#"{"type":"key","key":"PRIVATE_SENTINEL\ninvalid"}"#.into(),
    ] {
        let fixture = Fixture::new();
        fixture.legacy();
        let db = fixture.db("opencode.db");
        Fixture::row(&db, "selected", "opencode-go", Some(1), 1, 1, &value);
        let reason = load(&fixture.0).err().unwrap();
        assert!(!reason.contains("PRIVATE_SENTINEL"));
    }
    let fixture = Fixture::new();
    fs::write(fixture.0.join("auth.json"), vec![b'x'; MAX_JSON + 1]).unwrap();
    assert!(load(&fixture.0).err().unwrap().contains("too large"));
    fs::write(fixture.0.join("auth.json"), "{broken").unwrap();
    assert!(load(&fixture.0).is_err());
}

#[test]
fn missing_roots_never_create_login_files_and_wal_updates_remain_visible() {
    let fixture = Fixture::new();
    assert!(load(&fixture.0.join("missing")).unwrap().is_none());
    assert!(!fixture.0.join("missing").exists());
    let db = fixture.db("opencode.db");
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    Fixture::migrate(&db);
    assert!(load(&fixture.0).unwrap().is_none());
    Fixture::row(
        &db,
        "current",
        "opencode-go",
        Some(1),
        1,
        1,
        r#"{"type":"key","key":"synthetic-wal"}"#,
    );
    assert_eq!(load(&fixture.0).unwrap().unwrap().token, "synthetic-wal");
    db.execute("DELETE FROM credential", []).unwrap();
    assert!(load(&fixture.0).unwrap().is_none());
}
