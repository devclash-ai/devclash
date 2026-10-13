//! Durable metadata queue with bounded retries. Explicit Sync Now bypasses its timer.
use super::records::CloudRecord;
use super::{projects, ApiError, CloudStore, Period, Preferences, Status};
use chrono::{DateTime, Utc};
use serde_json::json;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const BATCH_SIZE: usize = 900;
const CHECKPOINT_BATCHES: usize = 10;
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(15);

fn acknowledge_batch(preferences: &mut Preferences, batch: &[CloudRecord]) {
    preferences.acknowledged.extend(batch.iter().map(CloudRecord::acknowledgement));
    // The syncing lock owns this queue for the cycle; the sent batch is its prefix.
    // Avoid rehashing every remaining record and cloning all historical acknowledgements.
    preferences.pending.drain(..batch.len());
    preferences.retry_attempts = 0;
    preferences.next_retry_at = None;
}

fn enqueue(preferences: &mut Preferences, records: Vec<CloudRecord>) {
    super::remove_imported_cursor(preferences);
    // Cursor is a complete account snapshot, unlike append-only local logs.
    // Rebuild its pending portion from the current authenticated collector so
    // sign-out/expiry/account changes cannot upload an older account's queue.
    preferences
        .pending
        .retain(|record| record.provider != "Cursor");
    let mut pending = preferences
        .pending
        .drain(..)
        .map(|record| (format!("{}:{}", record.provider, record.id), record))
        .collect::<BTreeMap<_, _>>();
    for record in records {
        if !super::allowed_record(&record) {
            continue;
        }
        if preferences.acknowledged.contains(&record.acknowledgement()) {
            continue;
        }
        let key = format!("{}:{}", record.provider, record.id);
        match pending.get(&key) {
            // Immutable events retain the first complete reading until acknowledged.
            Some(old) if old.kind != "snapshot" => {}
            Some(old) if old.revision_at >= record.revision_at => {}
            _ => {
                pending.insert(key, record);
            }
        }
    }
    preferences.pending = pending.into_values().collect();
}
fn retry_delay(attempts: u32) -> i64 {
    30i64
        .saturating_mul(1i64 << attempts.saturating_sub(1).min(7))
        .min(3600)
}
impl CloudStore {
    pub fn sync(&self, records: Vec<CloudRecord>) -> Result<Status, String> {
        self.sync_queue(records, true, &|_| {})
    }
    pub fn sync_automatic(&self, records: Vec<CloudRecord>) -> Result<Status, String> {
        self.sync_queue(records, false, &|_| {})
    }
    pub fn sync_automatic_with_progress(&self, records: Vec<CloudRecord>, progress: impl Fn(Status)) -> Result<Status, String> {
        self.sync_queue(records, false, &progress)
    }
    fn sync_queue(&self, mut records: Vec<CloudRecord>, force: bool, progress: &dyn Fn(Status)) -> Result<Status, String> {
        let Ok(_guard) = self.syncing.try_lock() else {
            return self.status();
        };
        {
            let mut preferences = self
                .preferences
                .lock()
                .map_err(|_| "Cloud preferences unavailable.")?;
            if !preferences.sync_enabled {
                return self.status();
            }
            projects::attribute(&mut records, &preferences.project_sessions);
            enqueue(&mut preferences, records);
            self.save(&preferences)?;
            self.queue_status(&preferences)?;
            if !force
                && preferences
                    .next_retry_at
                    .as_deref()
                    .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
                    .is_some_and(|date| date > Utc::now())
            {
                return self.status();
            }
        }
        let credential = self.credential()?;
        let mut checkpointed_at = Instant::now();
        let mut uncheckpointed_batches = 0;
        loop {
            let batch = {
                let preferences = self
                    .preferences
                    .lock()
                    .map_err(|_| "Cloud preferences unavailable.")?;
                if !preferences.sync_enabled || preferences.user_id != credential.user.id {
                    return self.status();
                }
                preferences
                    .pending
                    .iter()
                    .take(BATCH_SIZE)
                    .cloned()
                    .collect::<Vec<_>>()
            };
            if batch.is_empty() {
                break;
            }
            let response = self.api().map_err(ApiError::from).and_then(|api| {
                api.request(
                    "POST",
                    "/api/usage/sync",
                    Some(&credential.token),
                    Some(&json!({"records":batch})),
                )
            });
            let result = match response {
                Ok(reply) => {
                    let processed = reply["accepted"].as_u64().unwrap_or(0)
                        + reply["duplicates"].as_u64().unwrap_or(0);
                    if processed == batch.len() as u64 {
                        Ok(())
                    } else {
                        Err("Some usage records were not accepted. Try Sync Now again.".into())
                    }
                }
                Err(error) => {
                    if error.status == 409
                        && error.code.as_deref() == Some("USAGE_PROJECT_UNAVAILABLE")
                        && error.project_id.as_deref().is_some_and(|id| {
                            batch
                                .iter()
                                .any(|record| record.project_id.as_deref() == Some(id))
                        })
                    {
                        let project_id = error.project_id.as_deref().unwrap();
                        self.unassign_unavailable_project(project_id)?;
                        continue;
                    }
                    if error.status == 401 {
                        self.forget_session()?;
                    }
                    Err(error.message)
                }
            };
            if let Err(error) = result {
                let mut preferences = self
                    .preferences
                    .lock()
                    .map_err(|_| "Cloud preferences unavailable.")?;
                // A revoked login has already removed the session and disabled its queue.
                if preferences.sync_enabled && preferences.user_id == credential.user.id {
                    preferences.retry_attempts = preferences.retry_attempts.saturating_add(1);
                    preferences.next_retry_at = Some(
                        (Utc::now()
                            + chrono::Duration::seconds(retry_delay(preferences.retry_attempts)))
                        .to_rfc3339(),
                    );
                    self.save(&preferences)?;
                }
                self.queue_status(&preferences)?;
                self.set_error(error.clone())?;
                return Err(error);
            }
            let mut preferences = self
                .preferences
                .lock()
                .map_err(|_| "Cloud preferences unavailable.")?;
            acknowledge_batch(&mut preferences, &batch);
            uncheckpointed_batches += 1;
            // The complete queue was persisted before sending. A crash before this
            // checkpoint only replays acknowledged records; server deduplication
            // prevents double counting. Save by ten batches, or the next success
            // after 15 seconds; a slow request can extend that time interval.
            if uncheckpointed_batches >= CHECKPOINT_BATCHES
                || checkpointed_at.elapsed() >= CHECKPOINT_INTERVAL
            {
                self.save(&preferences)?;
                uncheckpointed_batches = 0;
                checkpointed_at = Instant::now();
            }
            self.queue_status(&preferences)?;
            drop(preferences);
            self.status.lock().map_err(|_| "Cloud status unavailable.")?.sync_error = None;
            progress(self.status()?);
        }
        let mut preferences = self
            .preferences
            .lock()
            .map_err(|_| "Cloud preferences unavailable.")?;
        preferences.last_synced_at = Some(Utc::now().to_rfc3339());
        preferences.retry_attempts = 0;
        preferences.next_retry_at = None;
        self.save(&preferences)?;
        self.queue_status(&preferences)?;
        let mut status = self
            .status
            .lock()
            .map_err(|_| "Cloud status unavailable.")?;
        status.last_synced_at = preferences.last_synced_at.clone();
        status.sync_error = None;
        drop(status);
        drop(preferences);
        self.status()
    }
    fn queue_status(&self, preferences: &Preferences) -> Result<(), String> {
        let mut status = self
            .status
            .lock()
            .map_err(|_| "Cloud status unavailable.")?;
        status.pending_count = preferences.pending.len() as u64;
        status.retry_attempts = preferences.retry_attempts;
        status.next_retry_at = preferences.next_retry_at.clone();
        Ok(())
    }
    fn unassign_unavailable_project(&self, project_id: &str) -> Result<(), String> {
        let mut preferences = self
            .preferences
            .lock()
            .map_err(|_| "Cloud preferences unavailable.")?;
        let affected = preferences
            .pending
            .iter_mut()
            .filter(|record| record.project_id.as_deref() == Some(project_id))
            .map(|record| {
                record.project_id = None;
                1usize
            })
            .sum::<usize>();
        if projects::active_project(&preferences.project_sessions) == Some(project_id) {
            projects::switch_project(&mut preferences.project_sessions, None, Utc::now());
        }
        preferences
            .cached_projects
            .retain(|project| project.id != project_id);
        preferences.project_warning = Some(format!(
            "A selected project became unavailable. {affected} queued usage {} will sync as unassigned; no measured usage was dropped.",
            if affected == 1 { "record" } else { "records" }
        ));
        self.save(&preferences)?;
        self.queue_status(&preferences)?;
        let mut status = self
            .status
            .lock()
            .map_err(|_| "Cloud status unavailable.")?;
        status.projects = preferences.cached_projects.clone();
        status.active_project_id =
            projects::active_project(&preferences.project_sessions).map(str::to_owned);
        status.project_warning = preferences.project_warning.clone();
        Ok(())
    }
    pub fn set_rank_period(&self, period: &str) -> Result<Status, String> {
        let _guard = self.syncing.lock().map_err(|_| "Cloud sync unavailable.")?;
        let period = match period {
            "Today" => Period::Today,
            "Week" => Period::Week,
            "Month" => Period::Month,
            _ => return Err("Choose Today, Week or Month for Rank Battle.".into()),
        };
        let mut preferences = self
            .preferences
            .lock()
            .map_err(|_| "Cloud preferences unavailable.")?;
        preferences.rank_period = period;
        self.save(&preferences)?;
        drop(preferences);
        {
            let mut status = self
                .status
                .lock()
                .map_err(|_| "Cloud status unavailable.")?;
            status.rank_period = period;
            status.rank = None;
            status.rank_tokens = None;
            status.rank_generated_at = None;
        }
        self.refresh_account_unlocked()
    }
}
#[cfg(test)]
mod tests {
    use super::super::records;
    use super::*;
    fn event(id: &str, tokens: u64) -> CloudRecord {
        records::normalize(
            &[crate::usage::UsageEvent {
                id: id.into(),
                provider: "codex".into(),
                timestamp: Utc::now().to_rfc3339(),
                model: "gpt-6-sol".into(),
                tokens,
                cost_usd: None,
                unpriced_tokens: tokens,
                cost_source: None,
                project: None,
            }],
            None,
        )
        .remove(0)
    }
    #[test]
    fn pending_events_survive_a_rotated_log_and_recover_without_credentials_on_disk() {
        let mut preferences = Preferences {
            user_id: "account-one".into(),
            sync_enabled: true,
            ..Preferences::default()
        };
        enqueue(&mut preferences, vec![event("first", 100)]);
        enqueue(&mut preferences, vec![event("second", 200)]);
        assert_eq!(preferences.pending.len(), 2);
        let encoded = serde_json::to_string(&preferences).unwrap();
        let restored: Preferences = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored.pending.len(), 2);
        assert!(!encoded.contains("first"));
        assert!(!encoded.contains("token\":"));
        enqueue(&mut preferences, vec![event("first", 300)]);
        assert_eq!(
            preferences
                .pending
                .iter()
                .find(|record| record.id == event("first", 1).id)
                .unwrap()
                .tokens,
            100
        );
    }
    #[test]
    fn retry_timer_is_bounded_and_snapshot_updates_replace_pending_revisions() {
        assert_eq!(retry_delay(1), 30);
        assert_eq!(retry_delay(2), 60);
        assert_eq!(retry_delay(100), 3600);
        let mut preferences = Preferences::default();
        let mut old = event("snapshot", 100);
        old.kind = "snapshot".into();
        old.revision_at = Some("2026-10-01T10:00:00Z".into());
        let mut new = old.clone();
        new.tokens = 200;
        new.revision_at = Some("2026-10-01T11:00:00Z".into());
        enqueue(&mut preferences, vec![old]);
        enqueue(&mut preferences, vec![new]);
        assert_eq!(preferences.pending.len(), 1);
        assert_eq!(preferences.pending[0].tokens, 200);
    }
    #[test]
    fn obsolete_cursor_imports_are_removed_from_persisted_and_new_uploads() {
        let mut imported = event("legacy-cursor", 900);
        imported.provider = "Cursor".into();
        imported.kind = "snapshot".into();
        imported.source_id = Some("cursor-account-export".into());
        let mut verified = imported.clone();
        verified.id = "verified-account-day-model".into();
        verified.source_id = Some("cursor-account-api".into());
        let mut forged_event = imported.clone();
        forged_event.id = "cursor-event".into();
        forged_event.kind = "event".into();
        forged_event.source_id = Some("cursor-account-api".into());
        let codex = event("codex", 100);
        let mut preferences = Preferences {
            user_id: "existing-account".into(),
            sync_enabled: true,
            pending: vec![imported.clone(), codex.clone()],
            ..Preferences::default()
        };
        enqueue(&mut preferences, vec![imported, forged_event, verified]);
        assert_eq!(preferences.pending.len(), 2);
        assert!(preferences
            .pending
            .iter()
            .any(|record| record.id == codex.id));
        assert!(preferences.pending.iter().all(super::super::allowed_record));
        assert_eq!(preferences.user_id, "existing-account");
        assert!(preferences.sync_enabled);
    }
    #[test]
    fn cursor_pending_snapshots_follow_current_authentication_without_losing_local_events() {
        let mut cursor = event("cursor-current-account", 100);
        cursor.provider = "Cursor".into();
        cursor.kind = "snapshot".into();
        cursor.source_id = Some("cursor-account-api".into());
        cursor.revision_at = Some("2026-10-01T12:00:00Z".into());
        let codex = event("offline-local-codex", 200);
        let mut preferences = Preferences::default();
        enqueue(&mut preferences, vec![cursor.clone(), codex.clone()]);
        assert_eq!(preferences.pending.len(), 2);
        // No currently authenticated Cursor source is eligible after expiry.
        enqueue(&mut preferences, vec![]);
        assert_eq!(preferences.pending.len(), 1);
        assert_eq!(preferences.pending[0].id, codex.id);
        // A new account's current complete snapshot may be queued afterward.
        cursor.id = "cursor-new-account".into();
        enqueue(&mut preferences, vec![cursor]);
        assert_eq!(preferences.pending.len(), 2);
        assert!(preferences
            .pending
            .iter()
            .any(|record| record.id == "cursor-new-account"));
    }
    #[test]
    #[ignore = "Opt-in synthetic disk/CPU benchmark; not part of routine regression tests"]
    fn large_queue_checkpoint_benchmark() {
        let root = std::env::temp_dir().join(format!("devclash-sync-bench-{}", Utc::now().timestamp_nanos_opt().unwrap()));
        std::fs::create_dir_all(&root).unwrap();
        let make = || Preferences {
            pending: (0..120_000).map(|index| CloudRecord {
                id: records::hash(&format!("synthetic-{index}")), provider: "Codex".into(), model: "synthetic-model".into(),
                tokens: 100, occurred_at: "2026-10-08T00:00:00Z".into(), cost_usd: None,
                unpriced_tokens: 100, kind: "event".into(), source_id: None, revision_at: None, project_id: None,
            }).collect(),
            acknowledged: (0..200_000).map(|index| records::hash(&format!("historical-{index}"))).collect(),
            ..Preferences::default()
        };
        let mut old = make(); let mut new = make();
        let initial_bytes = serde_json::to_vec(&old).unwrap().len();
        let started = Instant::now();
        for _ in 0..10 {
            let batch = old.pending.iter().take(BATCH_SIZE).cloned().collect::<Vec<_>>();
            old.acknowledged.extend(batch.iter().map(CloudRecord::acknowledgement));
            let acknowledged = old.acknowledged.clone();
            old.pending.retain(|record| !acknowledged.contains(&record.acknowledgement()));
            crate::storage::write_json(&root.join("old.json"), &old).unwrap();
        }
        let old_ms = started.elapsed().as_millis(); let started = Instant::now();
        for _ in 0..10 {
            let batch = new.pending.iter().take(BATCH_SIZE).cloned().collect::<Vec<_>>();
            acknowledge_batch(&mut new, &batch);
        }
        crate::storage::write_json(&root.join("new.json"), &new).unwrap();
        let new_ms = started.elapsed().as_millis();
        assert_eq!(old.acknowledged, new.acknowledged);
        assert_eq!(old.pending.len(),new.pending.len());
        assert!(old.pending.iter().zip(&new.pending).all(|(a,b)| a.id==b.id));
        println!("CHECKPOINT_BENCH {{\"records\":120000,\"previousAcknowledgements\":200000,\"batchSize\":900,\"batches\":10,\"initialBytes\":{initial_bytes},\"oldMs\":{old_ms},\"newMs\":{new_ms},\"oldWrites\":10,\"newWrites\":1}}");
        std::fs::remove_dir_all(root).unwrap();
    }

}
