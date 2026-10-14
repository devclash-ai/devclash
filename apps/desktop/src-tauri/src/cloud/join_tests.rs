use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    thread,
};

#[test]
fn pending_join_is_completed_only_by_server_confirmation_and_never_enables_uploads() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        for (path, body) in [
            ("/api/auth/me", json!({"user":{"id":"join-account"}})),
            (
                "/api/state",
                json!({"ranked":false,"publicProfile":false,"projects":[],"badges":[]}),
            ),
            ("/api/auth/me", json!({"user":{"id":"join-account"}})),
            (
                "/api/state",
                json!({"ranked":true,"publicProfile":false,"projects":[],"badges":[]}),
            ),
            (
                "/api/rank?period=Today",
                json!({"own":null,"generatedAt":Utc::now().to_rfc3339()}),
            ),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            assert!(request.starts_with(&format!("GET {path} ")), "{request}");
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
            let body = body.to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let directory = std::env::temp_dir().join(format!(
        "devclash-join-server-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let cloud = CloudStore::load_with_origins(directory.clone(), &origin, None);
    *cloud.credential.lock().unwrap() = Some(Credential {
        token: "synthetic-native-session".into(),
        user: User {
            id: "join-account".into(),
            email: "join@example.invalid".into(),
        },
        expires_at: (Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
    });
    cloud.preferences.lock().unwrap().user_id = "join-account".into();
    cloud
        .mark_join_pending(false, None, Some("join-account"))
        .unwrap();
    let pending = cloud.refresh_account().unwrap();
    assert!(pending.join_pending);
    assert!(!pending.ranked);
    assert!(!pending.sync_enabled);
    let joined = cloud.refresh_account().unwrap();
    assert!(!joined.join_pending);
    assert!(joined.ranked);
    assert!(joined.rank.is_none());
    assert!(!joined.sync_enabled);
    assert!(!joined.public_profile);
    let saved: Preferences = crate::storage::read_json(&directory.join("cloud-sync.json")).unwrap();
    assert!(!saved.join_pending);
    assert!(!saved.sync_enabled);
    server.join().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

const RECEIPT: &str = "01234567-1234-1234-1234-0123456789ab";

#[test]
fn sync_consent_rejects_unconfirmed_unrelated_and_other_account_joins() {
    let consent = || Preferences {
        user_id: "native-account".into(),
        join_account_id: Some("native-account".into()),
        join_pending: true,
        enable_sync_on_join: true,
        join_receipt: Some(RECEIPT.into()),
        ..Preferences::default()
    };
    for (state, account) in [
        (
            json!({"ranked":false,"desktopJoinReceipt":RECEIPT}),
            "native-account",
        ),
        (
            json!({"rankJoinedAt":"2026-10-14T00:00:00Z","desktopJoinReceipt":RECEIPT}),
            "native-account",
        ),
        (json!({"ranked":true}), "native-account"),
        (
            json!({"ranked":true,"desktopJoinReceipt":"other-receipt"}),
            "native-account",
        ),
        (
            json!({"ranked":true,"desktopJoinReceipt":RECEIPT}),
            "other-account",
        ),
    ] {
        let mut preferences = consent();
        confirm_join(&mut preferences, &state, account);
        assert!(!preferences.sync_enabled);
        assert!(!preferences.join_sync_activation_ready);
        assert!(!preferences.public_profile);
    }
    let mut no_consent = consent();
    no_consent.enable_sync_on_join = false;
    confirm_join(
        &mut no_consent,
        &json!({"ranked":true,"desktopJoinReceipt":RECEIPT}),
        "native-account",
    );
    assert!(!no_consent.sync_enabled);
    let mut ordinary = consent();
    ordinary.join_pending = false;
    confirm_join(
        &mut ordinary,
        &json!({"ranked":true,"desktopJoinReceipt":RECEIPT}),
        "native-account",
    );
    assert!(!ordinary.sync_enabled);
}

#[test]
fn confirmed_join_consent_survives_restart_and_dispatches_real_metadata_sync_once() {
    use std::io::Read;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        for (method, path, body) in [
            ("GET", "/api/auth/me", json!({"user":{"id":"join-account"}})),
            (
                "GET",
                "/api/state",
                json!({"ranked":true,"desktopJoinReceipt":"old-receipt","publicProfile":false,"projects":[],"badges":[]}),
            ),
            (
                "GET",
                "/api/rank?period=Today",
                json!({"own":null,"generatedAt":Utc::now().to_rfc3339()}),
            ),
            ("GET", "/api/auth/me", json!({"user":{"id":"join-account"}})),
            (
                "GET",
                "/api/state",
                json!({"ranked":true,"desktopJoinReceipt":"newer-device-receipt","desktopJoinReceipts":[RECEIPT,"newer-device-receipt"],"publicProfile":false,"projects":[],"badges":[]}),
            ),
            (
                "GET",
                "/api/rank?period=Today",
                json!({"own":null,"generatedAt":Utc::now().to_rfc3339()}),
            ),
            (
                "POST",
                "/api/usage/sync",
                json!({"accepted":1,"duplicates":0}),
            ),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            assert!(
                request.starts_with(&format!("{method} {path} ")),
                "{request}"
            );
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
                if line == "\r\n" {
                    break;
                }
            }
            if method == "POST" {
                let mut data = vec![0; length];
                reader.read_exact(&mut data).unwrap();
                let record: Value = serde_json::from_slice(&data).unwrap();
                assert_eq!(record["records"][0]["tokens"], 25);
                assert_eq!(record["records"][0]["provider"], "Codex");
                let encoded = String::from_utf8(data).unwrap();
                assert!(!encoded.contains("synthetic-native-session"));
                assert!(!encoded.contains("prompt"));
            }
            let body = body.to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let directory = std::env::temp_dir().join(format!(
        "devclash-join-sync-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let credential = Credential {
        token: "synthetic-native-session".into(),
        user: User {
            id: "join-account".into(),
            email: "join@example.invalid".into(),
        },
        expires_at: (Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
    };
    let cloud = CloudStore::load_with_origins(directory.clone(), &origin, None);
    cloud.preferences.lock().unwrap().user_id = "join-account".into();
    *cloud.credential.lock().unwrap() = Some(credential.clone());
    cloud
        .mark_join_pending(true, Some(RECEIPT), Some("join-account"))
        .unwrap();
    let already_ranked = cloud.refresh_account().unwrap();
    assert!(already_ranked.ranked);
    assert!(already_ranked.join_pending);
    assert!(cloud.preferences.lock().unwrap().enable_sync_on_join);
    assert!(!already_ranked.sync_enabled);
    cloud
        .dispatch_join_sync(|| panic!("Unconfirmed join must not start sync"))
        .unwrap();
    drop(cloud);
    let cloud = CloudStore::load_with_origins(directory.clone(), &origin, None);
    assert!(cloud.status().unwrap().join_pending);
    assert!(cloud.preferences.lock().unwrap().enable_sync_on_join);
    assert!(!cloud.preferences.lock().unwrap().sync_enabled);
    *cloud.credential.lock().unwrap() = Some(credential);
    let joined = cloud.refresh_account().unwrap();
    assert!(joined.sync_enabled);
    assert!(!joined.join_pending);
    assert!(!joined.public_profile);
    assert!(joined.rank.is_none());
    let starts = AtomicUsize::new(0);
    cloud
        .dispatch_join_sync(|| {
            starts.fetch_add(1, Ordering::SeqCst);
            let record = records::normalize(
                &[crate::usage::UsageEvent {
                    id: "synthetic-eligible-event".into(),
                    provider: "codex".into(),
                    model: "synthetic-model".into(),
                    timestamp: Utc::now().to_rfc3339(),
                    tokens: 25,
                    cost_usd: None,
                    unpriced_tokens: 25,
                    cost_source: None,
                    project: None,
                }],
                None,
            );
            assert!(cloud
                .sync_automatic(record)
                .unwrap()
                .last_synced_at
                .is_some());
        })
        .unwrap();
    cloud
        .dispatch_join_sync(|| {
            starts.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    let saved: Preferences = crate::storage::read_json(&directory.join("cloud-sync.json")).unwrap();
    assert!(saved.sync_enabled);
    assert!(!saved.join_sync_activation_ready);
    assert!(!saved.enable_sync_on_join);
    server.join().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn existing_membership_keeps_consent_until_its_later_receipt_is_confirmed() {
    let mut preferences = Preferences {
        user_id: "native-account".into(),
        join_account_id: Some("native-account".into()),
        join_pending: true,
        enable_sync_on_join: true,
        join_receipt: Some(RECEIPT.into()),
        ..Preferences::default()
    };
    for state in [
        json!({"ranked":true}),
        json!({"ranked":true,"desktopJoinReceipt":"old-receipt"}),
    ] {
        confirm_join(&mut preferences, &state, "native-account");
        assert!(preferences.join_pending);
        assert!(preferences.enable_sync_on_join);
        assert!(!preferences.sync_enabled);
        assert!(!preferences.join_sync_activation_ready);
    }
    confirm_join(
        &mut preferences,
        &json!({"ranked":true,"desktopJoinReceipt":RECEIPT}),
        "native-account",
    );
    assert!(preferences.sync_enabled);
    assert!(preferences.join_sync_activation_ready);
    assert!(!preferences.join_pending);
    assert!(!preferences.enable_sync_on_join);
    assert!(!preferences.public_profile);
}
