use crate::usage::UsageEvent;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRecord {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub tokens: u64,
    pub occurred_at: String,
    #[serde(rename = "costUSD", skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    pub unpriced_tokens: u64,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}
pub fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
impl CloudRecord {
    pub fn acknowledgement(&self) -> String {
        hash(&format!(
            "{}:{}:{}",
            self.provider,
            self.id,
            self.revision_at.as_deref().unwrap_or("event")
        ))
    }
}
// Called on collector records before daily::totals applies the device's calendar.
pub fn normalize(events: &[UsageEvent], cursor_revision: Option<&str>) -> Vec<CloudRecord> {
    let mut records = Vec::new();
    let mut cursor: BTreeMap<(String, String), CloudRecord> = BTreeMap::new();
    let mut seen = HashSet::new();
    let cursor_revision = cursor_revision
        .and_then(|revision| DateTime::parse_from_rfc3339(revision).ok())
        .map(|revision| {
            revision
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Millis, true)
        });
    for event in events {
        let Ok(date) = DateTime::parse_from_rfc3339(&event.timestamp) else {
            continue;
        };
        let utc = date.with_timezone(&Utc);
        let provider = match event.provider.as_str() {
            "claude" => "Claude",
            "codex" => "Codex",
            "cursor" => "Cursor",
            "opencode" => "OpenCode",
            _ => continue,
        };
        if event.id.starts_with("daily:") || event.tokens == 0 {
            continue;
        }
        if !seen.insert((event.provider.clone(), event.id.clone())) {
            continue;
        }
        let mut record = CloudRecord {
            id: hash(&format!("{provider}:{}", event.id)),
            provider: provider.into(),
            model: event.model.clone(),
            tokens: event.tokens,
            occurred_at: utc.to_rfc3339_opts(SecondsFormat::Millis, true),
            cost_usd: event.cost_usd,
            unpriced_tokens: event.unpriced_tokens,
            kind: "event".into(),
            source_id: None,
            revision_at: None,
            project_id: None,
        };
        if provider != "Cursor" {
            records.push(record);
            continue;
        }
        // Only records originating in a verified automatic API fetch can contribute.
        if !event.id.starts_with("cursor-api:") {
            continue;
        }
        let Some(revision) = cursor_revision.as_deref() else {
            continue;
        };
        let day = utc.date_naive().to_string();
        record.id = hash(&format!("cursor-account-api:{day}:{}", event.model));
        record.kind = "snapshot".into();
        record.source_id = Some("cursor-account-api".into());
        record.revision_at = Some(revision.into());
        record.occurred_at = format!("{day}T00:00:00.000Z");
        let key = (day, event.model.clone());
        if let Some(previous) = cursor.get_mut(&key) {
            previous.tokens = previous.tokens.saturating_add(record.tokens);
            previous.unpriced_tokens = previous
                .unpriced_tokens
                .saturating_add(record.unpriced_tokens);
            previous.cost_usd = match (previous.cost_usd, record.cost_usd) {
                (Some(a), Some(b)) => Some(a + b),
                (Some(a), None) | (None, Some(a)) => Some(a),
                _ => None,
            };
        } else {
            cursor.insert(key, record);
        }
    }
    records.extend(cursor.into_values());
    records.sort_by(|a, b| a.occurred_at.cmp(&b.occurred_at).then(a.id.cmp(&b.id)));
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(id: &str, provider: &str, timestamp: &str, tokens: u64) -> UsageEvent {
        UsageEvent {
            id: id.into(),
            provider: provider.into(),
            timestamp: timestamp.into(),
            model: "gpt-6-sol".into(),
            tokens,
            cost_usd: Some(0.01),
            unpriced_tokens: 0,
            cost_source: None,
            project: None,
        }
    }
    #[test]
    fn metadata_export_preserves_real_utc_occurrence_and_hides_source_identifiers() {
        let source = event(
            "session-opaque:message-opaque",
            "codex",
            "2026-09-30T23:30:00-03:00",
            100,
        );
        let records = normalize(&[source.clone()], None);
        assert_eq!(records[0].occurred_at, "2026-10-01T02:30:00.000Z");
        assert_eq!(records[0].id, normalize(&[source], None)[0].id);
        let json = serde_json::to_string(&records).unwrap();
        assert!(!json.contains("session-opaque"));
        assert!(!json.contains("message-opaque"));
        assert!(!json.contains("timestamp"));
        assert!(json.contains("costUSD"));
    }
    #[test]
    fn automatic_cursor_history_is_account_snapshots_grouped_and_deduplicated_in_utc() {
        let events = vec![
            event("cursor-api:a", "cursor", "2026-10-01T01:00:00+03:00", 100),
            event("cursor-api:b", "cursor", "2026-09-30T23:00:00Z", 50),
            event("cursor-api:a", "cursor", "2026-10-01T01:00:00+03:00", 100),
        ];
        let a = normalize(&events, Some("2026-10-01T10:00:00Z"));
        let b = normalize(&events, Some("2026-10-01T11:00:00Z"));
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].tokens, 150);
        assert_eq!(a[0].occurred_at, "2026-09-30T00:00:00.000Z");
        assert_eq!(a[0].kind, "snapshot");
        assert_eq!(a[0].source_id.as_deref(), Some("cursor-account-api"));
        assert_eq!(a[0].id, b[0].id);
        assert_ne!(a[0].acknowledgement(), b[0].acknowledgement());
        assert!(normalize(&events, None).is_empty());
        assert!(normalize(&events, Some("invalid-revision")).is_empty());
    }
    #[test]
    fn manual_cursor_imports_never_sync_even_with_a_new_api_revision() {
        let imported = event("cursor-csv:old-file", "cursor", "2026-09-30T23:00:00Z", 100);
        assert!(normalize(&[imported], Some("2026-10-01T10:00:00Z")).is_empty());
    }
    #[test]
    fn local_calendar_aggregates_are_never_uploaded_as_raw_usage() {
        let daily = event(
            "daily:codex:2026-09-30:gpt-6-sol",
            "codex",
            "2026-09-30T12:00:00+03:00",
            100,
        );
        assert!(normalize(&[daily], None).is_empty());
    }
    #[test]
    fn project_metadata_stays_local_and_does_not_change_cloud_identity_counts_or_prices() {
        let original = vec![
            event("opaque-event", "codex", "2026-09-30T12:00:00Z", 100),
            event(
                "cursor-api:opaque-event",
                "cursor",
                "2026-09-30T12:00:00Z",
                50,
            ),
        ];
        let mut assigned = original.clone();
        let project = crate::usage::Project::from_directory(
            "/synthetic/private-workspace/PRIVATE_PROJECT_LABEL",
        )
        .unwrap();
        for event in &mut assigned {
            event.project = Some(project.clone());
        }
        let without = normalize(&original, Some("2026-10-01T12:00:00Z"));
        let with = normalize(&assigned, Some("2026-10-01T12:00:00Z"));
        assert_eq!(
            serde_json::to_value(&without).unwrap(),
            serde_json::to_value(&with).unwrap()
        );
        let serialized = serde_json::to_string(&with).unwrap();
        for private in [&project.id, &project.name, &project.directory] {
            assert!(!serialized.contains(private));
        }
        for field in ["project", "directory", "name"] {
            assert!(!serialized.contains(&format!("\"{field}\"")));
        }
        assert_eq!(with.iter().map(|r| r.tokens).sum::<u64>(), 150);
    }
}
