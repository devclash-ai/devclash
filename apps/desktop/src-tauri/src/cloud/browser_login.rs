use super::{CloudStore, Status};
use serde_json::{json, Value};

pub(super) struct BrowserLogin {
    id: String,
    proof: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}
impl CloudStore {
    pub fn start_browser_login(&self, intent: Option<&str>, enable_sync: bool, receipt: Option<&str>) -> Result<Value, String> {
        let joining = match intent {
            None => false,
            Some("join-leaderboard") => true,
            _ => return Err("Unknown desktop sign-in intent.".into()),
        };
        if joining {
            self.mark_join_pending(enable_sync, receipt, None)?;
        } else if enable_sync || receipt.is_some() {
            return Err("Sync consent is only available when joining the leaderboard.".into());
        }
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
        open::that(browser_login_url(origin, &id, joining, enable_sync, receipt)?)
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
fn browser_login_url(origin: &str, id: &str, joining: bool, enable_sync: bool, receipt: Option<&str>) -> Result<String, String> {
    if !valid_flow_id(id) {
        return Err("Invalid sign-in request.".into());
    }
    let mut url = reqwest::Url::parse(origin).map_err(|_| "The browser app address is invalid.")?;
    url.set_path("/auth/desktop");
    if joining { url.query_pairs_mut().append_pair("intent", "join-leaderboard"); }
    super::origins::add_join_receipt(&mut url, enable_sync, receipt)?;
    url.set_fragment(Some(id));
    Ok(url.into())
}
pub(super) fn valid_flow_id(id: &str) -> bool {
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
    fn only_a_join_intent_is_added_to_the_browser_connection_url() {
        let id = "01234567-1234-1234-1234-0123456789ab";
        assert_eq!(super::browser_login_url("https://app.devclash.ai", id, true, false, None).unwrap(), format!("https://app.devclash.ai/auth/desktop?intent=join-leaderboard#{id}"));
        assert_eq!(super::browser_login_url("https://app.devclash.ai", id, false, false, None).unwrap(), format!("https://app.devclash.ai/auth/desktop#{id}"));
        assert!(super::browser_login_url("https://app.devclash.ai", "bad#flow", true, false, None).is_err());
        let consent = super::browser_login_url("https://app.devclash.ai", id, true, true, Some(id)).unwrap();
        let parsed = reqwest::Url::parse(&consent).unwrap();
        assert_eq!(parsed.fragment(), Some(id));
        assert!(parsed.query_pairs().any(|(key, value)| key == "sync" && value == "enable"));
        assert!(parsed.query_pairs().any(|(key, value)| key == "receipt" && value == id));
        assert!(super::browser_login_url("https://app.devclash.ai", id, true, true, None).is_err());
        let directory = std::env::temp_dir().join(format!("devclash-join-intent-{}", std::process::id()));
        let cloud = super::CloudStore::load_with_origins(directory.clone(), "http://127.0.0.1:9", None);
        assert!(cloud.start_browser_login(Some("unexpected"), false, None).unwrap_err().contains("Unknown"));
        assert!(!cloud.status().unwrap().join_pending);
        cloud.mark_join_pending(false, None, None).unwrap();
        let restored = super::CloudStore::load_with_origins(directory.clone(), "http://127.0.0.1:9", None);
        assert!(restored.status().unwrap().join_pending);
        assert!(!restored.status().unwrap().sync_enabled);
        assert!(!restored.status().unwrap().public_profile);
        assert!(!restored.status().unwrap().ranked);
        let _ = std::fs::remove_dir_all(directory);
    }
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
