use super::*;

fn record() -> scan::Record {
    scan::Record::plain(UsageEvent {
        id: "cursor-api:verified-project-migration".into(),
        provider: "cursor".into(),
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
fn old_events_decode_unassigned_and_absent_project_is_omitted() {
    let old = serde_json::json!({"id":"legacy","provider":"codex","timestamp":"2026-10-01T12:00:00Z",
        "model":"model","tokens":10,"costUSD":null});
    let decoded: UsageEvent = serde_json::from_value(old).unwrap();
    assert!(decoded.project.is_none());
    assert!(serde_json::to_value(&decoded)
        .unwrap()
        .get("project")
        .is_none());
    let mut assigned = decoded;
    assigned.project = Project::from_directory("/synthetic/work/app");
    let encoded = serde_json::to_value(&assigned).unwrap();
    assert_eq!(encoded["project"]["name"], "app");
    assert_eq!(encoded["project"]["directory"], "/synthetic/work/app");
    assert_eq!(
        serde_json::from_value::<UsageEvent>(encoded)
            .unwrap()
            .project,
        assigned.project
    );
}
#[test]
fn attribution_cache_upgrade_reparses_files_once_and_preserves_verified_cursor_history() {
    let file = serde_json::json!({"size":42,"modified":10,"records":[]});
    let api = cursor::VerifiedUsage {
        source: "cursor-account-api".into(),
        account_hash: crate::cloud::records::hash("synthetic-account"),
        fetched_at: "2026-10-01T12:00:00Z".into(),
        records: vec![record()],
    };
    let value = serde_json::json!({"version":4,"files":{"/synthetic/rollout.jsonl":file},
        "snapshot":Snapshot::default(),"cursor_api":api});
    let mut cache: Cache = serde_json::from_value(value).unwrap();
    assert_eq!(cache.files.len(), 1);
    cache.migrate();
    assert_eq!(cache.version, 5);
    assert!(cache.files.is_empty());
    assert_eq!(
        cache.cursor_api.as_ref().unwrap().records[0].event.tokens,
        10
    );
    cache.files.insert(
        PathBuf::from("/synthetic/reparsed.jsonl"),
        serde_json::from_value(file).unwrap(),
    );
    let mut reloaded: Cache =
        serde_json::from_value(serde_json::to_value(&cache).unwrap()).unwrap();
    reloaded.migrate();
    assert_eq!(reloaded.files.len(), 1);
    assert_eq!(
        reloaded.cursor_api.as_ref().unwrap().records[0].event.id,
        "cursor-api:verified-project-migration"
    );
}

#[test]
fn growing_logs_keep_project_storage_shared_after_reparse_and_cache_reload() {
    let root = std::env::temp_dir().join(format!("devclash-project-sharing-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("session.jsonl");
    let mut cache = Cache::default();
    let pricing = crate::pricing::ModelPricing::bundled();
    for count in [10, 20] {
        let rows = (0..count).map(|id| serde_json::json!({
            "type":"assistant", "timestamp":Utc::now().to_rfc3339(), "cwd":"/synthetic/workspace",
            "message":{"id":format!("message-{id}"),"model":"synthetic-model","usage":{"input_tokens":10}}
        }).to_string()).collect::<Vec<_>>().join("\n");
        std::fs::write(&path, rows).unwrap();
        let snapshot = scan::scan_roots(&mut cache.files, &pricing, vec![("claude", root.clone())], vec![], false);
        assert_eq!(snapshot.events.len(), count);
        assert_eq!(snapshot.events.iter().map(|e| e.tokens).sum::<u64>(), count as u64 * 10);
        let records = &cache.files.values().next().unwrap().records;
        let first = records[0].event.project.as_ref().unwrap();
        assert!(records.iter().all(|r| std::ptr::eq(&**first, &**r.event.project.as_ref().unwrap())));
        let encoded = serde_json::to_value(&cache).unwrap();
        cache = serde_json::from_value(encoded.clone()).unwrap();
        cache.share_projects();
        assert_eq!(serde_json::to_value(&cache).unwrap(), encoded);
        let records = &cache.files.values().next().unwrap().records;
        let first = records[0].event.project.as_ref().unwrap();
        assert!(records.iter().all(|r| std::ptr::eq(&**first, &**r.event.project.as_ref().unwrap())));
    }
    std::fs::remove_dir_all(root).unwrap();
}
