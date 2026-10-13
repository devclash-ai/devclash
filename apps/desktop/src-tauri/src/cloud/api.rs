use reqwest::blocking::Client;
use serde_json::Value;
use std::time::Duration;

pub struct ApiError {
    pub status: u16,
    pub message: String,
    pub code: Option<String>,
    pub project_id: Option<String>,
}
impl From<String> for ApiError {
    fn from(message: String) -> Self {
        Self {
            status: 0,
            message,
            code: None,
            project_id: None,
        }
    }
}
pub struct Api {
    pub origin: String,
    client: Client,
}
impl Api {
    pub fn new(origin: &str) -> Result<Self, String> {
        let origin = validate_origin(origin)?;
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("DevClash Desktop/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| "The cloud connection could not be initialized.")?;
        Ok(Self { origin, client })
    }
    pub fn request(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<&Value>,
    ) -> Result<Value, ApiError> {
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| "Invalid cloud request.".to_string())?;
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.origin));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().map_err(|error| {
            eprintln!(
                "Cloud connection failed: {}",
                if error.is_timeout() {
                    "timeout"
                } else {
                    "transport error"
                }
            );
            ApiError {
                status: 0,
                message: "Could not reach DevClash. Check your connection and try again.".into(),
                code: None,
                project_id: None,
            }
        })?;
        let status = response.status();
        let value = response.json::<Value>().map_err(|_| ApiError {
            status: status.as_u16(),
            message: "DevClash returned an unreadable response. Please try again.".into(),
            code: None,
            project_id: None,
        })?;
        if !status.is_success() {
            eprintln!("Cloud request failed with HTTP {}", status.as_u16());
            return Err(ApiError {
                status: status.as_u16(),
                code: value["code"].as_str().map(str::to_owned),
                project_id: value["projectId"].as_str().map(str::to_owned),
                message: value["error"]["message"]
                    .as_str()
                    .or_else(|| value["error"].as_str())
                    .or_else(|| value["message"].as_str())
                    .unwrap_or(if status.as_u16() == 401 {
                        "Your session expired or was revoked. Sign in again."
                    } else {
                        "The cloud request failed. Please try again."
                    })
                    .chars()
                    .take(300)
                    .collect(),
            });
        }
        Ok(value)
    }
}

pub fn validate_origin(value: &str) -> Result<String, String> {
    let url =
        reqwest::Url::parse(value.trim()).map_err(|_| "The DevClash API address is invalid.")?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !((url.scheme() == "https") || (url.scheme() == "http" && local))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err("Use an HTTPS DevClash origin, or localhost for development.".into());
    }
    Ok(value.trim().trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cloud_origin_never_accepts_credentials_or_insecure_remote_hosts() {
        assert_eq!(
            validate_origin("https://devclash.example/").unwrap(),
            "https://devclash.example"
        );
        assert!(validate_origin("http://127.0.0.1:1440").is_ok());
        for invalid in [
            "http://devclash.example",
            "https://secret@devclash.example",
            "https://devclash.example/api",
            "file:///tmp/token",
            "https://devclash.example/?token=secret",
        ] {
            assert!(validate_origin(invalid).is_err());
        }
    }
}
