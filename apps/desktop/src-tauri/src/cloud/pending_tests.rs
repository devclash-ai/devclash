use super::records::*;
use crate::usage::UsageEvent;
use std::collections::HashSet;

fn event(provider: &str, id: &str, tokens: u64) -> UsageEvent {
    UsageEvent {
        id: id.into(),
        provider: provider.into(),
        timestamp: "2026-10-01T12:00:00Z".into(),
        model: "synthetic-model".into(),
        tokens,
        cost_usd: Some(0.5),
        unpriced_tokens: 0,
        cost_source: None,
        project: None,
    }
}

#[test]
fn filtering_before_allocation_matches_queue_filter_and_preserves_duplicates() {
    let mut events = vec![
        event("claude", "a", 10),
        event("codex", "b", 20),
        event("opencode", "c", 30),
        event("cursor", "cursor-api:1", 40),
        event("cursor", "cursor-api:2", 50),
        event("cursor", "cursor-csv:invalid", 100),
        event("unknown", "unknown", 100),
        event("codex", "daily:bad", 100),
    ];
    events.extend(events.clone());
    let revision = Some("2026-10-01T13:00:00Z");
    let full = normalize(&events, revision);
    for count in 0..=full.len() {
        let ack: HashSet<_> = full
            .iter()
            .take(count)
            .map(CloudRecord::acknowledgement)
            .collect();
        let expected: Vec<_> = full
            .iter()
            .filter(|r| !ack.contains(&r.acknowledgement()))
            .collect();
        let actual = normalize_pending(&events, revision, &ack);
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
}

#[test]
fn cursor_new_revision_preserves_the_complete_bucket_and_account_reset_replays_events() {
    let events = [
        event("codex", "a", 10),
        event("cursor", "cursor-api:1", 40),
        event("cursor", "cursor-api:2", 50),
    ];
    let revision = Some("2026-10-01T13:00:00Z");
    let full = normalize(&events, revision);
    let mut ack: HashSet<_> = full.iter().map(CloudRecord::acknowledgement).collect();
    assert!(normalize_pending(&events, revision, &ack).is_empty());
    let newer = normalize_pending(&events, Some("2026-10-01T14:00:00Z"), &ack);
    assert_eq!(newer.len(), 1);
    assert_eq!(newer[0].tokens, 90);
    assert_eq!(newer[0].cost_usd, Some(1.0));
    ack.clear();
    assert_eq!(
        serde_json::to_value(normalize_pending(&events, revision, &ack)).unwrap(),
        serde_json::to_value(full).unwrap()
    );
}

#[test]
#[ignore = "explicit synthetic large-history allocation benchmark"]
fn pending_allocation_probe() {
    let events: Vec<_> = (0..400_000)
        .map(|i| event("codex", &format!("synthetic-{i}"), 10))
        .collect();
    // Build acknowledgements directly, without first allocating 400k CloudRecords.
    let ack: HashSet<_> = events
        .iter()
        .take(events.len() - 20)
        .map(|e| {
            let id = hash(&format!("Codex:{}", e.id));
            hash(&format!("Codex:{id}:event"))
        })
        .collect();
    let started = std::time::Instant::now();
    let mut digest = None;
    for _ in 0..3 {
        let records = if std::env::var_os("DEVCLASH_MEMORY_LEGACY").is_some() {
            let all = normalize(&events, None);
            all.into_iter()
                .filter(|r| !ack.contains(&r.acknowledgement()))
                .collect::<Vec<_>>()
        } else {
            normalize_pending(&events, None, &ack)
        };
        assert_eq!(records.len(), 20);
        let current = hash(&serde_json::to_string(&records).unwrap());
        if let Some(previous) = &digest {
            assert_eq!(previous, &current);
        }
        digest = Some(current);
    }
    eprintln!(
        "input={} pending=20 elapsed_ms={} digest={}",
        events.len(),
        started.elapsed().as_millis(),
        digest.unwrap()
    );
}
