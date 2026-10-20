use super::{CloudStore, Status};
use crate::usage::UsageStore;
use serde_json::Value;
use tauri::{Emitter, Manager};

fn publish(app: &tauri::AppHandle, result: Result<Status, String>) -> Result<Status, String> {
    let published = match result {
        Ok(status) => {
            app.emit("cloud-updated", &status)
                .map_err(|_| "Cloud status could not be updated.")?;
            Ok(status)
        }
        Err(error) => {
            let cloud = app.state::<CloudStore>();
            let _ = cloud.set_error(error.clone());
            if let Ok(status) = cloud.status() {
                let _ = app.emit("cloud-updated", status);
            }
            Err(error)
        }
    };
    // Publish the enabled setting first; collecting and syncing history can take time.
    app.state::<CloudStore>().dispatch_join_sync(|| {
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || automatic_sync(&handle));
    })?;
    published
}
pub fn automatic_sync(app: &tauri::AppHandle) {
    match app.state::<CloudStore>().sync_enabled() {
        Ok(false) => return,
        Ok(true) => {}
        Err(error) => {
            let _ = publish(app, Err(error));
            return;
        }
    }
    let result = app
        .state::<UsageStore>()
        .cloud_records()
        .and_then(|records| app.state::<CloudStore>().sync_automatic_with_progress(records, |status| {
            if let Err(error) = app.emit("cloud-updated", &status) {
                eprintln!("Cloud progress could not be published: {error}");
            }
        }));
    let _ = publish(app, result);
}

#[tauri::command]
pub async fn cloud_status(app: tauri::AppHandle) -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(move || {
        publish(&app, app.state::<CloudStore>().refresh_account())
    })
    .await
    .map_err(|_| "Your cloud account could not be read.")?
}
#[tauri::command]
pub async fn cloud_refresh_projects(app: tauri::AppHandle) -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // A project-list fetch failure is shown by this dialog action. It is
        // not a usage-upload failure and must not mark the dashboard's sync.
        match app.state::<CloudStore>().refresh_projects() {
            Ok(status) => {
                app.emit("cloud-updated", &status)
                    .map_err(|_| "Cloud status could not be updated.")?;
                Ok(status)
            }
            Err(error) => {
                if let Ok(status) = app.state::<CloudStore>().status() {
                    let _ = app.emit("cloud-updated", status);
                }
                Err(error)
            }
        }
    })
    .await
    .map_err(|_| "Your projects could not be refreshed.")?
}
#[tauri::command]
pub async fn cloud_set_active_project(
    app: tauri::AppHandle,
    project_id: Option<String>,
) -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(move || {
        publish(
            &app,
            app.state::<CloudStore>().set_active_project(project_id),
        )
    })
    .await
    .map_err(|_| "Your active project could not be saved.")?
}
#[tauri::command]
pub async fn cloud_dismiss_project_warning(app: tauri::AppHandle) -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(move || {
        publish(&app, app.state::<CloudStore>().dismiss_project_warning())
    })
    .await
    .map_err(|_| "The project notice could not be dismissed.")?
}
#[tauri::command]
pub async fn cloud_request_code(app: tauri::AppHandle, email: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<CloudStore>().request_code(email))
        .await
        .map_err(|_| "The sign-in code could not be requested.")?
}
#[tauri::command]
pub async fn cloud_verify_code(
    app: tauri::AppHandle,
    challenge_id: String,
    code: String,
) -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(move || {
        publish(
            &app,
            app.state::<CloudStore>().verify_code(challenge_id, code),
        )
    })
    .await
    .map_err(|_| "Your sign-in code could not be verified.")?
}
#[tauri::command]
pub async fn cloud_set_sync(app: tauri::AppHandle, enabled: bool) -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let status = publish(&app, app.state::<CloudStore>().set_sync(enabled))?;
        if enabled {
            automatic_sync(&app);
        }
        if enabled {
            app.state::<CloudStore>().status()
        } else {
            Ok(status)
        }
    })
    .await
    .map_err(|_| "Cloud sync could not be changed.")?
}
#[tauri::command]
pub async fn cloud_sync_now(app: tauri::AppHandle) -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let records = app.state::<UsageStore>().cloud_records()?;
        let cloud = app.state::<CloudStore>();
        let status = publish(&app, cloud.sync(records))?;
        if status.user.is_some() {
            publish(&app, cloud.refresh_account())
        } else {
            Ok(status)
        }
    })
    .await
    .map_err(|_| "Your usage could not be synced.")?
}
#[tauri::command]
pub async fn cloud_participation(
    app: tauri::AppHandle,
    ranked: bool,
    public_profile: bool,
) -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(move || {
        publish(
            &app,
            app.state::<CloudStore>()
                .set_participation(ranked, public_profile),
        )
    })
    .await
    .map_err(|_| "Your public participation could not be changed.")?
}
#[tauri::command]
pub async fn cloud_logout(app: tauri::AppHandle) -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(move || publish(&app, app.state::<CloudStore>().logout()))
        .await
        .map_err(|_| "You could not be signed out.")?
}
#[tauri::command]
pub fn cloud_open_page(store: tauri::State<'_, CloudStore>, page: String, enable_sync_on_join: Option<bool>, join_receipt: Option<String>) -> Result<(), String> {
    store.open_page(&page, enable_sync_on_join.unwrap_or(false), join_receipt.as_deref())
}

#[tauri::command]
pub async fn cloud_set_rank_period(
    app: tauri::AppHandle,
    period: String,
) -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(move || {
        publish(&app, app.state::<CloudStore>().set_rank_period(&period))
    })
    .await
    .map_err(|_| "Rank period could not be changed.")?
}

#[tauri::command]
pub async fn cloud_start_browser_login(app: tauri::AppHandle, intent: Option<String>, enable_sync_on_join: Option<bool>, join_receipt: Option<String>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<CloudStore>().start_browser_login(intent.as_deref(), enable_sync_on_join.unwrap_or(false), join_receipt.as_deref()))
        .await
        .map_err(|_| "Browser sign-in could not start.")?
}
#[tauri::command]
pub async fn cloud_poll_browser_login(
    app: tauri::AppHandle,
    flow_id: String,
) -> Result<Option<Status>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        match app.state::<CloudStore>().poll_browser_login(&flow_id)? {
            Some(status) => publish(&app, Ok(status)).map(Some),
            None => Ok(None),
        }
    })
    .await
    .map_err(|_| "Browser sign-in could not finish.")?
}
#[tauri::command]
pub async fn cloud_cancel_browser_login(
    app: tauri::AppHandle,
    flow_id: String,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<CloudStore>().cancel_browser_login(&flow_id)
    })
    .await
    .map_err(|_| "Browser sign-in could not be cancelled.")?
}

#[tauri::command]
pub async fn cloud_share_rank(app: tauri::AppHandle, period: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<CloudStore>().share_rank(&period))
        .await.map_err(|_| "Your rank could not be loaded.")?
}
