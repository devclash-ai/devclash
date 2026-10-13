//! Remember the floating widget without allowing a removed monitor to strand it off screen.
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};
use tauri::{Manager, PhysicalPosition, WebviewWindow, WindowEvent};
#[derive(Clone, Serialize, Deserialize)]
struct SavedPosition {
    x: i32,
    y: i32,
    monitor: Option<String>,
    scale: f64,
    origin_x: i32,
    origin_y: i32,
}
#[derive(Clone, Copy, Debug)]
struct Area {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}
fn clamp(position: (i32, i32), size: (u32, u32), area: Area) -> (i32, i32) {
    let min_x = area.x.saturating_add(8);
    let min_y = area.y.saturating_add(8);
    let max_x = area
        .x
        .saturating_add(area.width.saturating_sub(size.0).saturating_sub(8) as i32)
        .max(min_x);
    let max_y = area
        .y
        .saturating_add(area.height.saturating_sub(size.1).saturating_sub(8) as i32)
        .max(min_y);
    (
        position.0.clamp(min_x, max_x),
        position.1.clamp(min_y, max_y),
    )
}
fn overlaps(position: (i32, i32), size: (u32, u32), area: Area) -> bool {
    let left = position.0.max(area.x) as i64;
    let top = position.1.max(area.y) as i64;
    let right = (position.0 as i64 + size.0 as i64).min(area.x as i64 + area.width as i64);
    let bottom = (position.1 as i64 + size.1 as i64).min(area.y as i64 + area.height as i64);
    right - left >= 64 && bottom - top >= 64
}
fn area(monitor: &tauri::Monitor) -> Area {
    let work = monitor.work_area();
    Area {
        x: work.position.x,
        y: work.position.y,
        width: work.size.width,
        height: work.size.height,
    }
}
fn path(window: &WebviewWindow) -> Result<PathBuf, String> {
    Ok(crate::migration::directory(window.app_handle())?.join("widget-position.json"))
}
pub fn ensure_visible(window: &WebviewWindow, saved: bool) -> Result<(), String> {
    let monitors = window
        .available_monitors()
        .map_err(|_| "Available monitors could not be read.")?;
    if monitors.is_empty() {
        return Err("No monitor is available for the widget.".into());
    }
    let size = window
        .outer_size()
        .map_err(|_| "Widget dimensions could not be read.")?;
    let current = window
        .outer_position()
        .map_err(|_| "Widget position could not be read.")?;
    let old = if saved {
        crate::storage::read_json::<SavedPosition>(&path(window)?).ok()
    } else {
        None
    };
    let (position, preferred) = if let Some(old) = old {
        if let Some(index) = monitors
            .iter()
            .position(|monitor| monitor.name() == old.monitor.as_ref())
        {
            let monitor = &monitors[index];
            let bounds = area(monitor);
            let ratio = if old.scale.is_finite() && old.scale > 0. {
                monitor.scale_factor() / old.scale
            } else {
                1.
            };
            let x = bounds.x.saturating_add(
                ((old.x as i64 - old.origin_x as i64) as f64 * ratio).round() as i32,
            );
            let y = bounds.y.saturating_add(
                ((old.y as i64 - old.origin_y as i64) as f64 * ratio).round() as i32,
            );
            ((x, y), Some(index))
        } else {
            ((old.x, old.y), None)
        }
    } else {
        ((current.x, current.y), None)
    };
    let index = preferred.or_else(|| {
        monitors
            .iter()
            .position(|monitor| overlaps(position, (size.width, size.height), area(monitor)))
    });
    let (monitor, position) = if let Some(index) = index {
        (&monitors[index], position)
    } else {
        let primary = window.primary_monitor().ok().flatten();
        let monitor = primary
            .as_ref()
            .and_then(|primary| {
                monitors
                    .iter()
                    .find(|monitor| monitor.position() == primary.position())
            })
            .unwrap_or(&monitors[0]);
        let bounds = area(monitor);
        (
            monitor,
            (
                bounds
                    .x
                    .saturating_add(bounds.width.saturating_sub(size.width) as i32 / 2),
                bounds
                    .y
                    .saturating_add(bounds.height.saturating_sub(size.height) as i32 / 2),
            ),
        )
    };
    let (x, y) = clamp(position, (size.width, size.height), area(monitor));
    if (x, y) != (current.x, current.y) {
        window
            .set_position(PhysicalPosition::new(x, y))
            .map_err(|_| "Widget could not be moved onto the current monitor.")?;
    }
    Ok(())
}
pub fn save(window: &WebviewWindow) -> Result<(), String> {
    let position = window
        .outer_position()
        .map_err(|_| "Widget position could not be read.")?;
    let monitor = window
        .current_monitor()
        .map_err(|_| "Widget monitor could not be read.")?;
    let bounds = monitor.as_ref().map(area).unwrap_or(Area {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    });
    let value = SavedPosition {
        x: position.x,
        y: position.y,
        monitor: monitor.as_ref().and_then(|monitor| monitor.name().cloned()),
        scale: monitor.as_ref().map_or(1., tauri::Monitor::scale_factor),
        origin_x: bounds.x,
        origin_y: bounds.y,
    };
    let path = path(window)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|_| "Widget settings directory could not be created.")?;
    }
    crate::storage::write_json(&path, &value)
        .map_err(|_| "Widget position could not be saved.".into())
}
pub fn track(window: &WebviewWindow) {
    let (sender, receiver) = std::sync::mpsc::channel();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Moved(_)) {
            let _ = sender.send(());
        }
    });
    let window = window.clone();
    std::thread::spawn(move || {
        while receiver.recv().is_ok() {
            while receiver.recv_timeout(Duration::from_millis(300)).is_ok() {}
            if let Err(reason) = save(&window) {
                eprintln!("{reason}");
                let _ = window.emit("display-error", reason);
            }
        }
    });
}
use tauri::Emitter;
pub fn save_current(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("widget") {
        if let Err(reason) = save(&window) {
            eprintln!("{reason}");
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removed_monitor_and_negative_coordinates_are_clamped_to_work_area() {
        let laptop = Area {
            x: 0,
            y: 30,
            width: 1440,
            height: 870,
        };
        assert_eq!(clamp((5000, 1000), (310, 600), laptop), (1122, 292));
        assert!(!overlaps((5000, 1000), (310, 600), laptop));
        let left = Area {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert_eq!(clamp((-2100, -100), (620, 1000), left), (-1912, 8));
        assert!(overlaps((-1800, 100), (620, 700), left));
    }
    #[test]
    fn an_oversized_widget_keeps_its_header_reachable() {
        assert_eq!(
            clamp(
                (100, 500),
                (310, 1100),
                Area {
                    x: 0,
                    y: 30,
                    width: 1440,
                    height: 870
                }
            ),
            (100, 38)
        );
    }
}
