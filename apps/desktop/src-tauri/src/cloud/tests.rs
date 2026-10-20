use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    thread,
};

#[test]
fn destination_session_cannot_reassign_migrated_queue_to_another_account() {
    let _serial = super::origin_migration_tests::KEYRING_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
    let directory = std::env::temp_dir().join(format!(
        "devclash-origin-collision-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let preferences = Preferences {
        origin: origins::LEGACY_API_ORIGIN.into(),
        user_id: "synthetic-owner".into(),
        sync_enabled: true,
        acknowledged: HashSet::from(["synthetic-ack".into()]),
        retry_attempts: 2,
        next_retry_at: Some("2026-10-03T12:00:00Z".into()),
        rank_period: Period::Month,
        pending: vec![records::CloudRecord {
            id: "synthetic-owner-event".into(),
            provider: "Codex".into(),
            model: "synthetic-model".into(),
            tokens: 100,
            occurred_at: "2026-10-02T12:00:00Z".into(),
            cost_usd: None,
            unpriced_tokens: 100,
            kind: "event".into(),
            source_id: None,
            revision_at: None, project_id: None,
        }],
        ..Preferences::default()
    };
    crate::storage::write_json(&directory.join("cloud-sync.json"), &preferences).unwrap();
    let cloud =
        CloudStore::load_with_origins(directory.clone(), origins::PRODUCTION_API_ORIGIN, None);
    let mut credential = Credential {
        token: "synthetic-token-not-used-for-network".into(),
        user: User {
            id: "other-synthetic-owner".into(),
            email: "other@example.test".into(),
        },
        expires_at: (Utc::now() + chrono::Duration::days(1)).to_rfc3339(),
    };
    cloud
        .entry()
        .unwrap()
        .set_password(&serde_json::to_string(&credential).unwrap())
        .unwrap();
    assert!(cloud.set_sync(true).is_err());
    let saved: Preferences = crate::storage::read_json(&directory.join("cloud-sync.json")).unwrap();
    assert_eq!(saved.user_id, "synthetic-owner");
    assert_eq!(saved.pending[0].id, "synthetic-owner-event");
    assert_eq!(saved.acknowledged, preferences.acknowledged);
    assert_eq!(saved.retry_attempts, preferences.retry_attempts);
    assert_eq!(
        saved.credential_origin.as_deref(),
        Some(origins::LEGACY_API_ORIGIN)
    );
    credential.user.id = "synthetic-owner".into();
    cloud
        .entry()
        .unwrap()
        .set_password(&serde_json::to_string(&credential).unwrap())
        .unwrap();
    assert_eq!(cloud.credential().unwrap().user.id, "synthetic-owner");
    let saved: Preferences = crate::storage::read_json(&directory.join("cloud-sync.json")).unwrap();
    assert!(saved.credential_origin.is_none());
    assert!(saved.sync_enabled);
    assert_eq!(saved.pending[0].id, "synthetic-owner-event");
    cloud.forget_session().unwrap();
    let reloaded =
        CloudStore::load_with_origins(directory.clone(), origins::PRODUCTION_API_ORIGIN, None);
    assert!(reloaded.credential().is_err());
    assert!(!reloaded.sync_enabled().unwrap());
    assert!(reloaded
        .preferences
        .lock()
        .unwrap()
        .credential_origin
        .is_none());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn loading_old_queue_discards_cursor_imports_and_keeps_account_and_other_providers() {
    let directory = std::env::temp_dir().join(format!(
        "devclash-cursor-queue-migration-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let path = directory.join("cloud-sync.json");
    std::fs::create_dir_all(&directory).unwrap();
    let origin = std::env::var("DEVCLASH_API_URL")
        .or_else(|_| std::env::var("USAGERANK_API_URL"))
        .unwrap_or_else(|_| BUNDLED_ORIGIN.into());
    let record = records::CloudRecord {
        id: "synthetic-account-day-model".into(),
        provider: "Cursor".into(),
        model: "synthetic-model".into(),
        tokens: 100,
        occurred_at: "2026-10-01T00:00:00Z".into(),
        cost_usd: None,
        unpriced_tokens: 100,
        kind: "snapshot".into(),
        source_id: Some("cursor-account-api".into()),
        revision_at: Some("2026-10-01T12:00:00Z".into()),
        project_id: None,
    };
    let mut imported = record.clone();
    imported.id = "synthetic-legacy-import".into();
    imported.source_id = Some("cursor-account-export".into());
    let mut pending = vec![record, imported];
    for provider in ["Claude", "Codex", "OpenCode"] {
        pending.push(records::CloudRecord {
            id: format!("synthetic-{provider}"),
            provider: provider.into(),
            kind: "event".into(),
            source_id: None,
            revision_at: None,
            ..pending[0].clone()
        });
    }
    let preferences = Preferences {
        origin,
        user_id: "synthetic-saved-account".into(),
        sync_enabled: true,
        acknowledged: HashSet::from(["synthetic-acknowledgement".into()]),
        pending,
        retry_attempts: 2,
        next_retry_at: Some("2026-10-01T12:05:00Z".into()),
        ..Preferences::default()
    };
    crate::storage::write_json(&path, &preferences).unwrap();
    let cloud = CloudStore::load(directory.clone(), "ai.devclash.desktop");
    let saved: Preferences = crate::storage::read_json(&path).unwrap();
    assert_eq!(cloud.status().unwrap().pending_count, 4);
    assert_eq!(saved.pending.len(), 4);
    assert!(saved.pending.iter().all(allowed_record));
    assert_eq!(saved.user_id, "synthetic-saved-account");
    assert!(saved.sync_enabled);
    assert!(!saved.session_revoked);
    assert_eq!(saved.retry_attempts, 2);
    assert_eq!(saved.acknowledged.len(), 1);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn desktop_session_stays_native_sync_needs_consent_and_revocation_stops_uploads() {
    let _serial = super::origin_migration_tests::KEYRING_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let token = "unit-test-native-session-not-a-real-token";
    let expiry = (Utc::now() + chrono::Duration::days(1)).to_rfc3339();
    let user = json!({"id": "unit-user", "email": "desktop-test@example.invalid"});
    let responses = vec![
        (
            "/api/auth/request-code",
            202,
            json!({"challengeId":"unit-challenge","expiresAt":expiry,"resendAfterSeconds":60,"message":"Code sent"}),
        ),
        (
            "/api/auth/verify-code",
            200,
            json!({"user":user,"expiresAt":expiry,"token":token}),
        ),
        ("/api/auth/me", 200, json!({"user":user,"expiresAt":expiry})),
        (
            "/api/state",
            200,
            json!({"publicProfile":false,"rankJoinedAt":null,"badges":[],"projects":[{"id":"owned-project","title":"Owned Private Project","archived":false,"moderated":false,"visible":false,"privateNote":"never-cache"}]}),
        ),
        ("/api/usage/sync", 200, json!({"accepted":1,"duplicates":0})),
        ("/api/auth/logout", 200, json!({"ok":true})),
        (
            "/api/auth/verify-code",
            200,
            json!({"user":user,"expiresAt":expiry,"token":token}),
        ),
        ("/api/auth/me", 200, json!({"user":user,"expiresAt":expiry})),
        (
            "/api/state",
            200,
            json!({"publicProfile":false,"rankJoinedAt":null,"badges":[],"projects":[]}),
        ),
        (
            "/api/usage/sync",
            401,
            json!({"error":"Your session was revoked. Sign in again.","code":"AUTH_REQUIRED"}),
        ),
        (
            "/api/auth/verify-code",
            200,
            json!({"user":{"id":"second-account","email":"other-test@example.invalid"},"expiresAt":expiry,"token":token}),
        ),
        ("/api/auth/me", 200, json!({"user":{"id":"second-account"}})),
        (
            "/api/state",
            200,
            json!({"ranked":false,"rankJoinedAt":null,"publicProfile":false,"badges":[],"projects":[]}),
        ),
    ];
    let server = thread::spawn(move || {
        for (path, status, response) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut headers = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
                headers.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            assert!(headers.lines().next().unwrap().contains(path));
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            if path == "/api/auth/verify-code" {
                let value: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(value["client"], "desktop");
                assert_eq!(value["code"], "123456");
            }
            if path == "/api/usage/sync" {
                assert!(headers.contains(token));
                let value: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(value["records"][0]["provider"], "Codex");
                if status == 200 {
                    assert_eq!(value["records"][0]["projectId"], "owned-project");
                }
                let body = String::from_utf8(bytes).unwrap();
                for private in [token, "prompt", "filepath", "message-private-id"] {
                    assert!(!body.contains(private));
                }
            }
            let body = response.to_string();
            write!(stream, "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let directory = std::env::temp_dir().join(format!(
        "devclash-cloud-native-test-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let cloud = CloudStore {
        api: Some(Api::new(&origin).unwrap()),
        app_origin: Ok(origin.clone()),
        path: directory.join("cloud-sync.json"),
        credential: Mutex::new(None),
        preferences: Mutex::new(Preferences::default()),
        status: Mutex::new(Status {
            configured: true,
            origin: Some(origin.clone()),
            ..Status::default()
        }),
        syncing: Mutex::new(()),
        browser_login: Mutex::new(None),
        keyring: Some(keyring::Entry::new(SERVICE, &origin).unwrap()), credential_service: SERVICE.into(),
    };
    assert!(cloud.status().unwrap().user.is_none());
    let challenge = cloud
        .request_code("desktop-test@example.invalid".into())
        .unwrap();
    assert!(challenge.get("code").is_none());
    let signed_in = cloud
        .verify_code("unit-challenge".into(), "123456".into())
        .unwrap();
    assert!(signed_in.user.is_some());
    assert!(!signed_in.sync_enabled);
    assert!(!signed_in.ranked);
    assert_eq!(signed_in.projects.len(), 1);
    assert_eq!(signed_in.projects[0].title, "Owned Private Project");
    assert!(!serde_json::to_string(&signed_in)
        .unwrap()
        .contains("never-cache"));
    assert_eq!(
        cloud
            .set_active_project(Some("owned-project".into()))
            .unwrap()
            .active_project_id
            .as_deref(),
        Some("owned-project")
    );
    assert!(!serde_json::to_string(&signed_in).unwrap().contains(token));
    assert!(cloud
        .entry()
        .unwrap()
        .get_password()
        .unwrap()
        .contains(token));
    let event = crate::usage::UsageEvent {
        id: "message-private-id".into(),
        provider: "codex".into(),
        model: "gpt-6-sol".into(),
        timestamp: Utc::now().to_rfc3339(),
        tokens: 100,
        cost_usd: None,
        unpriced_tokens: 100,
        cost_source: None,
        project: None,
    };
    let records = records::normalize(&[event.clone()], None);
    assert!(cloud
        .sync(records.clone())
        .unwrap()
        .last_synced_at
        .is_none());
    cloud.set_sync(true).unwrap();
    cloud.sync(records.clone()).unwrap();
    // Already acknowledged source IDs do not request another upload.
    cloud.sync(records.clone()).unwrap();
    let saved = std::fs::read_to_string(&cloud.path).unwrap();
    for private in [token, "123456", "message-private-id"] {
        assert!(!saved.contains(private));
    }
    let signed_out = cloud.logout().unwrap();
    assert!(signed_out.user.is_none());
    assert!(!signed_out.sync_enabled);
    assert!(signed_out.projects.is_empty());
    assert!(signed_out.active_project_id.is_none());
    assert!(matches!(
        cloud.entry().unwrap().get_password(),
        Err(keyring::Error::NoEntry)
    ));
    // A fresh session for the same account must retire old-origin fallback.
    {
        let mut preferences = cloud.preferences.lock().unwrap();
        preferences.credential_origin = Some(origins::LEGACY_API_ORIGIN.into());
        cloud.save(&preferences).unwrap();
    }
    cloud
        .verify_code("unit-challenge".into(), "123456".into())
        .unwrap();
    let saved: Preferences = crate::storage::read_json(&cloud.path).unwrap();
    assert!(saved.credential_origin.is_none());
    cloud.set_sync(true).unwrap();
    let next = crate::usage::UsageEvent {
        id: "second-private-id".into(),
        ..event
    };
    assert!(cloud.sync(records::normalize(&[next], None)).is_err());
    let revoked = cloud.status().unwrap();
    assert!(revoked.user.is_none());
    assert!(!revoked.sync_enabled);
    assert!(revoked.sync_error.unwrap().contains("revoked"));
    // Background refresh after revocation performs no further network requests.
    cloud.sync(records).unwrap();
    let other = cloud
        .verify_code("unit-challenge".into(), "123456".into())
        .unwrap();
    assert_eq!(other.user.unwrap().id, "second-account");
    assert!(!other.sync_enabled);
    assert_eq!(other.pending_count, 0);
    assert!(other.projects.is_empty());
    assert!(other.active_project_id.is_none());
    let preferences = cloud.preferences.lock().unwrap();
    assert_eq!(preferences.user_id, "second-account");
    assert!(preferences.acknowledged.is_empty());
    assert!(preferences.pending.is_empty());
    assert!(preferences.project_sessions.is_empty());
    assert!(preferences.cached_projects.is_empty());
    drop(preferences);
    server.join().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn unavailable_project_retries_only_affected_pending_usage_as_unassigned() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let unavailable = "11111111-1111-4111-8111-111111111111";
    let available = "22222222-2222-4222-8222-222222222222";
    let server = thread::spawn(move || {
        for (attempt, response) in [
            json!({"code":"USAGE_PROJECT_UNAVAILABLE","projectId":unavailable,"error":"Project unavailable"}),
            json!({"accepted":2,"duplicates":0}),
        ].into_iter().enumerate() {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
                if line == "\r\n" { break; }
            }
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            let sent: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(sent["records"].as_array().unwrap().len(), 2);
            let records = sent["records"].as_array().unwrap();
            let affected = records.iter().find(|record| record["tokens"] == 100).unwrap();
            let unaffected = records.iter().find(|record| record["tokens"] == 200).unwrap();
            assert_eq!(affected["projectId"], if attempt == 0 { json!(unavailable) } else { Value::Null });
            assert_eq!(unaffected["projectId"], available);
            let body = response.to_string();
            let status = if attempt == 0 { 409 } else { 200 };
            write!(stream, "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let directory = std::env::temp_dir().join(format!(
        "devclash-attribution-retry-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let expiry = (Utc::now() + chrono::Duration::days(1)).to_rfc3339();
    let record = records::CloudRecord {
        id: "measured-a".into(),
        provider: "Codex".into(),
        model: "gpt-6-sol".into(),
        tokens: 100,
        occurred_at: Utc::now().to_rfc3339(),
        cost_usd: None,
        unpriced_tokens: 100,
        kind: "event".into(),
        source_id: None,
        revision_at: None,
        project_id: Some(unavailable.into()),
    };
    let other = records::CloudRecord {
        id: "measured-b".into(),
        tokens: 200,
        unpriced_tokens: 200,
        project_id: Some(available.into()),
        ..record.clone()
    };
    let preferences = Preferences {
        origin: origin.clone(),
        user_id: "unit-user".into(),
        sync_enabled: true,
        pending: vec![record, other],
        cached_projects: vec![
            projects::OwnedProject {
                id: unavailable.into(),
                title: "Old Project".into(),
                archived: false,
                moderated: false,
            },
            projects::OwnedProject {
                id: available.into(),
                title: "Current Project".into(),
                archived: false,
                moderated: false,
            },
        ],
        project_sessions: vec![projects::ProjectSession {
            project_id: unavailable.into(),
            started_at: (Utc::now() - chrono::Duration::minutes(1)).to_rfc3339(),
            ended_at: None,
        }],
        ..Preferences::default()
    };
    let cloud = CloudStore {
        api: Some(Api::new(&origin).unwrap()),
        app_origin: Ok(origin.clone()),
        browser_login: Mutex::new(None),
        path: directory.join("cloud-sync.json"),
        credential: Mutex::new(Some(Credential {
            token: "unit-token".into(),
            user: User {
                id: "unit-user".into(),
                email: "unit@example.test".into(),
            },
            expires_at: expiry,
        })),
        preferences: Mutex::new(preferences),
        status: Mutex::new(Status::default()),
        syncing: Mutex::new(()),
        keyring: None, credential_service: SERVICE.into(),
    };
    let status = cloud.sync(vec![]).unwrap();
    assert_eq!(status.pending_count, 0);
    assert!(status.active_project_id.is_none());
    assert!(status
        .project_warning
        .as_deref()
        .unwrap()
        .contains("1 queued usage record"));
    let saved: Preferences = crate::storage::read_json(&cloud.path).unwrap();
    assert!(saved.pending.is_empty());
    assert_eq!(saved.cached_projects.len(), 1);
    assert!(saved.project_warning.is_some());
    server.join().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn offline_project_refresh_does_not_turn_into_a_persistent_sync_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        for (status, response) in [
            (503, json!({"error":"Could not reach DevClash."})),
            (
                200,
                json!({"projects":[{"id":"project-a","title":"Saved Project","archived":false,"moderated":false}]}),
            ),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
            let body = response.to_string();
            write!(stream, "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let directory = std::env::temp_dir().join(format!(
        "devclash-project-refresh-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let cloud = CloudStore {
        api: Some(Api::new(&origin).unwrap()),
        app_origin: Ok(origin.clone()),
        browser_login: Mutex::new(None),
        path: directory.join("cloud-sync.json"),
        credential: Mutex::new(Some(Credential {
            token: "unit-token".into(),
            user: User {
                id: "unit-user".into(),
                email: "unit@example.test".into(),
            },
            expires_at: (Utc::now() + chrono::Duration::days(1)).to_rfc3339(),
        })),
        preferences: Mutex::new(Preferences {
            origin: origin.clone(),
            user_id: "unit-user".into(),
            cached_projects: vec![projects::OwnedProject {
                id: "project-a".into(),
                title: "Saved Project".into(),
                archived: false,
                moderated: false,
            }],
            ..Preferences::default()
        }),
        status: Mutex::new(Status {
            projects: vec![projects::OwnedProject {
                id: "project-a".into(),
                title: "Saved Project".into(),
                archived: false,
                moderated: false,
            }],
            ..Status::default()
        }),
        syncing: Mutex::new(()),
        keyring: None, credential_service: SERVICE.into(),
    };
    assert!(cloud.refresh_projects().is_err());
    assert_eq!(cloud.status().unwrap().projects.len(), 1);
    assert!(cloud.status().unwrap().sync_error.is_none());
    cloud
        .set_error("A separate usage upload failed.".into())
        .unwrap();
    let refreshed = cloud.refresh_projects().unwrap();
    assert_eq!(refreshed.projects.len(), 1);
    assert_eq!(
        refreshed.sync_error.as_deref(),
        Some("A separate usage upload failed.")
    );
    server.join().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn offline_queue_survives_restart_and_automatic_retry_publishes_progress() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        for (status, response) in [
            (503, json!({"error":"Temporary test outage"})),
            (200, json!({"accepted":1,"duplicates":0})),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
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
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["records"][0]["tokens"], 100);
            let body = response.to_string();
            write!(stream,"HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
    });
    let directory = std::env::temp_dir().join(format!(
        "devclash-offline-{}",
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let make = |preferences| CloudStore {
        api: Some(Api::new(&origin).unwrap()),
        app_origin: Ok(origin.clone()),
        path: directory.join("cloud-sync.json"),
        credential: Mutex::new(Some(Credential {
            token: "synthetic-native-token".into(),
            user: User {
                id: "account-one".into(),
                email: "qa@example.invalid".into(),
            },
            expires_at: (Utc::now() + chrono::Duration::days(1)).to_rfc3339(),
        })),
        preferences: Mutex::new(preferences),
        status: Mutex::new(Status::default()),
        syncing: Mutex::new(()),
        browser_login: Mutex::new(None),
        keyring: None, credential_service: SERVICE.into(),
    };
    let cloud = make(Preferences {
        origin: origin.clone(),
        user_id: "account-one".into(),
        sync_enabled: true,
        ..Preferences::default()
    });
    let records = records::normalize(
        &[crate::usage::UsageEvent {
            id: "offline-event".into(),
            provider: "codex".into(),
            timestamp: Utc::now().to_rfc3339(),
            model: "gpt-5.4".into(),
            tokens: 100,
            cost_usd: None,
            unpriced_tokens: 100,
            cost_source: None,
            project: None,
        }],
        None,
    );
    assert!(cloud.sync_automatic(records.clone()).is_err());
    let failed = cloud.status().unwrap();
    assert_eq!(failed.pending_count, 1);
    assert_eq!(failed.retry_attempts, 1);
    assert!(failed.next_retry_at.is_some());
    // Neither a second automatic attempt nor restart bypasses the saved timer.
    assert_eq!(cloud.sync_automatic(records).unwrap().retry_attempts, 1);
    let restored = make(crate::storage::read_json(&cloud.path).unwrap());
    assert_eq!(restored.sync_automatic(vec![]).unwrap().pending_count, 1);
    // A successful retry publishes batch progress before the full cycle timestamp,
    // and clears the previous failure immediately rather than waiting for the backlog.
    restored.preferences.lock().unwrap().next_retry_at = Some((Utc::now() - chrono::Duration::seconds(1)).to_rfc3339());
    restored.set_error("Previous connection failure".into()).unwrap();
    let progress = Mutex::new(Vec::new());
    let synced = restored.sync_automatic_with_progress(vec![], |status| progress.lock().unwrap().push(status)).unwrap();
    let progress = progress.lock().unwrap();
    assert_eq!(progress.len(), 1);
    assert_eq!(progress[0].pending_count, 0);
    assert_eq!(progress[0].retry_attempts, 0);
    assert!(progress[0].next_retry_at.is_none());
    assert!(progress[0].sync_error.is_none());
    assert!(progress[0].last_synced_at.is_none());
    assert_eq!(synced.pending_count, 0);
    assert_eq!(synced.retry_attempts, 0);
    assert!(synced.next_retry_at.is_none());
    server.join().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
