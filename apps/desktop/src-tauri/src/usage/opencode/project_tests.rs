use super::*;

#[test]
fn explicit_message_cwd_then_root_attribute_usage_without_loading_private_content() {
    let fixture = Fixture::new();
    let db = fixture.database("opencode.db");
    let mut cwd = message("opencode", json!(0.1));
    cwd["path"] = json!({"cwd":"/synthetic/one/app", "root":"/synthetic/other-root",
        "unrelated":"PRIVATE_PATH_METADATA"});
    v1(&db, "cwd", cwd);
    v2(
        &db,
        "root",
        "assistant",
        json!({"model":{"id":"custom-model"},"tokens":{"total":10},
        "path":{"root":"/synthetic/two/app"}, "content":"PRIVATE_V2_REPLY"}),
    );
    let mut unknown = message("opencode", json!(0.1));
    unknown["path"] = json!({"cwd":"relative", "root":null});
    v1(&db, "unknown", unknown);
    let result = scan_directory(&fixture.0, since());
    assert!(result.errors.is_empty());
    assert_eq!(result.records.len(), 3);
    let cwd = result
        .records
        .iter()
        .find(|r| r.event.id == "cwd")
        .unwrap()
        .event
        .project
        .as_ref()
        .unwrap();
    let root = result
        .records
        .iter()
        .find(|r| r.event.id == "root")
        .unwrap()
        .event
        .project
        .as_ref()
        .unwrap();
    assert_eq!(cwd.directory, "/synthetic/one/app");
    assert_eq!(root.directory, "/synthetic/two/app");
    assert_eq!(cwd.name, root.name);
    assert_ne!(cwd.id, root.id);
    assert!(result
        .records
        .iter()
        .find(|r| r.event.id == "unknown")
        .unwrap()
        .event
        .project
        .is_none());
    let stored = serde_json::to_string(&result.records).unwrap();
    for marker in [
        "PRIVATE_RESPONSE",
        "PRIVATE_V2_REPLY",
        "PRIVATE_PATH_METADATA",
        "other-root",
    ] {
        assert!(!stored.contains(marker));
    }
}

#[test]
fn session_directory_and_project_relation_are_optional_and_do_not_change_usage_totals() {
    for session_table in ["session", "session_v2"] {
        let fixture = Fixture::new();
        let db = fixture.database("opencode.db");
        let message_table = if session_table == "session" {
            "message"
        } else {
            "session_message"
        };
        db.execute_batch(&format!("ALTER TABLE {message_table} ADD COLUMN session_id TEXT;\n            CREATE TABLE {session_table}(id TEXT PRIMARY KEY, project_id TEXT, directory TEXT, title TEXT);\n            CREATE TABLE project(id TEXT PRIMARY KEY, worktree TEXT, name TEXT, commands TEXT);\n            INSERT INTO project VALUES ('project','/synthetic/project-root','PRIVATE_PROJECT_NAME','PRIVATE_COMMANDS');\n            INSERT INTO {session_table} VALUES ('known','project','/synthetic/session/app','PRIVATE_SESSION_TITLE');\n            INSERT INTO {session_table} VALUES ('fallback','project',NULL,'PRIVATE_SESSION_TITLE');")).unwrap();
        for (id, session, data) in [
            (
                "direct",
                "known",
                json!({"role":"assistant","tokens":{"total":100}, "path":{"cwd":"/synthetic/direct/app"}}),
            ),
            (
                "session",
                "known",
                json!({"role":"assistant","tokens":{"total":200}}),
            ),
            (
                "project",
                "fallback",
                json!({"role":"assistant","tokens":{"total":300}}),
            ),
            (
                "missing",
                "missing",
                json!({"role":"assistant","tokens":{"total":400}}),
            ),
        ] {
            let sql = if message_table == "message" {
                "INSERT INTO message(id,time_created,data,session_id) VALUES (?1,?2,?3,?4)"
            } else {
                "INSERT INTO session_message(id,type,time_created,data,session_id) VALUES (?1,'assistant',?2,?3,?4)"
            };
            db.execute(
                sql,
                rusqlite::params![id, CREATED, data.to_string(), session],
            )
            .unwrap();
        }
        let result = scan_directory(&fixture.0, since());
        assert!(result.errors.is_empty(), "{session_table}");
        assert_eq!(result.records.len(), 4);
        assert_eq!(
            result.records.iter().map(|r| r.event.tokens).sum::<u64>(),
            1000
        );
        let directory = |id| {
            result
                .records
                .iter()
                .find(|r| r.event.id == id)
                .unwrap()
                .event
                .project
                .as_ref()
                .map(|p| p.directory.as_str())
        };
        assert_eq!(directory("direct"), Some("/synthetic/direct/app"));
        assert_eq!(directory("session"), Some("/synthetic/session/app"));
        assert_eq!(directory("project"), Some("/synthetic/project-root"));
        assert_eq!(directory("missing"), None);
        let stored = serde_json::to_string(&result.records).unwrap();
        for private in [
            "PRIVATE_PROJECT_NAME",
            "PRIVATE_COMMANDS",
            "PRIVATE_SESSION_TITLE",
        ] {
            assert!(!stored.contains(private));
        }
    }
}

