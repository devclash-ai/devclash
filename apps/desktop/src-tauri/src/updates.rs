use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Idle,
    Checking,
    Available,
    Downloading,
    Installing,
    Ready,
    Error,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    enabled: bool,
    install_mode: &'static str,
    restart_required: bool,
    current_version: String,
    available_version: Option<String>,
    notes: String,
    phase: Phase,
    progress: Option<f64>,
    checked_at: Option<String>,
    error: Option<String>,
}

struct Inner {
    status: UpdateStatus,
    pending: Option<Update>,
    busy: bool,
    failures: u32,
    next_check: Instant,
}

pub struct UpdateStore {
    inner: Mutex<Inner>,
    wake: Condvar,
}

fn channel_allows_updates(identifier: &str, debug: bool, updater_fixture: bool) -> bool {
    updater_fixture || (identifier == "ai.devclash.desktop" && !debug)
}
fn updates_allowed(app: &AppHandle) -> bool {
    channel_allows_updates(&app.config().identifier, cfg!(debug_assertions), cfg!(feature = "updater-qa"))
}
fn automatic_preference(value: &Value) -> bool {
    value
        .get("autoUpdates")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn retry_delay(failures: u32) -> Duration {
    Duration::from_secs(
        (15 * 60 * 2_u64.pow(failures.saturating_sub(1).min(5))).min(CHECK_INTERVAL.as_secs()),
    )
}

fn status(app: &AppHandle) -> UpdateStatus {
    app.state::<UpdateStore>()
        .inner
        .lock()
        .unwrap()
        .status
        .clone()
}

fn publish(app: &AppHandle) {
    let snapshot = status(app);
    let _ = app.emit("update-status", &snapshot);
    #[cfg(feature = "updater-qa")]
    qa::trace(app, &snapshot);
}

pub fn apply_preferences(app: &AppHandle, preferences: &Value) {
    if let Some(store) = app.try_state::<UpdateStore>() {
        let mut inner = store.inner.lock().unwrap();
        let enabled = updates_allowed(app) && automatic_preference(preferences);
        if enabled && !inner.status.enabled {
            inner.next_check = Instant::now();
        }
        inner.status.enabled = enabled;
        drop(inner);
        store.wake.notify_one();
        publish(app);
    }
}

pub fn initialize(app: &AppHandle, directory: &Path) {
    let path = directory.join("display-preferences.json");
    // A damaged existing file should not turn a saved opt-out into an opt-in.
    let enabled = if path.exists()
        || path.with_extension("bak").exists()
        || path.with_extension("tmp").exists()
    {
        crate::storage::read_json::<Value>(&path)
            .map(|value| automatic_preference(&value))
            .unwrap_or(false)
    } else {
        true
    };
    let enabled = enabled && updates_allowed(app);
    app.manage(UpdateStore {
        inner: Mutex::new(Inner {
            status: UpdateStatus {
                enabled,
                install_mode: if silent_install_supported(app) {
                    "background"
                } else {
                    "manual"
                },
                restart_required: !cfg!(target_os = "windows"),
                current_version: app.package_info().version.to_string(),
                available_version: None,
                notes: String::new(),
                phase: Phase::Idle,
                progress: None,
                checked_at: None,
                error: None,
            },
            pending: None,
            busy: false,
            failures: 0,
            next_check: Instant::now() + Duration::from_secs(3),
        }),
        wake: Condvar::new(),
    });
    publish(app);
    let handle = app.clone();
    std::thread::spawn(move || loop {
        let store = handle.state::<UpdateStore>();
        let mut inner = store.inner.lock().unwrap();
        while !inner.status.enabled
            || inner.busy
            || inner.status.phase == Phase::Ready
            || Instant::now() < inner.next_check
        {
            if !inner.status.enabled || inner.busy || inner.status.phase == Phase::Ready {
                inner = store.wake.wait(inner).unwrap();
                continue;
            }
            let wait = inner
                .next_check
                .saturating_duration_since(Instant::now())
                .max(Duration::from_secs(1))
                .min(CHECK_INTERVAL);
            inner = store.wake.wait_timeout(inner, wait).unwrap().0;
        }
        drop(inner);
        tauri::async_runtime::block_on(perform_check(handle.clone(), true));
    });
    #[cfg(feature = "updater-qa")]
    qa::start(app);
}

#[tauri::command]
pub fn update_status(app: AppHandle) -> UpdateStatus {
    status(&app)
}

pub fn enabled(app: &AppHandle) -> Option<bool> {
    app.try_state::<UpdateStore>()
        .map(|store| store.inner.lock().unwrap().status.enabled)
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> UpdateStatus {
    perform_check(app, false).await
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> UpdateStatus {
    let candidate = {
        let store = app.state::<UpdateStore>();
        let mut inner = store.inner.lock().unwrap();
        if inner.busy || inner.status.phase == Phase::Ready {
            return inner.status.clone();
        }
        let Some(candidate) = inner.pending.clone() else {
            return inner.status.clone();
        };
        inner.busy = true;
        candidate
    };
    perform_install(app, candidate, false).await
}

async fn perform_check(app: AppHandle, automatic: bool) -> UpdateStatus {
    if !updates_allowed(&app) {
        let store = app.state::<UpdateStore>();
        let mut inner = store.inner.lock().unwrap();
        inner.status.phase = Phase::Error;
        inner.status.error = Some("Production updates are disabled in QA and preview builds.".into());
        inner.status.enabled = false;
        eprintln!("Production update check blocked for QA/preview build");
        let result = inner.status.clone();
        drop(inner);
        publish(&app);
        return result;
    }
    {
        let store = app.state::<UpdateStore>();
        let mut inner = store.inner.lock().unwrap();
        if inner.busy || inner.status.phase == Phase::Ready || (automatic && !inner.status.enabled)
        {
            return inner.status.clone();
        }
        inner.busy = true;
        inner.status.phase = Phase::Checking;
        inner.status.error = None;
        inner.status.progress = None;
    }
    publish(&app);
    let exit_handle = app.clone();
    let result = match app
        .updater_builder()
        .on_before_exit(move || crate::window_position::save_current(&exit_handle))
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(updater) => updater.check().await,
        Err(error) => Err(error),
    };
    match result {
        Ok(candidate) => {
            let silent_install = silent_install_supported(&app);
            let auto_install = {
                let store = app.state::<UpdateStore>();
                let mut inner = store.inner.lock().unwrap();
                inner.failures = 0;
                inner.next_check = Instant::now() + CHECK_INTERVAL;
                inner.status.checked_at = Some(chrono::Utc::now().to_rfc3339());
                inner.status.available_version =
                    candidate.as_ref().map(|update| update.version.clone());
                inner.status.notes = candidate
                    .as_ref()
                    .and_then(|update| update.body.clone())
                    .unwrap_or_default();
                inner.status.phase = if candidate.is_some() {
                    Phase::Available
                } else {
                    Phase::Idle
                };
                inner.status.install_mode = if silent_install {
                    "background"
                } else {
                    "manual"
                };
                inner.pending = candidate.clone();
                inner.status.enabled && candidate.is_some()
            };
            // Package managers and protected app folders require an explicit install action.
            if auto_install && silent_install {
                publish(&app);
                return perform_install(app, candidate.unwrap(), true).await;
            }
            finish(&app);
        }
        Err(error) => fail(&app, "check", &error),
    }
    status(&app)
}

async fn perform_install(app: AppHandle, mut candidate: Update, automatic: bool) -> UpdateStatus {
    {
        let store = app.state::<UpdateStore>();
        let mut inner = store.inner.lock().unwrap();
        if automatic && !inner.status.enabled {
            drop(inner);
            finish(&app);
            return status(&app);
        }
        inner.status.phase = Phase::Downloading;
        inner.status.error = None;
        inner.status.progress = Some(0.0);
    }
    publish(&app);
    candidate.timeout = Some(Duration::from_secs(120));
    let progress_handle = app.clone();
    let mut downloaded = 0_u64;
    let bytes = candidate
        .download(
            move |chunk, total| {
                downloaded += chunk as u64;
                let store = progress_handle.state::<UpdateStore>();
                let mut inner = store.inner.lock().unwrap();
                let progress = total
                    .filter(|total| *total > 0)
                    .map(|total| (downloaded as f64 / total as f64 * 100.0).min(100.0));
                // Avoid emitting an event for every small network chunk.
                if progress.map(|value| value.floor())
                    != inner.status.progress.map(|value| value.floor())
                {
                    inner.status.progress = progress;
                    drop(inner);
                    publish(&progress_handle);
                }
            },
            || {},
        )
        .await;
    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(error) => {
            fail(&app, "download", &error);
            return status(&app);
        }
    };
    {
        let store = app.state::<UpdateStore>();
        let mut inner = store.inner.lock().unwrap();
        // This lock makes disabling and the installation boundary an atomic decision.
        if automatic && !inner.status.enabled {
            inner.status.phase = Phase::Available;
            inner.status.progress = None;
            drop(inner);
            finish(&app);
            return status(&app);
        }
        inner.status.phase = Phase::Installing;
        inner.status.progress = Some(100.0);
    }
    publish(&app);
    // The macOS/Linux installers replace the package but do not interrupt the running app.
    // Windows NSIS installation closes this process and reopens the updated app.
    let result = tauri::async_runtime::spawn_blocking(move || {
        candidate.restart_after_install(true).install(bytes)
    })
    .await;
    match result {
        Ok(Ok(())) => {
            let store = app.state::<UpdateStore>();
            let mut inner = store.inner.lock().unwrap();
            inner.status.phase = Phase::Ready;
            inner.pending = None;
            drop(inner);
            finish(&app);
        }
        Ok(Err(error)) => fail(&app, "install", &error),
        Err(_) => {
            let store = app.state::<UpdateStore>();
            let mut inner = store.inner.lock().unwrap();
            inner.status.phase = Phase::Error;
            inner.status.error = Some(
                "The update could not be installed. Quit and reopen DevClash, then try again."
                    .into(),
            );
            drop(inner);
            finish(&app);
        }
    }
    status(&app)
}

fn finish(app: &AppHandle) {
    let store = app.state::<UpdateStore>();
    store.inner.lock().unwrap().busy = false;
    store.wake.notify_one();
    publish(app);
}

fn fail(app: &AppHandle, stage: &str, error: &tauri_plugin_updater::Error) {
    #[cfg(feature = "updater-qa")]
    if qa::enabled(app) {
        eprintln!("Updater QA {stage} failed: {error:?}");
    }
    use tauri_plugin_updater::Error;
    let message = match error {
        Error::Minisign(_) | Error::Base64(_) | Error::SignatureUtf8(_) | Error::SignedVersionMismatch { .. } | Error::MissingSignedVersion =>
            "This update could not be verified. Your current version is unchanged. Try checking again later.",
        _ if stage == "install" => "The update could not be installed. Check that DevClash is in a writable folder, then try again.",
        _ if stage == "download" => "The update could not be downloaded. Check your connection and try again.",
        _ => "Updates could not be checked. Check your connection and try again.",
    };
    let store = app.state::<UpdateStore>();
    let mut inner = store.inner.lock().unwrap();
    inner.status.phase = Phase::Error;
    inner.status.error = Some(message.into());
    inner.status.progress = None;
    inner.failures = inner.failures.saturating_add(1);
    let delay = retry_delay(inner.failures);
    #[cfg(feature = "updater-qa")]
    let delay = if qa::enabled(app) {
        Duration::from_secs(1)
    } else {
        delay
    };
    inner.next_check = Instant::now() + delay;
    drop(inner);
    finish(app);
}

fn writable_parent(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let probe = parent.join(format!(
        ".devclash-update-permission-{}",
        std::process::id()
    ));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            std::fs::remove_file(probe).is_ok()
        }
        Err(_) => false,
    }
}

