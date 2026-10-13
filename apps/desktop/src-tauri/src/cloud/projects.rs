use super::records::CloudRecord;
use super::{CloudStore, Status};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OwnedProject {
    pub id: String,
    pub title: String,
    pub archived: bool,
    pub moderated: bool,
}
impl OwnedProject {
    pub fn selectable(&self) -> bool {
        !self.archived && !self.moderated
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSession {
    pub project_id: String,
    pub started_at: String,
    pub ended_at: Option<String>,
}

pub fn owned_projects(state: &Value) -> Result<Vec<OwnedProject>, String> {
    let rows = state["projects"]
        .as_array()
        .ok_or("DevClash did not return your project list.")?;
    rows.iter()
        .map(|row| {
            Ok(OwnedProject {
                id: row["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or("DevClash returned an invalid project list.")?
                    .into(),
                title: row["title"]
                    .as_str()
                    .filter(|title| !title.is_empty())
                    .ok_or("DevClash returned an invalid project list.")?
                    .into(),
                archived: row["archived"].as_bool().unwrap_or(false),
                moderated: row["moderated"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

pub fn active_project(sessions: &[ProjectSession]) -> Option<&str> {
    sessions
        .last()
        .filter(|session| session.ended_at.is_none())
        .map(|session| session.project_id.as_str())
}

pub fn switch_project(
    sessions: &mut Vec<ProjectSession>,
    selected: Option<&str>,
    at: DateTime<Utc>,
) -> bool {
    if active_project(sessions) == selected {
        return false;
    }
    let time = at.to_rfc3339_opts(SecondsFormat::Millis, true);
    if let Some(previous) = sessions
        .last_mut()
        .filter(|session| session.ended_at.is_none())
    {
        previous.ended_at = Some(time.clone());
    }
    if let Some(project_id) = selected {
        sessions.push(ProjectSession {
            project_id: project_id.into(),
            started_at: time,
            ended_at: None,
        });
    }
    true
}

pub fn attribute(records: &mut [CloudRecord], sessions: &[ProjectSession]) {
    let windows = sessions
        .iter()
        .filter_map(|session| {
            let start = DateTime::parse_from_rfc3339(&session.started_at)
                .ok()?
                .timestamp_millis();
            let end = session
                .ended_at
                .as_deref()
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|date| date.timestamp_millis());
            Some((session.project_id.as_str(), start, end))
        })
        .collect::<Vec<_>>();
    for record in records {
        record.project_id = None;
        if record.provider == "Cursor" || record.kind != "event" {
            continue;
        }
        let Some(at) = DateTime::parse_from_rfc3339(&record.occurred_at)
            .ok()
            .map(|date| date.timestamp_millis())
        else {
            continue;
        };
        record.project_id = windows
            .iter()
            .rev()
            .find(|(_, start, end)| *start <= at && end.is_none_or(|end| at < end))
            .map(|(project_id, _, _)| (*project_id).into());
    }
}

impl CloudStore {
    pub fn refresh_projects(&self) -> Result<Status, String> {
        let _guard = self.syncing.lock().map_err(|_| "Cloud sync unavailable.")?;
        let credential = self.credential()?;
        let state = match self
            .api()?
            .request("GET", "/api/state", Some(&credential.token), None)
        {
            Ok(state) => state,
            Err(error) => {
                if error.status == 401 {
                    self.forget_session()?;
                }
                return Err(error.message);
            }
        };
        self.update_projects_from_state(&state)?;
        self.status()
    }

    pub(super) fn update_projects_from_state(&self, state: &Value) -> Result<(), String> {
        let projects = owned_projects(state)?;
        let mut preferences = self
            .preferences
            .lock()
            .map_err(|_| "Cloud preferences unavailable.")?;
        preferences.cached_projects = projects;
        if let Some(active) = active_project(&preferences.project_sessions).map(str::to_owned) {
            if !preferences
                .cached_projects
                .iter()
                .any(|project| project.id == active && project.selectable())
            {
                switch_project(&mut preferences.project_sessions, None, Utc::now());
                preferences.project_warning = Some(
                    "Your active project is no longer available. New usage is unassigned; earlier queued usage will be preserved."
                        .into(),
                );
            }
        }
        self.save(&preferences)?;
        let mut status = self
            .status
            .lock()
            .map_err(|_| "Cloud status unavailable.")?;
        status.projects = preferences.cached_projects.clone();
        status.active_project_id = active_project(&preferences.project_sessions).map(str::to_owned);
        status.project_warning = preferences.project_warning.clone();
        Ok(())
    }

    pub fn set_active_project(&self, project_id: Option<String>) -> Result<Status, String> {
        let _guard = self.syncing.lock().map_err(|_| "Cloud sync unavailable.")?;
        let credential = self.credential()?;
        let mut preferences = self
            .preferences
            .lock()
            .map_err(|_| "Cloud preferences unavailable.")?;
        if preferences.user_id != credential.user.id {
            return Err("Sign in to choose a project for usage.".into());
        }
        if let Some(id) = project_id.as_deref() {
            if !preferences
                .cached_projects
                .iter()
                .any(|project| project.id == id && project.selectable())
            {
                return Err("Choose an active project from your cloud account. Refresh your projects and try again.".into());
            }
        }
        switch_project(
            &mut preferences.project_sessions,
            project_id.as_deref(),
            Utc::now(),
        );
        self.save(&preferences)?;
        let mut status = self
            .status
            .lock()
            .map_err(|_| "Cloud status unavailable.")?;
        status.active_project_id = active_project(&preferences.project_sessions).map(str::to_owned);
        Ok(status.clone())
    }

    pub fn dismiss_project_warning(&self) -> Result<Status, String> {
        let _guard = self.syncing.lock().map_err(|_| "Cloud sync unavailable.")?;
        let mut preferences = self
            .preferences
            .lock()
            .map_err(|_| "Cloud preferences unavailable.")?;
        preferences.project_warning = None;
        self.save(&preferences)?;
        let mut status = self
            .status
            .lock()
            .map_err(|_| "Cloud status unavailable.")?;
        status.project_warning = None;
        Ok(status.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }
    fn record(provider: &str, occurred_at: &str) -> CloudRecord {
        CloudRecord {
            id: "stable-event-id".into(),
            provider: provider.into(),
            model: "gpt-6-sol".into(),
            tokens: 100,
            occurred_at: occurred_at.into(),
            cost_usd: None,
            unpriced_tokens: 100,
            kind: if provider == "Cursor" {
                "snapshot"
            } else {
                "event"
            }
            .into(),
            source_id: None,
            revision_at: None,
            project_id: None,
        }
    }
    #[test]
    fn switches_assign_by_event_occurrence_not_later_sync_time() {
        let mut sessions = vec![];
        switch_project(&mut sessions, Some("project-a"), at("2026-10-04T10:00:00Z"));
        switch_project(&mut sessions, Some("project-b"), at("2026-10-04T10:30:00Z"));
        switch_project(&mut sessions, None, at("2026-10-04T11:00:00Z"));
        assert!(!switch_project(
            &mut sessions,
            None,
            at("2026-10-04T11:30:00Z")
        ));
        let mut records = vec![
            record("Codex", "2026-10-04T09:59:59Z"),
            record("Claude", "2026-10-04T10:15:00Z"),
            record("OpenCode", "2026-10-04T10:30:00Z"),
            record("Codex", "2026-10-04T11:00:00Z"),
            record("Cursor", "2026-10-04T10:45:00Z"),
        ];
        attribute(&mut records, &sessions);
        assert_eq!(
            records
                .iter()
                .map(|record| record.project_id.as_deref())
                .collect::<Vec<_>>(),
            vec![None, Some("project-a"), Some("project-b"), None, None]
        );
    }
    #[test]
    fn account_project_cache_keeps_only_minimal_owned_metadata() {
        let state = serde_json::json!({"projects":[{"id":"owned-uuid","title":"A Project",
            "archived":false,"moderated":false,"privateSecret":"never-copy"}]});
        let projects = owned_projects(&state).unwrap();
        assert_eq!(projects.len(), 1);
        assert!(projects[0].selectable());
        assert!(!serde_json::to_string(&projects)
            .unwrap()
            .contains("never-copy"));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod mac_live_qa {
    use super::*;
    use crate::usage::UsageStore;
    use chrono::Duration;
    use serde_json::json;

    #[test]
    #[ignore = "Opt-in read-only real collector attribution; use DEVCLASH_MAC_LIVE_QA=1 with quota and Cursor network disabled"]
    fn actual_mac_project_window_assigns_measured_metadata_without_upload() {
        assert_eq!(std::env::var("DEVCLASH_MAC_LIVE_QA").as_deref(), Ok("1"));
        for variable in [
            "DEVCLASH_SOURCE_SETTINGS",
            "DEVCLASH_CLAUDE_DIRS",
            "DEVCLASH_CODEX_DIRS",
            "DEVCLASH_CURSOR_DIRS",
            "DEVCLASH_OPENCODE_DIRS",
        ] {
            assert!(
                std::env::var_os(variable).is_none(),
                "Use automatic local source discovery"
            );
        }
        assert_eq!(
            std::env::var("DEVCLASH_QUOTAS_DISABLED").as_deref(),
            Ok("1")
        );
        assert_eq!(
            std::env::var("DEVCLASH_CURSOR_FETCH_DISABLED").as_deref(),
            Ok("1")
        );
        let directory = std::env::temp_dir().join(format!(
            "devclash-mac-project-window-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let native_cache = dirs::data_local_dir()
            .unwrap()
            .join("ai.devclash.desktop/usage-cache.json");
        if native_cache.is_file() {
            std::fs::copy(native_cache, directory.join("usage-cache.json")).unwrap();
        }
        let store = UsageStore::load(directory.clone());
        store
            .refresh_forced()
            .expect("Actual local metadata collection must finish");
        let mut records = store.cloud_records().unwrap();
        let end = Utc::now();
        let start = end - Duration::days(7);
        let sessions = vec![ProjectSession {
            project_id: "read-only-local-project".into(),
            started_at: start.to_rfc3339(),
            ended_at: Some(end.to_rfc3339()),
        }];
        attribute(&mut records, &sessions);
        let assigned = records
            .iter()
            .filter(|record| record.project_id.is_some())
            .collect::<Vec<_>>();
        assert!(
            !assigned.is_empty(),
            "Expected an actual measured event within the last seven days"
        );
        assert!(assigned
            .iter()
            .all(|record| record.kind == "event" && record.provider != "Cursor"));
        assert!(records
            .iter()
            .filter(|record| record.provider == "Cursor")
            .all(|record| record.project_id.is_none()));
        let provider_counts = ["Claude", "Codex", "OpenCode"]
            .into_iter()
            .map(|provider| {
                (
                    provider,
                    assigned
                        .iter()
                        .filter(|record| record.provider == provider)
                        .count(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        println!(
            "DEVCLASH_MAC_PROJECT_WINDOW_RESULT={}",
            json!({
                "assignedRecords": assigned.len(),
                "assignedTokens": assigned.iter().map(|record| record.tokens).sum::<u64>(),
                "unassignedRecords": records.len() - assigned.len(),
                "cursorAssigned": 0,
                "cloudSyncInvoked": false,
                "providerCounts": provider_counts
            })
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
