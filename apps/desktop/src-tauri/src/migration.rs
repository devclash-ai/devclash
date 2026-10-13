//! Only the app's own caches and display settings migrate. Provider credentials are never copied.
use serde_json::Value;
use std::path::{Path, PathBuf};
use tauri::Manager;

fn copy_missing(old: &Path, new: &Path) -> std::io::Result<()> {
    if !old.exists() || (new.exists() && !old.is_dir()) {
        return Ok(());
    }
    if old.is_dir() {
        std::fs::create_dir_all(new)?;
        for entry in std::fs::read_dir(old)? {
            let entry = entry?;
            copy_missing(&entry.path(), &new.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = new.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(old, new)?;
    }
    Ok(())
}
pub fn initialize(directory: &Path) -> Result<(), String> {
    if std::env::var_os("DEVCLASH_DATA_DIR")
        .or_else(|| std::env::var_os("USAGERANK_DATA_DIR"))
        .is_some()
    {
        return Ok(());
    }
    let Some(base) = directory.parent() else {
        return Ok(());
    };
    let old = base.join("app.usagerank.desktop");
    for name in ["usage-cache.json", "cloud-sync.json", "pricing"] {
        copy_missing(&old.join(name), &directory.join(name)).map_err(|_| "Your previous local settings could not be migrated. They remain in the UsageRank directory.".to_string())?;
    }
    import_legacy_display_preferences(directory, legacy_display_preferences)
}
fn import_legacy_display_preferences(
    directory: &Path,
    legacy: impl FnOnce() -> Option<Value>,
) -> Result<(), String> {
    let settings = directory.join("display-preferences.json");
    // Interrupted writes and backups are existing settings, including a saved update opt-out.
    if settings.exists()
        || settings.with_extension("tmp").exists()
        || settings.with_extension("bak").exists()
    {
        return Ok(());
    }
    if let Some(value) = legacy() {
        write_preferences(directory, &value)?;
    }
    Ok(())
}
fn legacy_display_preferences() -> Option<Value> {
    #[cfg(target_os = "macos")]
    {
        let root = dirs::home_dir()?.join("Library/WebKit/app.usagerank.desktop");
        for entry in walkdir::WalkDir::new(root)
            .max_depth(10)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.file_name() != "localstorage.sqlite3" {
                continue;
            }
            let Ok(connection) = rusqlite::Connection::open_with_flags(
                entry.path(),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            ) else {
                continue;
            };
            let raw = connection
                .query_row(
                    "SELECT value FROM ItemTable WHERE key='usagerank.preferences.v1'",
                    [],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .ok();
            if let Some(bytes) = raw {
                // WebKit stores localStorage strings as UTF-16LE; earlier versions may use UTF-8.
                let text = String::from_utf8(bytes.clone())
                    .ok()
                    .filter(|value| !value.contains('\0'))
                    .or_else(|| {
                        String::from_utf16(
                            &bytes
                                .chunks_exact(2)
                                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                                .collect::<Vec<_>>(),
                        )
                        .ok()
                    });
                if let Some(value) = text
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                    .filter(|value| value["version"] == 1)
                {
                    return Some(value);
                }
            }
        }
    }
    None
}
pub fn directory(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    std::env::var_os("DEVCLASH_DATA_DIR")
        .or_else(|| std::env::var_os("USAGERANK_DATA_DIR"))
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(|| {
            app.path()
                .app_local_data_dir()
                .map_err(|_| "Local display settings are unavailable.".into())
        })
}
fn write_preferences(directory: &Path, value: &Value) -> Result<(), String> {
    if value["version"] != 1 || !value.is_object() || value.to_string().len() > 32768 {
        return Err("Invalid display preferences.".into());
    }
    std::fs::create_dir_all(directory).map_err(|_| "Display settings could not be saved.")?;
    crate::storage::write_json(&directory.join("display-preferences.json"), value)
        .map_err(|_| "Display settings could not be saved.".into())
}
#[tauri::command]
pub fn read_preferences(app: tauri::AppHandle) -> Result<Option<Value>, String> {
    let path = directory(&app)?.join("display-preferences.json");
    if !path.exists()
        && !path.with_extension("tmp").exists()
        && !path.with_extension("bak").exists()
    {
        return Ok(None);
    }
    let value = crate::storage::read_json::<Value>(&path)
        .map_err(|_| "Your saved display settings could not be read.")?;
    Ok(Some(value))
}
#[tauri::command]
pub fn save_preferences(
    app: tauri::AppHandle,
    mut preferences: Value,
    automatic_updates: Option<bool>,
) -> Result<(), String> {
    static PREFERENCE_WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = PREFERENCE_WRITES
        .lock()
        .map_err(|_| "Display settings could not be saved.")?;
    // A stale display window must not overwrite another window's confirmed update opt-out.
    if let Some(enabled) = automatic_updates.or_else(|| crate::updates::enabled(&app)) {
        if let Some(object) = preferences.as_object_mut() {
            object.insert("autoUpdates".into(), Value::Bool(enabled));
        }
    }
    write_preferences(&directory(&app)?, &preferences)?;
    crate::updates::apply_preferences(&app, &preferences);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_import_preserves_recoverable_update_opt_out() {
        let root = std::env::temp_dir().join(format!(
            "devclash-legacy-display-recovery-{}",
            std::process::id()
        ));
        for extension in ["json", "tmp", "bak"] {
            let directory = root.join(extension);
            std::fs::create_dir_all(&directory).unwrap();
            let settings = directory.join("display-preferences.json");
            let value = serde_json::json!({"version": 1, "autoUpdates": false, "theme": "dark"});
            std::fs::write(settings.with_extension(extension), value.to_string()).unwrap();
            import_legacy_display_preferences(&directory, || {
                panic!("Existing settings must not be replaced by legacy preferences")
            })
            .unwrap();
            let restored: Value = crate::storage::read_json(&settings).unwrap();
            assert_eq!(restored["autoUpdates"], false);
            assert_eq!(restored["theme"], "dark");
        }
        let fresh = root.join("fresh");
        import_legacy_display_preferences(&fresh, || {
            Some(serde_json::json!({"version": 1, "theme": "light"}))
        })
        .unwrap();
        let imported: Value = crate::storage::read_json(&fresh.join("display-preferences.json")).unwrap();
        assert_eq!(imported["theme"], "light");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn migration_preserves_new_settings_and_leaves_originals_untouched() {
        let root = std::env::temp_dir().join(format!("devclash-migration-{}", std::process::id()));
        let old = root.join("old");
        let new = root.join("new");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("usage-cache.json"), b"original").unwrap();
        std::fs::write(new.join("usage-cache.json"), b"newer").unwrap();
        copy_missing(&old, &new).unwrap();
        assert_eq!(
            std::fs::read(new.join("usage-cache.json")).unwrap(),
            b"newer"
        );
        assert_eq!(
            std::fs::read(old.join("usage-cache.json")).unwrap(),
            b"original"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