fn silent_install_supported(app: &AppHandle) -> bool {
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        std::env::current_exe()
            .ok()
            .and_then(|exe| {
                exe.ancestors()
                    .find(|path| path.extension().is_some_and(|ext| ext == "app"))
                    .map(Path::to_path_buf)
            })
            .is_some_and(|bundle| writable_parent(&bundle))
    }
    #[cfg(target_os = "linux")]
    {
        app.env()
            .appimage
            .as_ref()
            .is_some_and(|image| writable_parent(Path::new(image)))
    }
    #[cfg(target_os = "windows")]
    {
        // Quiet NSIS updates must preserve the registered, unelevated installation scope.
        if tauri::utils::platform::bundle_type() == Some(tauri::utils::config::BundleType::Nsis)
        {
            return std::env::current_exe().ok().is_some_and(|exe| {
                writable_parent(&exe)
                    && crate::updater_windows::is_current_user_installation(app, &exe)
            });
        }
        false
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = app;
        false
    }
}

#[cfg(feature = "updater-qa")]
pub fn prepare_qa_fixture(app: &AppHandle) -> Result<(), String> {
    qa::prepare_fixture(app.config().identifier.as_str())
}

// Test hooks are absent from production builds and work only in an isolated fixture app.
#[cfg(feature = "updater-qa")]
mod qa {
    use super::*;
    use std::io::Write;

