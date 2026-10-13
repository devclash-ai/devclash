mod cloud;
mod migration;
mod paths;
mod pricing;
mod storage;
#[cfg(any(target_os = "windows", test))]
mod updater_windows;
mod updates;
mod usage;
mod window_position;
mod windows;
#[tauri::command]
fn restart_app(app: tauri::AppHandle) {
    window_position::save_current(&app);
    app.restart();
}

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};
use usage::{Snapshot, UsageStore};

#[cfg(any(target_os = "macos", test))]
const MACOS_TRAY_ICON: &[u8] = include_bytes!("../icons/tray-template.png");

#[tauri::command]
async fn read_usage(app: tauri::AppHandle) -> Result<Snapshot, String> {
    // Authentication checks and a concurrent refresh may wait on disk or the
    // collector lock. Keep both away from the window's command thread.
    tauri::async_runtime::spawn_blocking(move || app.state::<UsageStore>().read())
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn refresh_usage(app: tauri::AppHandle) -> Result<Snapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let result = app.state::<UsageStore>().refresh_forced()?;
        app.emit("usage-updated", &result)
            .map_err(|error| error.to_string())?;
        cloud::commands::automatic_sync(&app);
        Ok(result)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn authorize_provider_login(source: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || usage::authorize_login(&source))
        .await
        .map_err(|_| "Provider authorization could not finish. Try again.".to_owned())?
}

#[tauri::command]
fn set_tray_summary(
    app: tauri::AppHandle,
    label: String,
    rows: Vec<String>,
    tooltip: String,
) -> Result<(), String> {
    if label.len() > 128
        || tooltip.len() > 512
        || rows.len() > 20
        || rows.iter().any(|row| row.len() > 256)
    {
        return Err("Invalid summary".into());
    }
    if let Some(tray) = app.tray_by_id("devclash") {
        tray.set_tooltip(Some(tooltip))
            .map_err(|error| error.to_string())?;
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        tray.set_title(Some(label.as_str()))
            .map_err(|error| error.to_string())?;
        // Windows has no tray title; Linux panels may omit it and have no tooltip support.
        #[cfg(not(target_os = "macos"))]
        let rows = {
            let mut rows = rows;
            if !label.is_empty() {
                rows.insert(0, format!("Pinned Summary: {label}"));
            }
            rows
        };
        tray.set_menu(Some(
            tray_menu(&app, &rows).map_err(|error| error.to_string())?,
        ))
        .map_err(|error| error.to_string())?;
        if std::env::var_os("DEVCLASH_TRAY_TRACE").is_some() {
            eprintln!("DevClash tray applied: {label}");
        }
    }
    Ok(())
}

