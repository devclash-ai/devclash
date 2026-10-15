//! Saved native/WSL source paths override discovery without touching provider login files.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Sources {
    pub claude: Vec<String>,
    pub codex: Vec<String>,
    pub cursor: Vec<String>,
    pub opencode: Vec<String>,
}
fn expand(value: &str, home: &Path) -> PathBuf {
    if value == "~" {
        home.into()
    } else if let Some(suffix) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        home.join(suffix)
    } else {
        value.into()
    }
}
fn saved_sources(path: &Path) -> Result<Option<Sources>, String> {
    if !path.exists()
        && !path.with_extension("tmp").exists()
        && !path.with_extension("bak").exists()
    {
        return Ok(None);
    }
    crate::storage::read_json(path).map(Some).map_err(|_| {
        eprintln!("Saved usage source configuration is unreadable; collection is paused");
        "Your usage source settings could not be read. Open Usage Sources to save them again. Your last recorded usage is still available.".into()
    })
}
pub fn configuration_error() -> Option<String> {
    std::env::var_os("DEVCLASH_SOURCE_SETTINGS")
        .and_then(|path| saved_sources(Path::new(&path)).err())
}
pub fn configured(provider: &str) -> Vec<PathBuf> {
    configured_with_discovery(provider).0
}

// Explicit source choices remain a boundary for automatic usage discovery.
pub fn configured_with_discovery(provider: &str) -> (Vec<PathBuf>, bool) {
    let Some(home) = dirs::home_dir() else {
        return (vec![], false);
    };
    let variable = format!("DEVCLASH_{}_DIRS", provider.to_ascii_uppercase());
    if let Ok(value) = std::env::var(variable) {
        return (
            value
                .lines()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| expand(value, &home))
                .collect(),
            false,
        );
    }
    let saved = std::env::var_os("DEVCLASH_SOURCE_SETTINGS")
        .map(|path| saved_sources(Path::new(&path)))
        .transpose();
    let config = match saved {
        Ok(value) => value.flatten(),
        Err(_) => return (vec![], false),
    };
    if let Some(config) = config {
        let values = match provider {
            "claude" => config.claude,
            "codex" => config.codex,
            "cursor" => config.cursor,
            "opencode" => config.opencode,
            _ => vec![],
        };
        if !values.is_empty() {
            return (
                values.iter().map(|value| expand(value, &home)).collect(),
                false,
            );
        }
    }
    let legacy = match provider {
        "claude" => "CLAUDE_CONFIG_DIR",
        "codex" => "CODEX_HOME",
        "opencode" => "OPENCODE_DATA_DIR",
        _ => "",
    };
    if let Ok(value) = std::env::var(legacy) {
        if !value.trim().is_empty() {
            return (
                value
                    .split(',')
                    .map(str::trim)
                    .map(|value| expand(value, &home))
                    .collect(),
                false,
            );
        }
    }
    (defaults(provider, &home, std::env::consts::OS), true)
}
pub fn defaults(provider: &str, home: &Path, platform: &str) -> Vec<PathBuf> {
    let config = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| expand(&value, home))
        .unwrap_or(home.join(".config"));
    let data = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| expand(&value, home))
        .unwrap_or(home.join(".local/share"));
    let roaming = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or(home.join("AppData/Roaming"));
    platform_defaults(provider, home, platform, &config, &data, &roaming)
}
fn platform_defaults(
    provider: &str,
    home: &Path,
    platform: &str,
    config: &Path,
    data: &Path,
    roaming: &Path,
) -> Vec<PathBuf> {
    match provider {
        "claude" => vec![home.join(".claude"), config.join("claude")],
        "codex" => vec![config.join("codex"), home.join(".codex")],
        "opencode" => vec![data.join("opencode")],
        "cursor" => vec![
            match platform {
                "macos" => home.join("Library/Application Support/Cursor"),
                "windows" => roaming.join("Cursor"),
                _ => config.join("Cursor"),
            },
            home.join(".cursor"),
        ],
        _ => vec![],
    }
}
#[derive(Serialize)]
pub struct SourceSettings {
    sources: Sources,
    detected: Sources,
    platform: &'static str,
}
#[tauri::command]
pub fn read_sources(app: tauri::AppHandle) -> Result<SourceSettings, String> {
    let file = crate::migration::directory(&app)?.join("usage-sources.json");
    let sources = if file.exists() {
        crate::storage::read_json(&file)
            .map_err(|_| "Your saved usage sources could not be read.")?
    } else {
        Sources::default()
    };
    let values = |provider| {
        configured(provider)
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    };
    Ok(SourceSettings {
        sources,
        platform: std::env::consts::OS,
        detected: Sources {
            claude: values("claude"),
            codex: values("codex"),
            cursor: values("cursor"),
            opencode: values("opencode"),
        },
    })
}
#[tauri::command]
pub fn save_sources(app: tauri::AppHandle, sources: Sources) -> Result<(), String> {
    for values in [
        &sources.claude,
        &sources.codex,
        &sources.cursor,
        &sources.opencode,
    ] {
        if values.len() > 20
            || values
                .iter()
                .any(|value| value.trim().is_empty() || value.len() > 2048 || value.contains('\0'))
        {
            return Err("Choose at most twenty valid folders per provider.".into());
        }
    }
    let directory = crate::migration::directory(&app)?;
    std::fs::create_dir_all(&directory).map_err(|_| "Usage source settings could not be saved.")?;
    crate::storage::write_json(&directory.join("usage-sources.json"), &sources)
        .map_err(|_| "Usage source settings could not be saved.".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unreadable_source_configuration_does_not_fall_back_to_real_provider_roots() {
        let path =
            std::env::temp_dir().join(format!("devclash-sources-{}.json", std::process::id()));
        assert!(saved_sources(&path).unwrap().is_none());
        std::fs::write(&path, b"{broken").unwrap();
        assert!(saved_sources(&path).is_err());
        std::fs::write(
            path.with_extension("tmp"),
            br#"{"codex":["/synthetic/chosen"]}"#,
        )
        .unwrap();
        assert_eq!(
            saved_sources(&path).unwrap().unwrap().codex,
            vec!["/synthetic/chosen"]
        );
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(path.with_extension("tmp")).unwrap();
    }
    #[test]
    fn native_and_wsl_paths_are_preserved_without_unix_splitting() {
        let home = Path::new("/home/builder");
        // Native defaults are checked without inheriting the CI runner's XDG/APPDATA roots.
        let defaults = |provider, platform| {
            platform_defaults(
                provider,
                home,
                platform,
                &home.join(".config"),
                &home.join(".local/share"),
                &home.join("AppData/Roaming"),
            )
        };
        assert_eq!(
            expand("~/chosen folder/.codex", home),
            home.join("chosen folder/.codex")
        );
        let unc = "\\\\wsl.localhost\\Ubuntu\\home\\builder\\.codex";
        assert_eq!(expand(unc, home), PathBuf::from(unc));
        assert!(defaults("cursor", "windows")[0].ends_with("Cursor"));
        assert_eq!(defaults("cursor", "linux")[0], home.join(".config/Cursor"));
        assert_eq!(defaults("cursor", "macos")[1], home.join(".cursor"));
        assert!(defaults("codex", "linux").contains(&home.join(".codex")));
    }
    #[test]
    fn custom_config_and_data_roots_are_honored() {
        let home = Path::new("/home/builder");
        let config = Path::new("/chosen/config");
        let data = Path::new("/chosen/data");
        let roaming = Path::new("/chosen/roaming");
        assert_eq!(
            platform_defaults("cursor", home, "linux", config, data, roaming),
            vec![config.join("Cursor"), home.join(".cursor")]
        );
        assert_eq!(
            platform_defaults("opencode", home, "linux", config, data, roaming),
            vec![data.join("opencode")]
        );
        assert_eq!(
            platform_defaults("cursor", home, "windows", config, data, roaming),
            vec![roaming.join("Cursor"), home.join(".cursor")]
        );
    }
}
