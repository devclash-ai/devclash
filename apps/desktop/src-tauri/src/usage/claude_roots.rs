//! Bounded discovery of Claude Desktop Cowork's local usage metadata folders.
//! Match OpenUsage's known account/org/session and account/org/agent/session
//! levels. Although its documented examples use local_* names, its scanner
//! accepts any nonhidden session directory with .claude/projects at those levels.
//! Never recurse into sandbox homes or follow discovery symlinks.
use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
};

pub fn discover(
    home: &Path,
    configured: &[PathBuf],
    native_defaults: bool,
) -> (Vec<PathBuf>, Vec<String>) {
    let mut roots = configured.to_vec();
    let mut errors = Vec::new();
    if !native_defaults {
        return (roots, errors);
    }
    let mut base = home.to_path_buf();
    for component in [
        "Library",
        "Application Support",
        "Claude",
        "local-agent-mode-sessions",
    ] {
        base.push(component);
        if !real_directory(&base, &mut errors) {
            return (roots, errors);
        }
    }
    let mut additional = Vec::new();
    for account in subdirectories(&base, &mut errors) {
        for organization in subdirectories(&account, &mut errors) {
            for session in subdirectories(&organization, &mut errors) {
                add_session(&session, &mut additional, &mut errors);
                if session.file_name().is_some_and(|name| name == "agent") {
                    for session in subdirectories(&session, &mut errors) {
                        add_session(&session, &mut additional, &mut errors);
                    }
                }
            }
        }
    }
    additional.sort();
    let mut seen = roots.iter().cloned().collect::<HashSet<_>>();
    roots.extend(
        additional
            .into_iter()
            .filter(|root| seen.insert(root.clone())),
    );
    (roots, errors)
}

fn report(error: io::Error, errors: &mut Vec<String>) {
    let (category, message) = match error.kind() {
        io::ErrorKind::NotFound => return,
        io::ErrorKind::PermissionDenied => (
            "permission denied",
            "Claude Cowork usage folders could not be read. Check folder permissions; other local Claude usage is still available.",
        ),
        _ => (
            "folder inspection failed",
            "Some Claude Cowork usage folders could not be inspected. Try Refresh; other local Claude usage is still available.",
        ),
    };
    if !errors.iter().any(|previous| previous == message) {
        eprintln!("claude: Cowork usage {category}");
        errors.push(message.into());
    }
}

fn real_directory(path: &Path, errors: &mut Vec<String>) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata.file_type().is_dir(),
        Err(error) => {
            report(error, errors);
            false
        }
    }
}

fn subdirectories(path: &Path, errors: &mut Vec<String>) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) => {
            report(error, errors);
            return Vec::new();
        }
    };
    let mut directories = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                report(error, errors);
                continue;
            }
        };
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => directories.push(entry.path()),
            Ok(_) => {}
            Err(error) => report(error, errors),
        }
    }
    directories.sort();
    directories
}

