//! Read-only, bounded discovery of Synara coding-session directories.
//! Credentials are not opened; settings contribute paths only. No recursive home scan.
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};
use zeroize::{Zeroize, Zeroizing};

pub type Root = (&'static str, PathBuf);
const CHANNELS: [&str; 5] = [
    ".synara",
    ".synara-beta",
    ".synara-canary",
    ".synara-dev",
    ".synara-cua",
];
const MAX_SETTINGS: u64 = 4 * 1024 * 1024;

#[derive(Default)]
pub struct Discovery {
    pub roots: Vec<Root>,
    pub errors: Vec<String>,
}
impl Discovery {
    fn error(&mut self, operation: &str) {
        // Do not log settings contents, account labels or local paths.
        let message = format!("Synara usage {operation} could not be read. Check access to Synara's local folders or choose Usage Sources manually.");
        if !self.errors.contains(&message) {
            eprintln!("{message}");
            self.errors.push(message);
        }
    }
    fn add(&mut self, provider: &'static str, path: PathBuf) {
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => self.roots.push((provider, path)),
            Ok(_) => self.error("folder"),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => self.error("folder"),
        }
    }
    fn codex(&mut self, home: &Path) {
        for directory in ["sessions", "archived_sessions"] {
            self.add("codex", home.join(directory));
        }
    }
    fn children(&mut self, path: &Path) -> Vec<PathBuf> {
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return vec![],
            Err(_) => {
                self.error("accounts");
                return vec![];
            }
        };
        let mut children = Vec::new();
        for (index, entry) in entries.enumerate() {
            if index >= 1024 {
                self.error("accounts (too many entries)");
                break;
            }
            match entry.and_then(|entry| Ok((entry.file_type()?, entry.path()))) {
                // Only the known session root links are resolved; never recurse through account links.
                Ok((kind, path)) if kind.is_dir() => children.push(path),
                Ok(_) => {}
                Err(_) => self.error("accounts"),
            }
        }
        children.sort();
        children
    }
}

pub fn discover(home: &Path, custom_base: Option<&str>, claude: bool, codex: bool) -> Discovery {
    let mut result = Discovery::default();
    if !claude && !codex {
        return result;
    }
    let mut bases: Vec<_> = CHANNELS.iter().map(|channel| home.join(channel)).collect();
    if let Some(base) = custom_base.and_then(|base| expand(base, home)) {
        bases.push(base);
    }
    bases.sort();
    bases.dedup();
    for base in bases {
        if codex {
            for runtime in [
                base.clone(),
                base.join("runtime"),
                base.join("userdata"),
                base.join("dev"),
            ] {
                let overlay = runtime.join("codex-home-overlay");
                result.codex(&overlay);
                for account in result.children(&overlay.join("accounts")) {
                    result.codex(&account);
                }
            }
        }
        for state in [base.join("userdata"), base.join("dev")] {
            if claude {
                for account in result.children(&state.join("provider-homes/claude")) {
                    result.add("claude", account.join(".claude/projects"));
                }
            }
            read_settings(
                &state.join("settings.json"),
                home,
                claude,
                codex,
                &mut result,
            );
        }
    }
    result.roots = canonical_roots(result.roots, &mut result.errors);
    result
}

/// Resolve only selected roots, so Synara's Codex directory links/junctions work
/// without following arbitrary links inside log trees. Deduplicate before parsing.
pub fn canonical_roots(roots: Vec<Root>, errors: &mut Vec<String>) -> Vec<Root> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for (provider, root) in roots {
        match fs::canonicalize(root) {
            Ok(path) if path.is_dir() => {
                if seen.insert((provider, path.clone())) {
                    result.push((provider, path));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            _ => {
                let message = format!("Some {provider} usage folders could not be read. Check Usage Sources and local folder access.");
                eprintln!("{message}");
                if !errors.contains(&message) {
                    errors.push(message);
                }
            }
        }
    }
    result
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Settings {
    settings: Option<Box<Settings>>,
    providers: BTreeMap<String, Config>,
    provider_instances: BTreeMap<String, Instance>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Config {
    home_path: String,
    profile_dir: String,
    config_dir: String,
    shadow_home_path: String,
    accounts: Vec<Config>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Instance {
    driver: String,
    config: Config,
    environment: Vec<Environment>,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Environment {
    #[serde(default)]
    name: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    value_redacted: bool,
}
impl Drop for Environment {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

fn expand(value: &str, home: &Path) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value == "~" {
        return Some(home.into());
    }
    if let Some(suffix) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        return Some(home.join(suffix));
    }
    let path = PathBuf::from(value);
    // Relative paths depend on Synara's launch cwd. Never guess using DevClash's cwd.
    path.is_absolute().then_some(path)
}
fn read_settings(path: &Path, home: &Path, claude: bool, codex: bool, result: &mut Discovery) {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(_) => {
            result.error("settings");
            return;
        }
    };
    let mut bytes = Zeroizing::new(Vec::new());
    if file.take(MAX_SETTINGS + 1).read_to_end(&mut bytes).is_err()
        || bytes.len() as u64 > MAX_SETTINGS
    {
        result.error("settings");
        return;
    }
    let mut settings: Settings = match serde_json::from_slice(&bytes) {
        Ok(settings) => settings,
        Err(_) => {
            result.error("settings");
            return;
        }
    };
    if let Some(inner) = settings.settings.take() {
        settings = *inner;
    }
    for (driver, config) in settings.providers {
        add_config(&driver, &config, &[], home, claude, codex, result);
        if driver == "codex" {
            for account in config.accounts {
                add_config(&driver, &account, &[], home, claude, codex, result);
            }
        }
    }
    for instance in settings.provider_instances.into_values() {
        add_config(
            &instance.driver,
            &instance.config,
            &instance.environment,
            home,
            claude,
            codex,
            result,
        );
    }
}
fn add_config(
    driver: &str,
    config: &Config,
    env: &[Environment],
    home: &Path,
    claude: bool,
    codex: bool,
    result: &mut Discovery,
) {
    if driver == "codex" && codex {
        for value in [
            &config.home_path,
            &config.profile_dir,
            &config.shadow_home_path,
        ] {
            if let Some(path) = expand(value, home) {
                result.codex(&path);
            }
        }
        for variable in env
            .iter()
            .filter(|v| !v.value_redacted && v.name.eq_ignore_ascii_case("CODEX_HOME"))
        {
            if let Some(path) = expand(&variable.value, home) {
                result.codex(&path);
            }
        }
    } else if driver == "claudeAgent" && claude {
        for value in [&config.config_dir, &config.profile_dir] {
            if let Some(path) = expand(value, home) {
                result.add("claude", path.join("projects"));
            }
        }
        if let Some(path) = expand(&config.home_path, home) {
            result.add("claude", path.join(".claude/projects"));
        }
        for variable in env.iter().filter(|v| !v.value_redacted) {
            if let Some(path) = expand(&variable.value, home) {
                match variable.name.to_ascii_uppercase().as_str() {
                    "CLAUDE_CONFIG_DIR" => result.add("claude", path.join("projects")),
                    "HOME" | "USERPROFILE" => result.add("claude", path.join(".claude/projects")),
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "synara_roots_tests.rs"]
mod tests;
