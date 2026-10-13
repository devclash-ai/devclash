mod account;
mod browser_login;
mod api;
pub mod commands;
mod credentials;
mod origin_migration;
mod origins;
mod projects;
pub mod records;
mod sync;
#[cfg(test)]
mod origin_migration_tests;
#[cfg(test)]
mod tests;

use api::{Api, ApiError};
use chrono::{DateTime, Utc};
use projects::{OwnedProject, ProjectSession};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::{collections::HashSet, path::PathBuf, sync::Mutex};

const SERVICE: &str = "ai.devclash.desktop.cloud";
// A release build supplies its production origin at compile time; local dev can override it.
const BUNDLED_ORIGIN: &str = match option_env!("DEVCLASH_API_URL") {
    Some(value) => value,
    None => origins::PRODUCTION_API_ORIGIN,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: String,
    pub email: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Credential {
    token: String,
    user: User,
    expires_at: String,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Preferences {
    origin: String,
    credential_origin: Option<String>,
    user_id: String,
    sync_enabled: bool,
    last_synced_at: Option<String>,
    #[serde(default)]
    acknowledged: HashSet<String>,
    pending: Vec<records::CloudRecord>,
    retry_attempts: u32,
    next_retry_at: Option<String>,
    rank_period: Period,
    session_revoked: bool,
    ranked: bool,
    public_profile: bool,
    cached_projects: Vec<OwnedProject>,
    project_sessions: Vec<ProjectSession>,
    project_warning: Option<String>,
}
// Older versions could queue manually imported Cursor files. Preserve other
// providers and verified account snapshots while removing that obsolete path.
fn allowed_record(record: &records::CloudRecord) -> bool {
    record.provider != "Cursor"
        || (record.kind == "snapshot" && record.source_id.as_deref() == Some("cursor-account-api"))
}
fn remove_imported_cursor(preferences: &mut Preferences) -> bool {
    let before = preferences.pending.len();
    preferences.pending.retain(allowed_record);
    preferences.pending.len() != before
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Period {
    #[default]
    Today,
    Week,
    Month,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub configured: bool,
    pub origin: Option<String>,
    pub app_origin: Option<String>,
    pub user: Option<User>,
    pub expires_at: Option<String>,
    pub sync_enabled: bool,
    pub last_synced_at: Option<String>,
    pub sync_error: Option<String>,
    pub ranked: bool,
    pub public_profile: bool,
    pub rank: Option<u64>,
    pub badges: u64,
    pub pending_count: u64,
    pub retry_attempts: u32,
    pub next_retry_at: Option<String>,
    pub rank_period: Period,
    pub rank_tokens: Option<u64>,
    pub rank_generated_at: Option<String>,
    pub projects: Vec<OwnedProject>,
    pub active_project_id: Option<String>,
    pub project_warning: Option<String>,
}
pub struct CloudStore {
    api: Option<Api>,
    app_origin: Result<String, String>,
    path: PathBuf,
    credential: Mutex<Option<Credential>>,
    preferences: Mutex<Preferences>,
    status: Mutex<Status>,
    syncing: Mutex<()>,
    browser_login: Mutex<Option<browser_login::BrowserLogin>>,
    keyring: Option<keyring::Entry>,
}
impl CloudStore {
    pub fn load(directory: PathBuf) -> Self {
        let origin = std::env::var("DEVCLASH_API_URL")
            .or_else(|_| std::env::var("USAGERANK_API_URL"))
            .unwrap_or_else(|_| BUNDLED_ORIGIN.into());
        let app_override = std::env::var("DEVCLASH_APP_URL")
            .ok()
            .or_else(|| option_env!("DEVCLASH_APP_URL").map(str::to_owned));
        Self::load_with_origins(directory, &origin, app_override.as_deref())
    }
    fn load_with_origins(directory: PathBuf, origin: &str, app_override: Option<&str>) -> Self {
        let path = directory.join("cloud-sync.json");
        let (mut preferences, mut load_error) = match crate::storage::read_json::<Preferences>(
            &path,
        ) {
            Ok(preferences) => (preferences, None),
            Err(_)
                if path.exists()
                    || path.with_extension("tmp").exists()
                    || path.with_extension("bak").exists() =>
            {
                eprintln!("Saved cloud state is unreadable; uploads are paused");
                (Preferences::default(), Some("Your saved cloud state could not be read. Uploads are paused. Sign in again to reconnect; your local usage is still available.".into()))
            }
            Err(_) => (Preferences::default(), None),
        };
        let api = if origin.trim().is_empty() {
            None
        } else {
            match Api::new(&origin) {
                Ok(api) => Some(api),
                Err(error) => {
                    eprintln!("Cloud configuration failed: {error}");
                    None
                }
            }
        };
        let origin_migrated = origin_migration::align_preferences(
            &mut preferences,
            api.as_ref().map(|api| api.origin.as_str()),
        );
        if load_error.is_some() {
            preferences.session_revoked = true;
        }
        if (remove_imported_cursor(&mut preferences) || origin_migrated)
            && crate::storage::write_json(&path, &preferences).is_err()
        {
            // The in-memory queue is still safe, and enqueue filters it again
            // before any request. A later sync can persist the cleaned state.
            eprintln!("Cloud state migration could not be saved");
            load_error = Some("Your cloud settings could not be updated. Unlock your app data folder and try again; your local usage is still available.".into());
        }
        let app_origin = origins::app_origin(
            api.as_ref().map_or(origin, |api| api.origin.as_str()),
            app_override,
        );
        if let Err(error) = &app_origin {
            eprintln!("Browser app configuration failed: {error}");
        }
        let status = Status {
            configured: api.is_some(),
            origin: api.as_ref().map(|api| api.origin.clone()),
            app_origin: app_origin.as_ref().ok().cloned(),
            pending_count: preferences.pending.len() as u64,
            retry_attempts: preferences.retry_attempts,
            next_retry_at: preferences.next_retry_at.clone(),
            rank_period: preferences.rank_period,
            sync_error: load_error,
            ..Status::default()
        };
        let keyring =
            api.as_ref()
                .and_then(|api| match keyring::Entry::new(SERVICE, &api.origin) {
                    Ok(entry) => Some(entry),
                    Err(_) => {
                        eprintln!("Cloud OS credential store could not be initialized");
                        None
                    }
                });
        Self {
            api,
            app_origin,
            path,
            credential: Mutex::new(None),
            preferences: Mutex::new(preferences),
            status: Mutex::new(status),
            syncing: Mutex::new(()),
            browser_login: Mutex::new(None),
            keyring,
        }
    }
    fn api(&self) -> Result<&Api, String> {
        self.api.as_ref().ok_or_else(|| "The cloud service is not configured in this build. Local tracking is still available.".into())
    }
    fn entry(&self) -> Result<&keyring::Entry, String> {
        self.api()?;
        self.keyring
            .as_ref()
            .ok_or_else(|| "Could not open the OS credential store.".into())
    }
    pub fn status(&self) -> Result<Status, String> {
        self.status
            .lock()
            .map(|status| status.clone())
            .map_err(|_| "Cloud status unavailable.".into())
    }
    pub fn sync_enabled(&self) -> Result<bool, String> {
        self.preferences
            .lock()
            .map(|preferences| preferences.sync_enabled)
            .map_err(|_| "Cloud preferences unavailable.".into())
    }
    pub fn refresh_account(&self) -> Result<Status, String> {
        let _guard = self.syncing.lock().map_err(|_| "Cloud sync unavailable.")?;
        self.refresh_account_unlocked()
    }
    fn refresh_account_unlocked(&self) -> Result<Status, String> {
        if self.api.is_none() {
            return self.status();
        }
        let credential = match self.credential() {
            Ok(credential) => credential,
            Err(error) => {
                if error != "Sign in to save your usage history." {
                    self.set_error(error)?;
                }
                return self.status();
            }
        };
        // A valid local session remains usable offline; a 401 below clears it immediately.
        {
            let preferences = self
                .preferences
                .lock()
                .map_err(|_| "Cloud preferences unavailable.")?;
            let mut status = self
                .status
                .lock()
                .map_err(|_| "Cloud status unavailable.")?;
            status.user = Some(credential.user.clone());
            status.expires_at = Some(credential.expires_at.clone());
            status.sync_enabled =
                preferences.sync_enabled && preferences.user_id == credential.user.id;
            status.last_synced_at = preferences.last_synced_at.clone();
            status.pending_count = preferences.pending.len() as u64;
            status.next_retry_at = preferences.next_retry_at.clone();
            status.retry_attempts = preferences.retry_attempts;
            status.ranked = preferences.ranked;
            status.public_profile = preferences.public_profile;
            status.projects = preferences.cached_projects.clone();
            status.active_project_id =
                projects::active_project(&preferences.project_sessions).map(str::to_owned);
            status.project_warning = preferences.project_warning.clone();
        }
        self.request("GET", "/api/auth/me", Some(&credential.token), None)?;
        let state = self.request("GET", "/api/state", Some(&credential.token), None)?;
        self.update_projects_from_state(&state)?;
        let mut preferences = self
            .preferences
            .lock()
            .map_err(|_| "Cloud preferences unavailable.")?;
        preferences.ranked = state["ranked"]
            .as_bool()
            .unwrap_or_else(|| state["rankJoinedAt"].is_string());
        preferences.public_profile = state["publicProfile"].as_bool().unwrap_or(false);
        self.save(&preferences)?;
        let mut status = self
            .status
            .lock()
            .map_err(|_| "Cloud status unavailable.")?;
        status.user = Some(credential.user.clone());
        status.expires_at = Some(credential.expires_at.clone());
        status.sync_enabled = preferences.sync_enabled && preferences.user_id == credential.user.id;
        status.last_synced_at = preferences.last_synced_at.clone();
        status.ranked = preferences.ranked;
        status.rank_period = preferences.rank_period;
        status.rank = None;
        status.rank_tokens = None;
        status.rank_generated_at = None;
        status.pending_count = preferences.pending.len() as u64;
        status.next_retry_at = preferences.next_retry_at.clone();
        status.retry_attempts = preferences.retry_attempts;
        status.public_profile = preferences.public_profile;
        status.badges = state["badges"].as_array().map_or(0, |badges| {
            badges
                .iter()
                .filter(|badge| matches!(badge["kind"].as_str(), Some("podium" | "champion")))
                .count() as u64
        });
        if preferences.retry_attempts == 0 {
            status.sync_error = None;
        }
        let period = preferences.rank_period;
        drop(status);
        drop(preferences);
        if self.status()?.ranked {
            let rank = self.request(
                "GET",
                &format!("/api/rank?period={period:?}"),
                Some(&credential.token),
                None,
            )?;
            let mut status = self
                .status
                .lock()
                .map_err(|_| "Cloud status unavailable.")?;
            status.rank = rank["own"]["rank"].as_u64();
            status.rank_tokens = rank["own"]["tokens"].as_u64();
            status.rank_generated_at = rank["generatedAt"].as_str().map(str::to_owned);
        }
        self.status()
    }
    fn request(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<&Value>,
    ) -> Result<Value, String> {
        match self.api()?.request(method, path, token, body) {
            Ok(value) => Ok(value),
            Err(ApiError {
                status, message, ..
            }) => {
                if status == 401 && token.is_some() {
                    self.forget_session()?;
                }
                self.set_error(message.clone())?;
                Err(message)
            }
        }
    }
    fn forget_session(&self) -> Result<(), String> {
        *self
            .credential
            .lock()
            .map_err(|_| "Cloud session unavailable.")? = None;
        let mut preferences = self
            .preferences
            .lock()
            .map_err(|_| "Cloud preferences unavailable.")?;
        preferences.sync_enabled = false;
        preferences.session_revoked = true;
        preferences.credential_origin = None;
        preferences.ranked = false;
        preferences.public_profile = false;
        preferences.pending.clear();
        projects::switch_project(&mut preferences.project_sessions, None, Utc::now());
        preferences.retry_attempts = 0;
        preferences.next_retry_at = None;
        self.save(&preferences)?;
        let mut status = self
            .status
            .lock()
            .map_err(|_| "Cloud status unavailable.")?;
        status.user = None;
        status.expires_at = None;
        status.sync_enabled = false;
        status.ranked = false;
        status.rank = None;
        status.rank_tokens = None;
        status.rank_generated_at = None;
        status.pending_count = 0;
        status.next_retry_at = None;
        status.retry_attempts = 0;
        status.public_profile = false;
        status.badges = 0;
        status.projects.clear();
        status.active_project_id = None;
        status.project_warning = None;
        drop(status);
        drop(preferences);
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(_) => {
                eprintln!("Cloud credential deletion failed");
                return Err("Could not remove the session from your OS credential store. Unlock it and try again.".into());
            }
        }
        Ok(())
    }
    pub fn set_error(&self, error: String) -> Result<(), String> {
        eprintln!("Cloud operation failed: {error}");
        self.status
            .lock()
            .map_err(|_| "Cloud status unavailable.")?
            .sync_error = Some(error);
        Ok(())
    }
    fn save(&self, preferences: &Preferences) -> Result<(), String> {
        std::fs::create_dir_all(
            self.path
                .parent()
                .ok_or("Invalid cloud preferences directory.")?,
        )
        .map_err(|_| "Cloud settings could not be saved.")?;
        crate::storage::write_json(&self.path, preferences)
            .map_err(|_| "Cloud settings could not be saved.".into())
    }
}

#[cfg(test)]
mod sync_checkpoint_tests;
