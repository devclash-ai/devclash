use super::{CloudStore, Status};
use serde_json::{json, Value};

pub(super) struct BrowserLogin {
    id: String,
    proof: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}
impl CloudStore {
    pub fn start_browser_login(&self) -> Result<Value, String> {
        let mut pending = self
            .browser_login
            .lock()
            .map_err(|_| "Sign-in unavailable.")?;
        *pending = None;
        let origin = self.app_origin.as_ref().map_err(Clone::clone)?;
        let reply = self.request(
            "POST",
            "/api/auth/desktop/start",
            None,
            Some(&json!({
                "deviceName": format!("DevClash on {}", std::env::consts::OS)
            })),
        )?;
        let id = reply["flowId"]
            .as_str()
            .filter(|id| valid_flow_id(id))
            .ok_or("Invalid sign-in request.")?
            .to_string();
        let proof = reply["proof"]
            .as_str()
            .filter(|v| {
                v.len() == 43
                    && v.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            })
            .ok_or("Invalid sign-in proof.")?
            .to_string();
        let code = reply["displayCode"]
            .as_str()
            .filter(|v| v.len() == 8 && v.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or("Invalid connection code.")?;
        let expiry = reply["expiresAt"]
            .as_str()
            .ok_or("Invalid sign-in expiry.")?;
        let expires_at = chrono::DateTime::parse_from_rfc3339(expiry)
            .map_err(|_| "Invalid sign-in expiry.")?
            .with_timezone(&chrono::Utc);
        open::that(format!("{origin}/auth/desktop#{id}"))
            .map_err(|_| "Your browser could not be opened. Try signing in again.")?;
        *pending = Some(BrowserLogin {
            id: id.clone(),
            proof,
            expires_at,
        });
        Ok(json!({"flowId":id,"displayCode":code,"expiresAt":expiry}))
    }
    pub fn cancel_browser_login(&self, flow_id: &str) -> Result<(), String> {
        let mut pending = self
            .browser_login
            .lock()
            .map_err(|_| "Sign-in unavailable.")?;
        if pending.as_ref().is_some_and(|flow| flow.id == flow_id) {
            *pending = None;
        }
        Ok(())
    }
    pub fn poll_browser_login(&self, flow_id: &str) -> Result<Option<Status>, String> {
        let mut pending = self
            .browser_login
            .lock()
            .map_err(|_| "Sign-in unavailable.")?;
        let flow = pending
            .as_ref()
            .filter(|flow| flow.id == flow_id)
            .ok_or("Sign-in was cancelled. Start again.")?;
        if flow.expires_at <= chrono::Utc::now() {
            *pending = None;
            return Err("Sign-in expired. Start again in the app.".into());
        }
        let reply = self.request(
            "POST",
            "/api/auth/desktop/poll",
            None,
            Some(&json!({"flowId":flow.id,"proof":flow.proof})),
        )?;
        if reply["status"] == "pending" {
            return Ok(None);
        }
        if reply["status"] != "connected" {
            return Err("Unexpected sign-in response. Try again.".into());
        }
        *pending = None;
        let _guard = self.syncing.lock().map_err(|_| "Cloud sync unavailable.")?;
        self.accept_session(reply).map(Some)
    }
}
fn valid_flow_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
#[cfg(test)]
mod tests {
    #[test]
    fn flow_ids_cannot_inject_a_browser_path_or_query() {
        assert!(super::valid_flow_id("01234567-1234-1234-1234-0123456789ab"));
        for id in [
            "../../evil",
            "01234567-1234-1234-1234-0123456789ab?token=x",
            "https://evil.test",
        ] {
            assert!(!super::valid_flow_id(id));
        }
    }
    #[test]
    fn cancelled_and_expired_attempts_never_exchange_a_session() {
        let directory =
            std::env::temp_dir().join(format!("devclash-browser-login-{}", std::process::id()));
        let cloud =
            super::CloudStore::load_with_origins(directory.clone(), "http://127.0.0.1:9", None);
        let id = "01234567-1234-1234-1234-0123456789ab";
        *cloud.browser_login.lock().unwrap() = Some(super::BrowserLogin {
            id: id.into(),
            proof: "synthetic-test-proof".into(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(1),
        });
        cloud.cancel_browser_login("old-request").unwrap();
        assert!(cloud.browser_login.lock().unwrap().is_some());
        cloud.cancel_browser_login(id).unwrap();
        assert!(cloud
            .poll_browser_login(id)
            .err().unwrap()
            .contains("cancelled"));
        *cloud.browser_login.lock().unwrap() = Some(super::BrowserLogin {
            id: id.into(),
            proof: "synthetic-test-proof".into(),
            expires_at: chrono::Utc::now() - chrono::Duration::seconds(1),
        });
        assert!(cloud
            .poll_browser_login(id)
            .err().unwrap()
            .contains("expired"));
        assert!(cloud.browser_login.lock().unwrap().is_none());
    }
}