    pub fn prepare_fixture(identifier: &str) -> Result<(), String> {
        if identifier != "ai.devclash.updater.qa" {
            return Ok(());
        }
        let mut arguments = std::env::args_os();
        let mut root = None;
        while let Some(argument) = arguments.next() {
            if argument == "--updater-qa-root" {
                root = Some(std::path::PathBuf::from(
                    arguments
                        .next()
                        .ok_or("Updater QA fixture directory is missing.")?,
                ));
                break;
            }
        }
        let Some(root) = root else {
            return Ok(()); // Existing Mac/Linux fixtures supply their isolated environment.
        };
        if !root.is_absolute() || !root.is_dir() {
            return Err("Updater QA requires an existing absolute fixture directory.".into());
        }
        let config: Value = serde_json::from_slice(
            &std::fs::read(root.join("qa-runtime.json"))
                .map_err(|_| "Updater QA runtime configuration is missing.")?,
        )
        .map_err(|_| "Updater QA runtime configuration is invalid.")?;
        let api = config["apiUrl"]
            .as_str()
            .and_then(|value| reqwest::Url::parse(value).ok())
            .filter(|url| {
                url.scheme() == "http"
                    && url.host_str() == Some("127.0.0.1")
                    && url.port().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
            })
            .ok_or("Updater QA requires a loopback API.")?;
        let action = config["action"]
            .as_str()
            .filter(|action| {
                matches!(
                    *action,
                    "disabled"
                        | "check-exit"
                        | "disable-during-download"
                        | "restart-on-ready"
                        | "manual-restart-on-ready"
                )
            })
            .ok_or("Updater QA action is invalid.")?;
        let data = root.join("data");
        std::fs::create_dir_all(&data).map_err(|_| "Updater QA data directory is unavailable.")?;
        std::env::set_var("DEVCLASH_DATA_DIR", data);
        std::env::set_var("DEVCLASH_QUOTAS_DISABLED", "1");
        std::env::set_var("DEVCLASH_CURSOR_FETCH_DISABLED", "1");
        std::env::set_var("DEVCLASH_API_URL", api.as_str());
        std::env::set_var("DEVCLASH_UPDATE_QA_ACTION", action);
        if config["verifyWidget"].as_bool() == Some(true) {
            std::env::set_var("DEVCLASH_UPDATE_QA_WIDGET", "1");
        }
        for provider in ["CLAUDE", "CODEX", "CURSOR", "OPENCODE"] {
            let directory = root.join(provider.to_lowercase());
            std::fs::create_dir_all(&directory)
                .map_err(|_| "Updater QA provider directory is unavailable.")?;
            std::env::set_var(format!("DEVCLASH_{provider}_DIRS"), directory);
        }
        Ok(())
    }

