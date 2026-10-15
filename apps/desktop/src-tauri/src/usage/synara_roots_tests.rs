use super::*;
use crate::{pricing::ModelPricing, usage::scan};
use serde_json::json;
use std::collections::HashMap;

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "devclash-synara-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn dir(&self, suffix: &str) -> PathBuf {
        let path = self.0.join(suffix);
        fs::create_dir_all(&path).unwrap();
        path
    }
    fn settings(&self, value: serde_json::Value) {
        let state = self.dir(".synara/userdata");
        fs::write(state.join("settings.json"), value.to_string()).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn root(provider: &'static str, path: &Path) -> Root {
    (provider, fs::canonicalize(path).unwrap())
}

#[test]
fn managed_accounts_and_channels_use_portable_home_relative_layouts() {
    let f = Fixture::new();
    let mut expected = Vec::new();
    for channel in CHANNELS {
        // Same managed .claude path: macOS sets CONFIG_DIR; Linux/Windows set HOME.
        expected.push(root(
            "claude",
            &f.dir(&format!(
                "{channel}/userdata/provider-homes/claude/instance-abc/.claude/projects"
            )),
        ));
        expected.push(root(
            "codex",
            &f.dir(&format!(
                "{channel}/codex-home-overlay/accounts/work-aabbcc/sessions"
            )),
        ));
        expected.push(root(
            "codex",
            &f.dir(&format!(
                "{channel}/runtime/codex-home-overlay/archived_sessions"
            )),
        ));
    }
    let found = discover(&f.0, None, true, true);
    assert!(found.errors.is_empty());
    assert_eq!(found.roots.len(), expected.len());
    for path in expected {
        assert!(found.roots.contains(&path));
    }
}

#[test]
fn custom_base_dev_state_and_account_settings_are_discovered() {
    let f = Fixture::new();
    let custom = f.dir("Custom Synara Home");
    let dev = f.dir("Custom Synara Home/dev/provider-homes/claude/instance-work/.claude/projects");
    let claude = f.dir("Claude Work/projects");
    let codex = f.dir("Codex Work/sessions");
    let legacy = f.dir("Codex Old/archived_sessions");
    let env_claude = f.dir("env-claude/.claude/projects");
    let env_codex = f.dir("env-codex/sessions");
    f.settings(json!({"revision": 3, "settings": {
        "providers":{"codex":{"accounts":[{"homePath":"~/Codex Old"}]}},
        "providerInstances":{
            "c":{"driver":"claudeAgent","config":{"configDir":"~/Claude Work"}},
            "x":{"driver":"codex","config":{"homePath":"~/Codex Work"}},
            "ce":{"driver":"claudeAgent","environment":[{"name":"USERPROFILE","value":f.0.join("env-claude")}]},
            "xe":{"driver":"codex","environment":[{"name":"CODEX_HOME","value":f.0.join("env-codex")}]},
            "ignored":{"driver":"cursor","config":{"homePath":"~/unrelated"}}
        },
        "unrelatedSecret":"DO_NOT_RETAIN"
    }}));
    let found = discover(&f.0, custom.to_str(), true, true);
    assert!(found.errors.is_empty());
    for (provider, path) in [
        ("claude", dev),
        ("claude", claude),
        ("codex", codex),
        ("codex", legacy),
        ("claude", env_claude),
        ("codex", env_codex),
    ] {
        assert!(
            found.roots.contains(&root(provider, &path)),
            "missing {provider}"
        );
    }
}

#[test]
fn explicit_provider_selection_remains_isolated_and_secrets_are_not_paths() {
    let f = Fixture::new();
    f.dir(".synara/userdata/provider-homes/claude/instance-a/.claude/projects");
    let codex = f.dir(".synara/codex-home-overlay/sessions");
    let secret = f.dir("secret/sessions");
    f.settings(json!({"providerInstances":{
        "redacted":{"driver":"codex","environment":[{"name":"CODEX_HOME","value":secret.parent(),"valueRedacted":true}]},
        "secret":{"driver":"codex","environment":[{"name":"API_KEY","value":secret.parent()}]}
    }}));
    assert!(discover(&f.0, None, false, false).roots.is_empty());
    assert_eq!(
        discover(&f.0, None, false, true).roots,
        vec![root("codex", &codex)]
    );
}

#[test]
fn missing_install_is_quiet_but_broken_settings_are_reported_without_contents() {
    let f = Fixture::new();
    assert!(discover(&f.0, None, true, true).errors.is_empty());
    let state = f.dir(".synara/userdata");
    fs::write(state.join("settings.json"), "PRIVATE_BROKEN_JSON").unwrap();
    let found = discover(&f.0, None, true, true);
    assert_eq!(found.errors.len(), 1);
    assert!(!found.errors[0].contains("PRIVATE"));
    fs::write(
        state.join("settings.json"),
        vec![b' '; (MAX_SETTINGS + 1) as usize],
    )
    .unwrap();
    assert_eq!(discover(&f.0, None, true, true).errors.len(), 1);
}

fn link_directory(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        // cmd's mklink does not accept the mixed separators PathBuf can retain.
        let output = std::process::Command::new("cmd")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(target.to_string_lossy().replace('/', "\\"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction fixture failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
fn codex_log(path: &Path, id: &str, total: u64) {
    let timestamp = chrono::Utc::now().to_rfc3339();
    let lines = [
        json!({"type":"session_meta","timestamp":timestamp,"payload":{"id":id}}),
        json!({"type":"turn_context","payload":{"model":"gpt-6-sol"}}),
        json!({"type":"event_msg","timestamp":timestamp,"payload":{"type":"token_count","info":{
            "total_token_usage":{"input_tokens":total,"output_tokens":0,"total_tokens":total}}}}),
    ];
    fs::write(path, lines.map(|line| line.to_string()).join("\n")).unwrap();
}
fn claude_log(path: &Path, id: &str, total: u64) {
    fs::write(path, json!({"timestamp":chrono::Utc::now().to_rfc3339(),"sessionId":"session", "requestId":id,
        "message":{"id":id,"model":"claude-opus-4-1","usage":{"input_tokens":total,"output_tokens":0},"content":"PRIVATE_PROMPT"}}).to_string()).unwrap();
}

#[test]
fn linked_roots_copied_archives_and_multiple_accounts_count_each_event_once_on_rescan() {
    let f = Fixture::new();
    let source = f.dir(".codex/sessions");
    codex_log(&source.join("native.jsonl"), "native", 100);
    for id in ["personal", "work"] {
        let overlay = f.dir(&format!(".synara/codex-home-overlay/accounts/{id}"));
        link_directory(&source, &overlay.join("sessions"));
    }
    let archive = f.dir(".synara/codex-home-overlay/archived_sessions");
    fs::copy(source.join("native.jsonl"), archive.join("copy.jsonl")).unwrap();
    codex_log(&archive.join("independent.jsonl"), "other-account", 200);
    let claude = f.dir(".synara/userdata/provider-homes/claude/instance-a/.claude/projects");
    claude_log(&claude.join("a.jsonl"), "a", 50);
    let other_claude = f.dir(".synara/userdata/provider-homes/claude/instance-b/.claude/projects");
    fs::copy(claude.join("a.jsonl"), other_claude.join("copy.jsonl")).unwrap();
    claude_log(&other_claude.join("b.jsonl"), "b", 70);
    // Nested arbitrary links must not be traversed, even within an approved root.
    let outside = f.dir("unrelated");
    codex_log(&outside.join("bad.jsonl"), "bad", 999);
    link_directory(&outside, &source.join("nested-link"));
    let found = discover(&f.0, None, true, true);
    assert!(found.errors.is_empty());
    let mut roots = found.roots;
    roots.push(("codex", source));
    let mut cache = HashMap::new();
    let pricing = ModelPricing::bundled();
    for _ in 0..2 {
        let snapshot = scan::scan_roots(&mut cache, &pricing, roots.clone(), vec![], false);
        assert!(snapshot.errors.is_empty());
        assert_eq!(snapshot.events.len(), 4);
        assert_eq!(
            snapshot
                .events
                .iter()
                .map(|event| event.tokens)
                .sum::<u64>(),
            420
        );
        assert_eq!(cache.len(), 6, "shared physical files are parsed only once");
        assert!(!serde_json::to_string(&snapshot)
            .unwrap()
            .contains("PRIVATE_PROMPT"));
    }
}

#[test]
fn relative_custom_paths_are_not_resolved_against_the_devclash_working_directory() {
    let f = Fixture::new();
    assert_eq!(expand("~", &f.0), Some(f.0.clone()));
    assert_eq!(expand("~/account", &f.0), Some(f.0.join("account")));
    assert!(expand("relative/account", &f.0).is_none());
}