#[test]
fn legacy_usage_only_schema_stays_unassigned_and_current_v2_session_relation_works() {
    let fixture = Fixture::new();
    let db = fixture.database("opencode.db");
    v1(&db, "legacy", message("opencode", json!(0.1)));
    let before = scan_directory(&fixture.0, since());
    assert!(before.errors.is_empty());
    assert!(before.records[0].event.project.is_none());
    db.execute_batch("ALTER TABLE session_message ADD COLUMN session_id TEXT;\n        CREATE TABLE session(id TEXT PRIMARY KEY, directory TEXT);\n        INSERT INTO session VALUES ('v2','/synthetic/current-v2');").unwrap();
    db.execute("INSERT INTO session_message(id,type,time_created,data,session_id) VALUES ('current','assistant',?1,?2,'v2')",
        rusqlite::params![CREATED,json!({"tokens":{"total":10}}).to_string()]).unwrap();
    let after = scan_directory(&fixture.0, since());
    assert!(after.errors.is_empty());
    assert_eq!(after.records.len(), 2);
    let current = after
        .records
        .iter()
        .find(|r| r.event.id == "current")
        .unwrap();
    assert_eq!(
        current.event.project.as_ref().unwrap().directory,
        "/synthetic/current-v2"
    );
    assert_eq!(
        after.records.iter().map(|r| r.event.tokens).sum::<u64>(),
        before.records[0].event.tokens + 10
    );
}

#[test]
fn migrated_v2_session_tables_can_coexist_without_losing_project_metadata() {
    let fixture = Fixture::new();
    let db = fixture.database("opencode.db");
    db.execute_batch("ALTER TABLE session_message ADD COLUMN session_id TEXT;\n        CREATE TABLE session(id TEXT PRIMARY KEY, directory TEXT);\n        CREATE TABLE session_v2(id TEXT PRIMARY KEY, directory TEXT);\n        INSERT INTO session VALUES ('current','/synthetic/current');\n        INSERT INTO session_v2 VALUES ('beta','/synthetic/beta');").unwrap();
    for id in ["current", "beta"] {
        db.execute("INSERT INTO session_message(id,type,time_created,data,session_id) VALUES (?1,'assistant',?2,?3,?1)",
            rusqlite::params![id,CREATED,json!({"tokens":{"total":10}}).to_string()]).unwrap();
    }
    let result = scan_directory(&fixture.0, since());
    assert!(result.errors.is_empty());
    assert_eq!(result.records.len(), 2);
    for record in result.records {
        assert_eq!(
            record.event.project.unwrap().directory,
            format!("/synthetic/{}", record.event.id)
        );
    }
}