fn tray_menu(app: &tauri::AppHandle, rows: &[String]) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    for (index, row) in rows.iter().enumerate() {
        menu.append(&MenuItem::with_id(
            app,
            format!("summary-{index}"),
            row,
            false,
            None::<&str>,
        )?)?;
    }
    if !rows.is_empty() {
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }
    let dashboard = MenuItem::with_id(app, "dashboard", "Open Dashboard", true, None::<&str>)?;
    let widget = MenuItem::with_id(app, "widget", "Floating Widget", true, None::<&str>)?;
    let customize = MenuItem::with_id(app, "customize", "Customize Menu Bar", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh Usage", true, None::<&str>)?;
    let updates = MenuItem::with_id(app, "updates", "App Updates", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit DevClash", true, None::<&str>)?;
    for item in [&dashboard, &widget, &customize, &refresh, &updates] {
        menu.append(item)?;
    }
    menu.append(&separator)?;
    menu.append(&quit)?;
    Ok(menu)
}

fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let menu = tray_menu(app.handle(), &[])?;
    let mut tray = TrayIconBuilder::with_id("devclash")
        .menu(&menu)
        .tooltip("DevClash")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let result = match event.id.as_ref() {
                "dashboard" => windows::show_dashboard(app),
                "widget" | "customize" => {
                    let handle = app.clone();
                    let customize = event.id.as_ref() == "customize";
                    std::thread::spawn(move || {
                        let result = if customize {
                            windows::open_customization(handle, Some("menubar".into()))
                        } else {
                            windows::open_widget(handle)
                        };
                        if let Err(error) = result {
                            eprintln!("Window action failed: {error}");
                        }
                    });
                    Ok(())
                }
                "updates" => windows::show_dashboard(app).and_then(|_| {
                    app.emit("open-app-updates", ())
                        .map_err(|error| error.to_string())
                }),
                "refresh" => {
                    let handle = app.clone();
                    std::thread::spawn(move || {
                        match handle.state::<UsageStore>().refresh_forced() {
                            Ok(snapshot) => {
                                let _ = handle.emit("usage-updated", snapshot);
                                cloud::commands::automatic_sync(&handle);
                            }
                            Err(error) => eprintln!("Usage refresh failed: {error}"),
                        }
                    });
                    Ok(())
                }
                "quit" => {
                    window_position::save_current(app);
                    app.exit(0);
                    Ok(())
                }
                _ => Ok(()),
            };
            if let Err(error) = result {
                eprintln!("Window action failed: {error}");
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = windows::show_dashboard(tray.app_handle());
            }
        });
    #[cfg(target_os = "macos")]
    {
        // macOS templates use alpha as their mask. The app icon's opaque tile hides the mark.
        tray = tray
            .icon(tauri::image::Image::from_bytes(MACOS_TRAY_ICON)?)
            .icon_as_template(true);
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

pub fn run() {
    #[cfg(target_os = "macos")]
    if usage::handle_keychain_helper() {
        return;
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            let _ = windows::show_dashboard(app);
        }))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--background"])
                .build(),
        )
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            read_usage,
            paths::read_sources,
            paths::save_sources,
            restart_app,
            updates::update_status,
            updates::check_for_updates,
            updates::install_update,
            migration::read_preferences,
            migration::save_preferences,
            refresh_usage,
            authorize_provider_login,
            set_tray_summary,
            windows::open_widget,
            windows::open_customization,
            windows::configure_widget,
            windows::fit_widget,
            cloud::commands::cloud_status,
            cloud::commands::cloud_refresh_projects,
            cloud::commands::cloud_set_active_project,
            cloud::commands::cloud_dismiss_project_warning,
            cloud::commands::cloud_set_rank_period,
            cloud::commands::cloud_start_browser_login,
            cloud::commands::cloud_poll_browser_login,
            cloud::commands::cloud_cancel_browser_login,
            cloud::commands::cloud_request_code,
            cloud::commands::cloud_verify_code,
            cloud::commands::cloud_set_sync,
            cloud::commands::cloud_sync_now,
            cloud::commands::cloud_participation,
            cloud::commands::cloud_logout,
            cloud::commands::cloud_open_page
        ])
        .setup(|app| {
            #[cfg(feature = "updater-qa")]
            updates::prepare_qa_fixture(app.handle()).map_err(std::io::Error::other)?;
            let data_directory = std::env::var_os("DEVCLASH_DATA_DIR")
                .or_else(|| std::env::var_os("USAGERANK_DATA_DIR"))
                .map(std::path::PathBuf::from)
                .unwrap_or(app.path().app_local_data_dir()?);
            migration::initialize(&data_directory).map_err(std::io::Error::other)?;
            std::env::set_var(
                "DEVCLASH_SOURCE_SETTINGS",
                data_directory.join("usage-sources.json"),
            );
            let store = UsageStore::load(data_directory.clone());
            app.manage(store);
            app.manage(cloud::CloudStore::load(data_directory.clone()));
            updates::initialize(app.handle(), &data_directory);
            let tray_failed = if let Err(error) = build_tray(app) {
                eprintln!("System tray unavailable: {error}");
                true
            } else {
                false
            };
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            if tray_failed || !std::env::args().any(|arg| arg == "--background") {
                windows::show_dashboard(app.handle())?;
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                match handle.state::<UsageStore>().refresh() {
                    Ok(snapshot) => {
                        let _ = handle.emit("usage-updated", snapshot);
                        cloud::commands::automatic_sync(&handle);
                    }
                    Err(error) => eprintln!("Usage refresh failed: {error}"),
                }
                std::thread::sleep(std::time::Duration::from_secs(300));
            });
            let retry_handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(30));
                cloud::commands::automatic_sync(&retry_handle);
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("DevClash could not start");
}

#[cfg(test)]
mod tests {
    use super::MACOS_TRAY_ICON;

    #[test]
    fn menu_bar_template_preserves_the_mark_without_an_opaque_tile() {
        let icon = tauri::image::Image::from_bytes(MACOS_TRAY_ICON).unwrap();
        assert_eq!((icon.width(), icon.height()), (36, 36));

        let pixels: Vec<_> = icon.rgba().chunks_exact(4).collect();
        let visible = pixels.iter().filter(|pixel| pixel[3] != 0).count();
        assert!(visible > pixels.len() / 4 && visible < pixels.len() / 2);
        for (index, pixel) in pixels.iter().enumerate() {
            let (x, y) = (index % 36, index / 36);
            if x == 0 || x == 35 || y == 0 || y == 35 {
                assert_eq!(pixel[3], 0, "The template needs a transparent margin");
            }
            if pixel[3] != 0 {
                assert_eq!(&pixel[..3], &[0, 0, 0]);
            }
        }
    }
}
