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
                ..Preferences::default()
            };
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
    pub fn open_page(&self, page: &str) -> Result<(), String> {
        self.api()?;
        let origin = self.app_origin.as_ref().map_err(Clone::clone)?;
        let url = super::origins::page_url(origin, page, self.status()?.rank_period)?;
        open::that(url).map_err(|_| "The DevClash page could not be opened in your browser.".into())
    }
}
