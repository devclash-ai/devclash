use super::*;

// Explicit opt-in, metadata-only local probe. Never writes the input cache or calls providers.
// Run in a separate process under /usr/bin/time -l for a comparable peak footprint.
#[test]
#[ignore = "requires DEVCLASH_MEMORY_CACHE pointing to a local metadata cache"]
fn cache_refresh_memory_probe() {
    let input = PathBuf::from(std::env::var_os("DEVCLASH_MEMORY_CACHE").unwrap());
    let legacy = std::env::var_os("DEVCLASH_MEMORY_LEGACY").is_some();
    let mut cache: Cache = if legacy {
        serde_json::from_slice(&std::fs::read(&input).unwrap()).unwrap()
    } else {
        crate::storage::read_json(&input).unwrap()
    };
    let pricing = crate::pricing::ModelPricing::bundled();
    let directory =
        std::env::temp_dir().join(format!("devclash-memory-probe-{}", std::process::id()));
    crate::storage::private_directory(&directory).unwrap();
    let output = directory.join("cache.json");
    let mut cloud_events = Vec::new();
    let started = std::time::Instant::now();
    let mut digest = None;
    let now = DateTime::parse_from_rfc3339("2100-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    for cycle in 0..3 {
        let records = cache.files.values().flat_map(|file| file.records.iter());
        let events: Vec<_> = if legacy {
            scan::dedup(records.cloned().collect())
                .iter()
                .map(|r| r.priced(&pricing))
                .collect()
        } else {
            scan::dedup_refs(records)
                .into_iter()
                .map(|r| r.priced(&pricing))
                .collect()
        };
        if legacy {
            cloud_events = events.clone();
            cache.snapshot.events = daily::totals(events);
        } else {
            cloud_events = events;
            cache.snapshot.events = daily::totals_ref(&cloud_events);
        }
        let cloud = if legacy {
            normalize(&syncable_events(&cloud_events, now), None)
        } else {
            crate::cloud::records::normalize_iter(syncable_event_refs(&cloud_events, now), None)
        };
        let serialized = serde_json::to_vec(&cloud).unwrap();
        let current = crate::cloud::records::hash(std::str::from_utf8(&serialized).unwrap());
        if let Some(previous) = &digest {
            assert_eq!(previous, &current);
        }
        digest = Some(current);
        if legacy {
            std::fs::write(&output, serde_json::to_vec(&cache).unwrap()).unwrap();
        } else {
            crate::storage::write_json(&output, &cache).unwrap();
        }
        eprintln!(
            "cycle={cycle} records={} daily_rows={} cloud_rows={}",
            cloud_events.len(),
            cache.snapshot.events.len(),
            cloud.len()
        );
    }
    eprintln!(
        "elapsed_ms={} cloud_digest={}",
        started.elapsed().as_millis(),
        digest.unwrap()
    );
    std::fs::remove_file(output).unwrap();
    std::fs::remove_dir(directory).unwrap();
    std::hint::black_box(cloud_events);
}

use super::scan::Record;

#[test]
fn borrowed_dedup_preserves_winners_and_reuses_cache_allocations() {
    let records: Vec<_> = (0..10_000)
        .map(|i| Record {
            event: UsageEvent {
                id: format!("event-{}", i % 400),
                provider: "claude".into(),
                timestamp: "2026-10-01T12:00:00Z".into(),
                model: "unpriced-model".into(),
                tokens: i + 1,
                cost_usd: None,
                unpriced_tokens: 0,
                cost_source: None,
                project: None,
            },
            sidechain: i % 3 == 0,
            message: Some(format!("message-{}", i % 200)),
            breakdown: None,
        })
        .collect();
    let expected = scan::dedup(records.clone());
    for _ in 0..3 {
        let actual = scan::dedup_refs(&records);
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        assert!(actual
            .iter()
            .all(|winner| records.iter().any(|source| std::ptr::eq(*winner, source))));
    }
}
