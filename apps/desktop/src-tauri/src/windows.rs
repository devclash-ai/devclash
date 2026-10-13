use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub fn show_dashboard(app: &tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or("Dashboard unavailable")?;
    #[cfg(target_os = "windows")]
    window
        .set_shadow(false)
        .map_err(|error| error.to_string())?;
    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())
}

// WebView2 window construction must run outside its main-thread callbacks.
#[tauri::command(async)]
pub fn open_widget(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("widget") {
        let position = window.outer_position().map_err(|error| error.to_string())?;
        window.show().map_err(|error| error.to_string())?;
        // Restore after showing: the window manager can reposition a hidden window on remap.
        window
            .set_position(position)
            .map_err(|error| error.to_string())?;
        crate::window_position::ensure_visible(&window, false)?;
        return window.set_focus().map_err(|error| error.to_string());
    }
    let window = WebviewWindowBuilder::new(
        &app,
        "widget",
        WebviewUrl::App("index.html?window=widget".into()),
    )
    .title("DevClash Widget")
    // Restore against the minimum size; the content fit will clamp the final dimensions.
    // A provisional larger height can shift a valid compact widget before it is measured.
    .inner_size(240., 140.)
    .min_inner_size(240., 140.)
    .decorations(false)
    .transparent(true)
    .shadow(!cfg!(target_os = "windows"))
    // GTK's nonresizable window can retain its initial height when content shrinks.
    // Linux uses content-sized geometry constraints in fit_widget instead.
    .resizable(cfg!(target_os = "linux"))
    .always_on_top(true)
    .skip_taskbar(true)
    .accept_first_mouse(true)
    .disable_drag_drop_handler()
    .build()
    .map_err(|error| error.to_string())?;
    crate::window_position::ensure_visible(&window, true)?;
    crate::window_position::track(&window);
    if std::env::var_os("DEVCLASH_WINDOW_TRACE").is_some() {
        window.on_window_event(|event| {
            if let tauri::WindowEvent::Moved(position) = event {
                eprintln!("DevClash widget moved: {}, {}", position.x, position.y);
            }
        });
    }
    Ok(())
}

#[tauri::command(async)]
pub fn open_customization(app: tauri::AppHandle, surface: Option<String>) -> Result<(), String> {
    let surface = surface
        .filter(|value| ["dashboard", "widget", "menubar"].contains(&value.as_str()))
        .unwrap_or("widget".into());
    if let Some(window) = app.get_webview_window("customize") {
        window
            .emit("customize-surface", &surface)
            .map_err(|error| error.to_string())?;
        window.show().map_err(|error| error.to_string())?;
        return window.set_focus().map_err(|error| error.to_string());
    }
    WebviewWindowBuilder::new(
        &app,
        "customize",
        WebviewUrl::App(format!("index.html?window=customize&surface={surface}").into()),
    )
    .title("Customize DevClash")
    .inner_size(1384., 990.)
    .min_inner_size(780., 650.)
    .decorations(false)
    .transparent(true)
    .shadow(!cfg!(target_os = "windows"))
    .disable_drag_drop_handler()
    .center()
    .build()
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn configure_widget(app: tauri::AppHandle, always_on_top: bool) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("widget") {
        window
            .set_always_on_top(always_on_top)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn fit_widget(app: tauri::AppHandle, width: f64, height: f64) -> Result<(), String> {
    if !width.is_finite() || !(240.0..=400.0).contains(&width) || !height.is_finite() {
        return Err("Invalid widget dimensions".into());
    }
    if let Some(window) = app.get_webview_window("widget") {
        let size = tauri::LogicalSize::new(width, height.clamp(140., 1100.));
        #[cfg(target_os = "linux")]
        {
            // Replace both bounds together so a previous larger minimum cannot
            // prevent shrinking, while user resizing still cannot change the fit.
            window
                .set_size_constraints(tauri::WindowSizeConstraints {
                    min_width: Some(tauri::LogicalUnit::new(size.width).into()),
                    min_height: Some(tauri::LogicalUnit::new(size.height).into()),
                    max_width: Some(tauri::LogicalUnit::new(size.width).into()),
                    max_height: Some(tauri::LogicalUnit::new(size.height).into()),
                })
                .map_err(|error| error.to_string())?;
        }
        window.set_size(size).map_err(|error| error.to_string())?;
        crate::window_position::ensure_visible(&window, false)?;
    }
    Ok(())
}