    pub fn enabled(app: &AppHandle) -> bool {
        app.config().identifier == "ai.devclash.updater.qa"
            && std::env::var_os("DEVCLASH_DATA_DIR").is_some()
            && std::env::var_os("DEVCLASH_UPDATE_QA_ACTION").is_some()
    }
    pub fn trace(app: &AppHandle, status: &UpdateStatus) {
        if !enabled(app) {
            return;
        }
        let directory = crate::migration::directory(app).unwrap();
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.join("update-trace.jsonl"))
        {
            let mut record = serde_json::to_value(status).unwrap();
            record["processId"] = serde_json::json!(std::process::id());
            let _ = writeln!(file, "{}", serde_json::to_string(&record).unwrap());
        }
    }
    pub fn start(app: &AppHandle) {
        if !enabled(app) {
            return;
        }
        let handle = app.clone();
        std::thread::spawn(move || {
            let action = std::env::var("DEVCLASH_UPDATE_QA_ACTION").unwrap();
            if std::env::var_os("DEVCLASH_UPDATE_QA_WIDGET").is_some() {
                if let Err(error) = verify_widget(&handle) {
                    eprintln!("Updater QA widget failed: {error}");
                    handle.exit(2);
                    return;
                }
            }
            if handle.package_info().version.to_string() == "0.0.2" {
                handle.exit(0);
                return;
            }
            if action == "manual-restart-on-ready" {
                std::thread::sleep(Duration::from_secs(1));
                tauri::async_runtime::block_on(check_for_updates(handle.clone()));
                tauri::async_runtime::block_on(install_update(handle.clone()));
            }
            let started = Instant::now();
            loop {
                let snapshot = status(&handle);
                if action == "disabled" && started.elapsed() >= Duration::from_secs(4) {
                    handle.exit(0);
                    return;
                }
                if action == "disable-during-download"
                    && snapshot.phase == Phase::Downloading
                    && snapshot.enabled
                {
                    let mut preferences = crate::migration::read_preferences(handle.clone())
                        .unwrap()
                        .unwrap_or(serde_json::json!({"version": 1}));
                    preferences["autoUpdates"] = Value::Bool(false);
                    crate::migration::save_preferences(handle.clone(), preferences, Some(false))
                        .unwrap();
                }
                if action == "disable-during-download"
                    && !snapshot.enabled
                    && snapshot.phase == Phase::Available
                {
                    handle.exit(0);
                    return;
                }
                if action == "check-exit"
                    && (snapshot.phase == Phase::Error
                        || (snapshot.phase == Phase::Idle && snapshot.checked_at.is_some()))
                {
                    handle.exit(0);
                    return;
                }
                if snapshot.phase == Phase::Ready && action.ends_with("restart-on-ready") {
                    handle.restart();
                }
                if started.elapsed() > Duration::from_secs(90) {
                    handle.exit(2);
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
    }

    fn verify_widget(app: &AppHandle) -> Result<(), String> {
        // This function already runs on the QA worker. Constructing a second
        // WebView2 window on the main thread would deadlock Windows.
        crate::windows::open_widget(app.clone())?;
        let (sender, receiver) = std::sync::mpsc::channel();
        let handle = app.clone();
        app.run_on_main_thread(move || {
            let result = (|| {
                let widget = handle
                    .get_webview_window("widget")
                    .ok_or("Widget missing")?;
                if handle.package_info().version.to_string() == "0.0.1" {
                    widget
                        .set_position(tauri::PhysicalPosition::new(96, 96))
                        .map_err(|error| error.to_string())?;
                }
                let position = widget.outer_position().map_err(|error| error.to_string())?;
                let dashboard_visible = handle
                    .get_webview_window("main")
                    .is_some_and(|window| window.is_visible().unwrap_or(true));
                let record = serde_json::json!({
                    "version": handle.package_info().version.to_string(),
                    "processId": std::process::id(),
                    "x": position.x, "y": position.y,
                    "dashboardVisible": dashboard_visible,
                });
                let directory = crate::migration::directory(&handle)?;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(directory.join("widget-qa.jsonl"))
                    .map_err(|error| error.to_string())?;
                writeln!(file, "{record}").map_err(|error| error.to_string())
            })();
            let _ = sender.send(result);
        })
        .map_err(|error| error.to_string())?;
        receiver
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "Widget verification timed out".to_owned())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_opt_out_and_legacy_preferences_keep_their_intended_behavior() {
        assert!(!automatic_preference(
            &serde_json::json!({"version": 1, "autoUpdates": false, "period": "month"})
        ));
        assert!(automatic_preference(
            &serde_json::json!({"version": 1, "period": "month"})
        ));
        assert!(automatic_preference(
            &serde_json::json!({"autoUpdates": "false"})
        ));
    }
    #[test]
    fn failed_checks_back_off_without_a_retry_storm() {
        assert_eq!(retry_delay(1), Duration::from_secs(900));
        assert_eq!(retry_delay(2), Duration::from_secs(1800));
        assert_eq!(retry_delay(100), CHECK_INTERVAL);
    }
}

#[cfg(test)]
mod isolation_tests {
    #[test]
    fn qa_and_debug_builds_cannot_turn_into_production_through_auto_update() {
        use super::channel_allows_updates;
        assert!(channel_allows_updates("ai.devclash.desktop", false, false));
        assert!(!channel_allows_updates("ai.devclash.desktop", true, false));
        assert!(!channel_allows_updates("ai.devclash.diagnostics.qa", false, false));
        assert!(!channel_allows_updates("ai.devclash.desktop.preview", false, false));
        assert!(channel_allows_updates("ai.devclash.updater.fixture", false, true));
    }
}
