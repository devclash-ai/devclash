use super::*;

fn frame(code: u8, payload: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut bytes = Vec::from(*b"DCK1");
    bytes.push(code);
    bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    bytes.extend_from_slice(payload);
    Zeroizing::new(bytes)
}

fn synthetic_child(script: &str) -> Child {
    Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

#[test]
fn exact_provider_allowlist_rejects_arbitrary_services_accounts_and_suffixes() {
    for (service, account) in [
        ("Claude Safe Storage", Some("Claude Key")),
        ("Claude Code-credentials", None),
        ("Claude Code-credentials-0123abcd", None),
        ("cursor-access-token", Some("cursor-user")),
        ("Codex Auth", Some("cli|0123456789abcdef")),
    ] {
        assert!(permitted(service, account));
    }
    for (service, account) in [
        ("Claude Safe Storage", None),
        ("Claude Safe Storage", Some("other")),
        ("Claude Code-credentials", Some("")),
        ("Claude Code-credentials-0123abcd-more", None),
        ("Claude Code-credentials-0123xyz!", None),
        ("cursor-access-token", None),
        ("cursor-user", Some("cursor-user")),
        ("Codex Auth", Some("cli|0123456789abcdefmore")),
        ("Codex Auth", Some("cli|0123456789abcdez")),
        ("app.devclash.desktop.cloud", None),
    ] {
        assert!(!permitted(service, account));
    }
    // Validate before locating/spawning a helper, even when a test binary override is unset.
    assert_eq!(
        password("arbitrary-private-service", None).unwrap_err(),
        FAILURE
    );
}

#[test]
fn protocol_returns_private_bytes_and_distinguishes_missing_from_denied() {
    assert_eq!(
        decode(frame(0, b"synthetic-only"))
            .unwrap()
            .unwrap()
            .as_slice(),
        b"synthetic-only"
    );
    assert!(decode(frame(1, b"")).unwrap().is_none());
    assert_eq!(decode(frame(2, b"")).unwrap_err(), ACCESS);
    assert_eq!(decode(frame(3, b"")).unwrap_err(), INVALID);
    assert_eq!(decode(frame(4, b"")).unwrap_err(), FAILURE);
}

#[test]
fn malformed_protocol_never_echoes_private_payload_or_accepts_error_payloads() {
    let mut truncated = frame(0, b"synthetic-sensitive-marker");
    truncated.pop();
    let mut trailing = frame(0, b"synthetic-sensitive-marker");
    trailing.push(0);
    let mut bad_magic = frame(0, b"synthetic-sensitive-marker");
    bad_magic[0] = b'!';
    for value in [
        truncated,
        trailing,
        bad_magic,
        frame(2, b"synthetic-sensitive-marker"),
        frame(0, b""),
        frame(9, b""),
        Zeroizing::new(Vec::new()),
    ] {
        assert_eq!(decode(value).unwrap_err(), FAILURE);
    }
    assert!(decode(frame(0, &vec![0; MAX_SECRET])).unwrap().is_some());
    assert_eq!(
        decode(frame(0, &vec![0; MAX_SECRET + 1])).unwrap_err(),
        FAILURE
    );
}

#[test]
fn concurrent_reader_drains_more_than_a_pipe_buffer_without_deadlocking() {
    // A complete maximum-size frame; exec leaves no descendant holding the pipe open.
    let child = synthetic_child(
        "printf 'DCK1\\000\\000\\020\\000\\000'; exec /usr/bin/head -c 1048576 /dev/zero",
    );
    let bytes = collect_child(child, Duration::from_secs(3)).unwrap();
    assert_eq!(decode(bytes).unwrap().unwrap().len(), MAX_SECRET);
}

#[test]
fn oversize_output_terminates_a_helper_without_exposing_its_output() {
    let child = synthetic_child("exec /usr/bin/head -c 2097152 /dev/zero");
    let started = Instant::now();
    assert_eq!(
        collect_child(child, Duration::from_secs(3)).unwrap_err(),
        FAILURE
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn deadline_kills_and_reaps_a_stalled_child() {
    let child = synthetic_child("exec /bin/sleep 10");
    let pid = child.id();
    let started = Instant::now();
    assert_eq!(
        collect_child(child, Duration::from_millis(100)).unwrap_err(),
        TIMED_OUT
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    // With WNOHANG, -1 proves this process no longer has an unreaped child with that PID.
    extern "C" {
        fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    }
    let mut status = 0;
    assert_eq!(unsafe { waitpid(pid as i32, &mut status, 1) }, -1);
}

#[test]
fn failed_child_returns_only_a_fixed_error_and_is_reaped() {
    let child = synthetic_child("printf synthetic-sensitive-marker; exit 2");
    assert_eq!(
        collect_child(child, Duration::from_secs(2)).unwrap_err(),
        FAILURE
    );
}
