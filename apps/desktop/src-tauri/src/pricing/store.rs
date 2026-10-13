use super::{catalog::Catalog, ModelPricing, Supplement};
use chrono::Utc;
use reqwest::{blocking::Client, header::IF_NONE_MATCH, StatusCode};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

const SOURCES: [(&str, &str); 3] = [
    ("supplement", "https://robinebers.github.io/openusage/pricing_supplement.json"),
    ("litellm", "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json"),
    ("models_dev", "https://models.dev/api.json"),
];
#[derive(Clone, Default, Deserialize, Serialize)]
struct SourceState {
    etag: Option<String>,
    fetched_at: Option<i64>,
    failed_at: Option<i64>,
}
impl SourceState {
    fn due(&self, now: i64) -> bool {
        self.failed_at.is_none_or(|time| now - time >= 1800)
            && self.fetched_at.is_none_or(|time| now - time >= 3600)
    }
}
struct State {
    current: Arc<ModelPricing>,
    sources: HashMap<String, SourceState>,
    refreshing: bool,
    warnings: Vec<String>,
}
pub struct PricingStore {
    directory: PathBuf,
    state: Arc<Mutex<State>>,
}
impl PricingStore {
    pub fn load(directory: PathBuf) -> Self {
        let sources = std::fs::read(directory.join("state.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            state: Arc::new(Mutex::new(State {
                current: Arc::new(load_prices(&directory)),
                sources,
                refreshing: false,
                warnings: Vec::new(),
            })),
            directory,
        }
    }
    /// Usage scanning always proceeds with bundled/cached rates. Refresh public lists separately.
    pub fn current(&self) -> Arc<ModelPricing> {
        let mut state = self.state.lock().unwrap();
        let now = Utc::now().timestamp();
        let due = SOURCES
            .iter()
            .any(|(id, _)| state.sources.get(*id).is_none_or(|source| source.due(now)));
        if due && !state.refreshing {
            state.refreshing = true;
            let shared = Arc::clone(&self.state);
            let directory = self.directory.clone();
            let mut sources = state.sources.clone();
            std::thread::spawn(move || {
                let mut warnings = Vec::new();
                let client = Client::builder().timeout(Duration::from_secs(15)).build();
                for (id, url) in SOURCES {
                    let source = sources.entry(id.into()).or_default();
                    if !source.due(now) {
                        continue;
                    }
                    let result = client
                        .as_ref()
                        .map_err(|e| e.to_string())
                        .and_then(|client| fetch(client, url, id, source, &directory));
                    match result {
                        Ok(()) => {
                            source.fetched_at = Some(now);
                            source.failed_at = None;
                        }
                        Err(error) => {
                            eprintln!("{id} pricing refresh failed: {error}");
                            source.failed_at = Some(now);
                            warnings.push(
                                "The latest model prices could not be fetched. Using saved prices."
                                    .into(),
                            );
                        }
                    }
                }
                if let Err(error) = serde_json::to_vec(&sources)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| atomic_write(&directory.join("state.json"), &bytes))
                {
                    eprintln!("Pricing cache save failed: {error}");
                    warnings.push(
                        "Model prices are available, but their update status could not be saved."
                            .into(),
                    );
                }
                let current = Arc::new(load_prices(&directory));
                let mut state = shared.lock().unwrap();
                state.current = current;
                state.sources = sources;
                state.warnings = warnings;
                state.refreshing = false;
            });
        }
        Arc::clone(&state.current)
    }
    pub fn warnings(&self) -> Vec<String> {
        self.state.lock().unwrap().warnings.clone()
    }
}
fn load_prices(directory: &Path) -> ModelPricing {
    let read = |id| std::fs::read(directory.join(format!("{id}.json"))).ok();
    let supplement = read("supplement");
    let primary = read("litellm");
    let secondary = read("models_dev");
    ModelPricing::from_cached(
        supplement.as_deref(),
        primary.as_deref(),
        secondary.as_deref(),
    )
    .unwrap_or_else(|error| {
        eprintln!("Pricing cache load failed: {error}");
        ModelPricing::bundled()
    })
}
fn fetch(
    client: &Client,
    url: &str,
    id: &str,
    state: &mut SourceState,
    directory: &Path,
) -> Result<(), String> {
    let path = directory.join(format!("{id}.json"));
    let mut request = client.get(url);
    if path.exists() {
        if let Some(etag) = &state.etag {
            request = request.header(IF_NONE_MATCH, etag);
        }
    }
    let response = request.send().map_err(|e| e.to_string())?;
    if response.status() == StatusCode::NOT_MODIFIED && path.exists() {
        return Ok(());
    }
    let response = response.error_for_status().map_err(|e| e.to_string())?;
    let etag = response
        .headers()
        .get("etag")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let mut bytes = Vec::new();
    response
        .take(20 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 20 * 1024 * 1024 {
        return Err("Pricing feed is too large".into());
    }
    let normalized = match id {
        "supplement" => {
            Supplement::decode(&bytes)?;
            bytes
        }
        "litellm" => serde_json::to_vec(&Catalog::litellm(&bytes)?).map_err(|e| e.to_string())?,
        _ => serde_json::to_vec(&Catalog::models_dev(&bytes)?).map_err(|e| e.to_string())?,
    };
    atomic_write(&path, &normalized)?;
    state.etag = etag;
    Ok(())
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(path.parent().ok_or("Invalid pricing cache directory")?)
        .map_err(|e| e.to_string())?;
    let temp = path.with_extension("tmp");
    let mut file = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    #[cfg(target_os = "windows")]
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    std::fs::rename(temp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn success_refreshes_hourly_and_failure_retries_without_hammering() {
        let mut source = SourceState {
            fetched_at: Some(100),
            ..SourceState::default()
        };
        assert!(!source.due(3699));
        assert!(source.due(3700));
        source.failed_at = Some(3700);
        assert!(!source.due(5499));
        assert!(source.due(5500));
    }
}
