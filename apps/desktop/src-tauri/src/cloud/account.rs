use super::{CloudStore, Credential, Preferences, Status};
use serde_json::{json, Value};
impl CloudStore {
    // Read a share period without changing dashboard/widget rank preferences.
    pub fn share_rank(&self, period: &str) -> Result<Value, String> {
        if !matches!(period, "Today" | "Week" | "Month") {
            return Err("Choose Today, Week or Month for Rank Battle.".into());
        }
        let credential = self.credential()?;
        let reply = self.request("GET", &format!("/api/rank?period={period}"), Some(&credential.token), None)?;
        Ok(json!({"position": reply["own"]["rank"], "tokens": reply["own"]["tokens"], "updatedAt": reply["generatedAt"]}))
    }
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

#[cfg(test)]
mod share_tests {
    use super::super::*;
    use std::{io::{BufRead, BufReader, Write}, net::TcpListener, thread};
    #[test]
    fn share_rank_reads_requested_period_without_mutating_dashboard_or_sync() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            for (period, own) in [("Week", json!({"rank":42,"tokens":123456,"private":"excluded"})), ("Month", Value::Null)] {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new(); reader.read_line(&mut first).unwrap();
                assert!(first.starts_with(&format!("GET /api/rank?period={period} ")));
                loop { let mut line = String::new(); reader.read_line(&mut line).unwrap(); if line == "\r\n" { break; } }
                let body = json!({"own":own,"generatedAt":"2026-10-20T12:00:00Z"}).to_string();
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        let directory = std::env::temp_dir().join(format!("devclash-share-rank-{}", Utc::now().timestamp_nanos_opt().unwrap()));
        let cloud = CloudStore::load_with_origins(directory.clone(), &origin, None);
        *cloud.credential.lock().unwrap() = Some(Credential {token:"synthetic-session".into(), user:User{id:"qa".into(),email:"qa@example.invalid".into()}, expires_at:(Utc::now()+chrono::Duration::hours(1)).to_rfc3339()});
        let before = serde_json::to_value(cloud.status().unwrap()).unwrap();
        assert!(cloud.share_rank("Week&user=other").is_err());
        assert_eq!(cloud.share_rank("Week").unwrap(), json!({"position":42,"tokens":123456,"updatedAt":"2026-10-20T12:00:00Z"}));
        assert!(cloud.share_rank("Month").unwrap()["position"].is_null());
        assert_eq!(serde_json::to_value(cloud.status().unwrap()).unwrap(), before);
        server.join().unwrap();
        if directory.exists() { std::fs::remove_dir_all(directory).unwrap(); }
    }
}
