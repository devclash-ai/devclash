use super::{CloudStore, Credential, Preferences, Status};
use serde_json::{json, Value};
impl CloudStore {
    pub fn request_code(&self, email: String) -> Result<Value, String> {
        if email.len() > 320 {
            return Err("Enter a valid email address.".into());
        }
        self.request(
            "POST",
            "/api/auth/request-code",
            None,
            Some(&json!({"email": email.trim()})),
        )
    }
    pub fn verify_code(&self, challenge_id: String, code: String) -> Result<Status, String> {
        let _guard = self.syncing.lock().map_err(|_| "Cloud sync unavailable.")?;
        if challenge_id.len() > 100
            || code.len() != 6
            || !code.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err("Enter the six-digit code from your email.".into());
        }
        let reply = self.request("POST", "/api/auth/verify-code", None, Some(&json!({
            "challengeId": challenge_id, "code": code, "client": "desktop", "deviceName": format!("DevClash on {}", std::env::consts::OS)
        })))?;
        self.accept_session(reply)
    }
    pub(super) fn accept_session(&self, reply: Value) -> Result<Status, String> {
        let credential = Credential {
            token: reply["token"]
                .as_str()
                .filter(|token| !token.is_empty())
                .ok_or("DevClash did not return a desktop session.")?
                .into(),
            user: serde_json::from_value(reply["user"].clone())
                .map_err(|_| "DevClash returned an invalid account.")?,
            expires_at: reply["expiresAt"]
                .as_str()
                .ok_or("DevClash returned an invalid session expiry.")?
                .into(),
        };
        let encoded =
            serde_json::to_string(&credential).map_err(|_| "Could not save your cloud session.")?;
        if self.entry()?.set_password(&encoded).is_err() {
            eprintln!("Cloud credential save failed");
            // The token never reaches the webview, even when the native secure store is unavailable.
            let _ = self.request("POST", "/api/auth/logout", Some(&credential.token), None);
            return Err(
                "Your OS credential store could not save the session. Unlock it and try again."
                    .into(),
            );
        }
        let mut preferences = self
            .preferences
            .lock()
            .map_err(|_| "Cloud preferences unavailable.")?;
        if preferences.user_id != credential.user.id || preferences.origin != self.api()?.origin {
            *preferences = Preferences {
                origin: self.api()?.origin.clone(),
                user_id: credential.user.id.clone(),
                join_pending: preferences.join_pending,
                enable_sync_on_join: preferences.enable_sync_on_join,
                join_receipt: preferences.join_receipt.clone(),
                join_account_id: preferences.join_account_id.clone(),
                ..Preferences::default()
            };
        }
        if preferences.join_pending && preferences.join_account_id.is_none() {
            preferences.join_account_id = Some(credential.user.id.clone());
        }
        preferences.session_revoked = false;
        preferences.credential_origin = None;
        self.save(&preferences)?;
        *self
            .credential
            .lock()
            .map_err(|_| "Cloud session unavailable.")? = Some(credential);
        drop(preferences);
        self.refresh_account_unlocked()
    }
    pub fn set_sync(&self, enabled: bool) -> Result<Status, String> {
        let _guard = self.syncing.lock().map_err(|_| "Cloud sync unavailable.")?;
        let credential = self.credential()?;
        let mut preferences = self
            .preferences
            .lock()
            .map_err(|_| "Cloud preferences unavailable.")?;
        preferences.sync_enabled = enabled;
        if !enabled {
            preferences.enable_sync_on_join = false;
            preferences.join_sync_activation_ready = false;
        }
        preferences.user_id = credential.user.id;
        preferences.origin = self.api()?.origin.clone();
        self.save(&preferences)?;
        self.status
            .lock()
            .map_err(|_| "Cloud status unavailable.")?
            .sync_enabled = enabled;
        self.status()
    }
    pub fn set_participation(&self, ranked: bool, public_profile: bool) -> Result<Status, String> {
        let credential = self.credential()?;
        self.request(
            "PATCH",
            "/api/participation",
            Some(&credential.token),
            Some(&json!({"ranked": ranked, "publicProfile": public_profile})),
        )?;
        self.refresh_account()
    }
    pub fn logout(&self) -> Result<Status, String> {
        let _guard = self.syncing.lock().map_err(|_| "Cloud sync unavailable.")?;
        let mut preferences = self
            .preferences
            .lock()
            .map_err(|_| "Cloud preferences unavailable.")?;
        preferences.sync_enabled = false;
        self.save(&preferences)?;
        drop(preferences);
        let result = self.credential().ok().map(|credential| {
            self.request("POST", "/api/auth/logout", Some(&credential.token), None)
        });
        self.forget_session()?;
        if let Some(Err(_)) = result {
            self.set_error("Signed out on this device. The server could not revoke its session; you can revoke it from your web account.".into())?;
        }
        self.status()
    }
    pub(super) fn mark_join_pending(&self, enable_sync: bool, receipt: Option<&str>, account_id: Option<&str>) -> Result<(), String> {
        if receipt.is_some_and(|value| !super::browser_login::valid_flow_id(value)) || (enable_sync && receipt.is_none()) {
            return Err("Invalid leaderboard join receipt. Start joining again.".into());
        }
        let mut preferences = self
            .preferences
            .lock()
            .map_err(|_| "Cloud preferences unavailable.")?;
        preferences.origin = self.api()?.origin.clone();
        preferences.join_pending = true;
        preferences.enable_sync_on_join = enable_sync;
        preferences.join_receipt = receipt.map(str::to_owned);
        preferences.join_account_id = account_id.map(str::to_owned);
        preferences.join_sync_activation_ready = false;
        self.save(&preferences)?;
        self.status
            .lock()
            .map_err(|_| "Cloud status unavailable.")?
            .join_pending = true;
        Ok(())
    }
    pub(super) fn dispatch_join_sync(&self, start: impl FnOnce()) -> Result<(), String> {
        let mut preferences = self.preferences.lock().map_err(|_| "Cloud preferences unavailable.")?;
        if !preferences.join_sync_activation_ready || !preferences.sync_enabled || preferences.session_revoked {
            return Ok(());
        }
        preferences.join_sync_activation_ready = false;
        self.save(&preferences)?;
        drop(preferences);
        start();
        Ok(())
    }
    pub fn open_page(&self, page: &str, enable_sync: bool, receipt: Option<&str>) -> Result<(), String> {
        self.api()?;
        let origin = self.app_origin.as_ref().map_err(Clone::clone)?;
        let url = if page == "join-rank" {
            let credential = self.credential()?;
            let url = super::origins::join_url(origin, &credential.user.id, enable_sync, receipt)?;
            self.mark_join_pending(enable_sync, receipt, Some(&credential.user.id))?;
            url
        } else {
            if enable_sync || receipt.is_some() { return Err("Sync consent is only available when joining the leaderboard.".into()); }
            super::origins::page_url(origin, page, self.status()?.rank_period)?
        };
        open::that(url).map_err(|_| "The DevClash page could not be opened in your browser.".into())
    }
}
