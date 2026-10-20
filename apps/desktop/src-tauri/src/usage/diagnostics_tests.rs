use super::*;
use crate::{cloud::records::normalize, pricing::ModelPricing};
use serde_json::{json, Value};
fn range() -> DateRange {
    let today = Utc::now().date_naive().to_string();
    DateRange::parse(&today, &today).unwrap()
}
fn fixture(name: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("devclash-diagnostic-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}
fn codex_log(session: &str, time: &str, cached: u64) -> String {
    [
        json!({"type":"session_meta","payload":{"id":session,"cwd":"/PRIVATE_PROJECT","secret":"PRIVATE_CREDENTIAL"}}),
        json!({"type":"turn_context","payload":{"model":"PRIVATE_MODEL", "cwd":"/PRIVATE_PROJECT"}}),
        json!({"type":"response_item","payload":{"type":"message","content":[{"text":"PRIVATE_PROMPT"}]}}),
        json!({"timestamp":time,"type":"event_msg","payload":{"type":"token_count", "info":{
            "total_token_usage":{"input_tokens":70000,"cached_input_tokens":cached,"output_tokens":45,"total_tokens":70045},
            "last_token_usage":{"input_tokens":70000,"cached_input_tokens":cached,"output_tokens":45,"total_tokens":70045}
        }}}),
    ].into_iter().map(|row| row.to_string()).collect::<Vec<_>>().join("\n")
}
#[test]
fn copied_logs_count_once_and_near_matches_keep_session_and_bucket_evidence() {
    let root = fixture("copies");
    let date = Utc::now().date_naive();
    let a = format!("{date}T13:38:15.587Z");
    let b = format!("{date}T13:38:15.741Z");
    std::fs::write(
        root.join("PRIVATE_FILENAME.jsonl"),
        codex_log("PRIVATE_SESSION_A", &a, 60000),
    )
    .unwrap();
    std::fs::write(
        root.join("copy.jsonl"),
        codex_log("PRIVATE_SESSION_A", &a, 60000),
    )
    .unwrap();
    std::fs::write(
        root.join("second.jsonl"),
        codex_log("PRIVATE_SESSION_B", &b, 50000),
    )
    .unwrap();
    let pricing = ModelPricing::bundled();
    let mut files = HashMap::new();
    let snapshot = scan::scan_roots(
        &mut files,
        &pricing,
        vec![("codex", root.clone())],
        vec![],
        false,
    );
    let output = report(&files, &range(), &pricing, snapshot.errors.len());
    assert_eq!(output.occurrences_before_dedup, 3);
    assert_eq!(output.events_after_dedup, 2);
    assert_eq!(output.tokens_after_dedup, 140090);
    assert_eq!(output.source_file_count, 3);
    assert_ne!(output.events[0].session_hash, output.events[1].session_hash);
    assert_eq!(output.events[0].token_breakdown.unwrap().cache_read, 60000);
    assert_eq!(output.events[1].token_breakdown.unwrap().cache_read, 50000);
    assert_eq!(output.events[0].identity_counter, Some(70045));
    let cloud = normalize(&snapshot.events, None);
    assert_eq!(output.events[0].cloud_event_id, cloud[0].id);
    assert_eq!(output.events[1].cloud_event_id, cloud[1].id);
    let json = serde_json::to_string(&output).unwrap();
    assert!(!json.contains("PRIVATE_"));
    assert!(!json.contains(&*root.to_string_lossy()));
    assert_eq!(output.events[0].model_hash, hash("PRIVATE_MODEL"));
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn claude_export_omits_content_and_handles_copies_sidechains_and_cache_buckets() {
    let root = fixture("claude");
    let time = format!("{}T01:00:00Z", Utc::now().date_naive());
    let row = json!({"timestamp":time,"requestId":"PRIVATE_REQUEST","sessionId":"PRIVATE_SESSION",
        "cwd":"/PRIVATE_PROJECT","email":"PRIVATE_EMAIL", "credentials":"PRIVATE_CREDENTIAL",
        "message":{"id":"PRIVATE_MESSAGE","model":"PRIVATE_MODEL","content":"PRIVATE_REPLY",
            "usage":{"input_tokens":10,"output_tokens":2,"cache_read_input_tokens":30,"cache_creation_input_tokens":5}}});
    std::fs::write(root.join("a.jsonl"), row.to_string()).unwrap();
    std::fs::write(root.join("b.jsonl"), row.to_string()).unwrap();
    let mut sidechain = row.clone();
    sidechain["isSidechain"] = json!(true);
    sidechain["requestId"] = json!("PRIVATE_SIDECHAIN_REQUEST");
    std::fs::write(root.join("c.jsonl"), sidechain.to_string()).unwrap();
    let pricing = ModelPricing::bundled();
    let mut files = HashMap::new();
    scan::scan_roots(
        &mut files,
        &pricing,
        vec![("claude", root.clone())],
        vec![],
        false,
    );
    let report = report(&files, &range(), &pricing, 1);
    assert_eq!(report.occurrences_before_dedup, 3);
    assert_eq!(report.events_after_dedup, 1);
    assert_eq!(report.tokens_after_dedup, 47);
    assert_eq!(report.scan_warning_count, 1);
    assert_eq!(report.events[0].token_breakdown.unwrap().cache_write_5m, 5);
    let json = serde_json::to_string(&report).unwrap();
    for private in ["PRIVATE_", "credentials", "project"] {
        assert!(!json.contains(private));
    }
    assert!(report.events[0].message_hash.is_some());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn utc_range_is_inclusive_validated_and_free_form_timestamps_cannot_escape() {
    let range = DateRange::parse("2026-10-07", "2026-10-07").unwrap();
    assert_eq!(
        range.timestamp("2026-10-08T01:00:00+03:00").as_deref(),
        Some("2026-10-07T22:00:00.000Z")
    );
    assert!(range.timestamp("2026-10-08T00:00:00Z").is_none());
    assert!(range.timestamp("PRIVATE_SECRET").is_none());
    for (from, through) in [
        ("2026-10-08", "2026-10-07"),
        ("2026-01-01", "2026-10-07"),
        ("PRIVATE", "2026-10-07"),
        ("2026-1-1", "2026-10-07"),
        ("2999-01-01", "2999-01-01"),
    ] {
        assert!(DateRange::parse(from, through).is_err());
    }
}
#[test]
fn saved_export_is_private_and_empty_or_partial_is_explicit() {
    let root = fixture("save");
    let report = report(&HashMap::new(), &range(), &ModelPricing::bundled(), 2);
    let name = save(&report, &root).unwrap();
    let path = root.join(&name);
    let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(value["eventsAfterDedup"], 0);
    assert_eq!(value["scanWarningCount"], 2);
    assert_eq!(value["occurrences"], json!([]));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(save(&report, &root.join("missing-folder")).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn export_preserves_recorded_cost_precedence_before_estimating_copies() {
    let root = fixture("recorded-cost");
    let time = format!("{}T01:00:00Z", Utc::now().date_naive());
    let mut row = json!({"timestamp":time,"requestId":"request", "message":{
        "id":"same-message","model":"claude-opus-4-1","usage":{"input_tokens":1000,"output_tokens":20}}});
    std::fs::write(root.join("a-estimated.jsonl"), row.to_string()).unwrap();
    row["costUSD"] = json!(0.000123);
    std::fs::write(root.join("b-recorded.jsonl"), row.to_string()).unwrap();
    let pricing = ModelPricing::bundled();
    let mut files = HashMap::new();
    let snapshot = scan::scan_roots(
        &mut files,
        &pricing,
        vec![("claude", root.clone())],
        vec![],
        false,
    );
    let output = report(&files, &range(), &pricing, 0);
    assert_eq!(output.events.len(), 1);
    assert_eq!(output.events[0].cost_usd, Some(0.000123));
    assert_eq!(output.events[0].cost_usd, snapshot.events[0].cost_usd);
    std::fs::remove_dir_all(root).unwrap();
}