fn add_session(session: &Path, roots: &mut Vec<PathBuf>, errors: &mut Vec<String>) {
    let root = session.join(".claude");
    if real_directory(&root, errors) && real_directory(&root.join("projects"), errors) {
        roots.push(root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let home = std::env::temp_dir().join(format!(
                "devclash-cowork-roots-{}-{}-{}",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&home).unwrap();
            Self(home)
        }
        fn base(&self) -> PathBuf {
            self.0
                .join("Library/Application Support/Claude/local-agent-mode-sessions")
        }
        fn session(&self, suffix: &str) -> PathBuf {
            let root = self.base().join(suffix).join(".claude");
            std::fs::create_dir_all(root.join("projects")).unwrap();
            root
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn cowork_only_without_cli_logs_discovers_existing_session_projects() {
        let fixture = Fixture::new();
        let cowork = fixture.session("account/org/local_example");
        let missing_cli = fixture.0.join(".claude");
        let (roots, errors) = discover(&fixture.0, &[missing_cli.clone()], true);
        assert_eq!(roots, vec![missing_cli, cowork]);
        assert!(errors.is_empty());
    }

    #[test]
    fn cli_only_and_both_preserve_existing_configured_order_without_duplicate_roots() {
        let fixture = Fixture::new();
        let cli = fixture.0.join(".claude");
        std::fs::create_dir_all(cli.join("projects")).unwrap();
        assert_eq!(
            discover(&fixture.0, &[cli.clone()], true),
            (vec![cli.clone()], Vec::new())
        );
        let cowork = fixture.session("account/org/agent/local_example");
        assert_eq!(
            discover(&fixture.0, &[cli.clone(), cowork.clone()], true),
            (vec![cli, cowork], Vec::new())
        );
    }

    #[test]
    fn configured_custom_roots_are_isolated_from_native_cowork_discovery() {
        let fixture = Fixture::new();
        fixture.session("account/org/local_example");
        let custom = fixture.0.join("chosen-config");
        assert_eq!(
            discover(&fixture.0, &[custom.clone()], false),
            (vec![custom], Vec::new())
        );
    }

    #[test]
    fn copied_cli_and_cowork_jsonl_is_counted_once_by_the_existing_parser_and_dedup() {
        let fixture = Fixture::new();
        let cli = fixture.0.join(".claude");
        std::fs::create_dir_all(cli.join("projects")).unwrap();
        let cowork = fixture.session("account/org/local_example");
        let row = serde_json::json!({
            "timestamp": "2026-10-02T12:00:00Z", "requestId": "synthetic-request",
            "message": {"id": "synthetic-message", "model": "claude-sonnet-4-6",
                "usage": {"input_tokens": 10, "output_tokens": 5, "cache_read_input_tokens": 7}}
        })
        .to_string();
        std::fs::write(cli.join("projects/copied.jsonl"), &row).unwrap();
        std::fs::write(cowork.join("projects/copied.jsonl"), &row).unwrap();
        let (roots, errors) = discover(&fixture.0, &[cli], true);
        assert!(errors.is_empty());
        let records = roots
            .iter()
            .flat_map(|root| {
                super::super::claude::parse(&root.join("projects/copied.jsonl")).unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(records.len(), 2);
        let deduplicated = super::super::scan::dedup(records);
        assert_eq!(deduplicated.len(), 1);
        assert_eq!(deduplicated[0].event.tokens, 22);
    }

    #[test]
    fn known_levels_match_upstream_without_recursing_sandbox_homes_or_hidden_sessions() {
        let fixture = Fixture::new();
        let renamed = fixture.session("account/org/session_example");
        let direct = fixture.session("account/org/local_direct");
        let nested = fixture.session("account/org/agent/local_nested");
        fixture.session("account/org/local_direct/vm/home/private-user");
        fixture.session("account/org/local_direct/agent/local_too_deep");
        fixture.session("account/org/.hidden_session");
        std::fs::create_dir_all(
            fixture
                .base()
                .join("account/org/local_without_projects/.claude"),
        )
        .unwrap();
        let (roots, errors) = discover(&fixture.0, &[], true);
        let mut expected = vec![renamed, direct, nested];
        expected.sort();
        assert_eq!(roots, expected);
        assert!(errors.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn discovery_never_follows_session_claude_projects_or_base_symlinks() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let valid = fixture.session("account/org/local_valid");
        let org = fixture.base().join("account/org");
        symlink(org.join("local_valid"), org.join("local_linked")).unwrap();
        let linked_claude = org.join("local_linked_claude");
        std::fs::create_dir_all(&linked_claude).unwrap();
        symlink(&valid, linked_claude.join(".claude")).unwrap();
        let linked_projects = org.join("local_linked_projects/.claude");
        std::fs::create_dir_all(&linked_projects).unwrap();
        symlink(valid.join("projects"), linked_projects.join("projects")).unwrap();
        assert_eq!(discover(&fixture.0, &[], true).0, vec![valid]);
        let other_home = Fixture::new();
        symlink(fixture.0.join("Library"), other_home.0.join("Library")).unwrap();
        assert!(discover(&other_home.0, &[], true).0.is_empty());
    }

    #[test]
    fn permission_errors_are_deduplicated_and_never_expose_private_paths() {
        let mut errors = Vec::new();
        for _ in 0..2 {
            report(
                io::Error::new(io::ErrorKind::PermissionDenied, "/private/account/session"),
                &mut errors,
            );
        }
        assert_eq!(errors.len(), 1);
        assert!(!errors[0].contains("/private"));
        report(io::Error::from(io::ErrorKind::NotFound), &mut errors);
        assert_eq!(errors.len(), 1);
    }
}
