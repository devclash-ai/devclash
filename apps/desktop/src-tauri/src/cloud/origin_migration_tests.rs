use super::*;
use keyring::credential::{CredentialApi, CredentialBuilderApi};
use std::{
    any::Any,
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, MutexGuard,
    },
};

pub(super) static KEYRING_TEST_LOCK: Mutex<()> = Mutex::new(());
type Entries = Arc<Mutex<HashMap<(String, String), SharedCredential>>>;

#[derive(Clone)]
struct SharedCredential {
    inner: Arc<keyring::mock::MockCredential>,
    fail_write: Arc<AtomicBool>,
    reads: Arc<AtomicUsize>,
    writes: Arc<AtomicUsize>,
    deletes: Arc<AtomicUsize>,
}
impl CredentialApi for SharedCredential {
    fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        if self.fail_write.swap(false, Ordering::SeqCst) {
            return Err(keyring::Error::NoStorageAccess(Box::new(
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "synthetic write failure",
                ),
            )));
        }
        self.inner.set_secret(secret)
    }
    fn get_secret(&self) -> keyring::Result<Vec<u8>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner.get_secret()
    }
    fn delete_credential(&self) -> keyring::Result<()> {
        self.deletes.fetch_add(1, Ordering::SeqCst);
        self.inner.delete_credential()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
struct SharedBuilder(Entries);
impl CredentialBuilderApi for SharedBuilder {
    fn build(
        &self,
        _: Option<&str>,
        service: &str,
        user: &str,
    ) -> keyring::Result<Box<keyring::credential::Credential>> {
        let mut entries = self.0.lock().unwrap();
        let entry = entries
            .entry((service.into(), user.into()))
            .or_insert_with(|| SharedCredential {
                inner: Arc::new(keyring::mock::MockCredential::default()),
                fail_write: Arc::new(AtomicBool::new(false)),
                reads: Arc::new(AtomicUsize::new(0)),
                writes: Arc::new(AtomicUsize::new(0)),
                deletes: Arc::new(AtomicUsize::new(0)),
            });
        Ok(Box::new(entry.clone()))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct Fixture {
    directory: PathBuf,
    entries: Entries,
    original: Preferences,
}
impl Fixture {
    fn new() -> Self {
        let entries = Arc::new(Mutex::new(HashMap::new()));
        keyring::set_default_credential_builder(Box::new(SharedBuilder(entries.clone())));
        let directory = std::env::temp_dir().join(format!(
            "devclash-origin-transfer-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap(),
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let original = Preferences {
            origin: origins::LEGACY_API_ORIGIN.into(),
            user_id: "synthetic-transfer-owner".into(),
            sync_enabled: false,
            last_synced_at: Some("2026-10-02T12:00:00Z".into()),
            acknowledged: HashSet::from(["synthetic-acknowledgement".into()]),
            retry_attempts: 3,
            next_retry_at: Some("2026-10-03T12:00:00Z".into()),
            rank_period: Period::Month,
            pending: vec![records::CloudRecord {
                id: "synthetic-pending-event".into(),
                provider: "Claude".into(),
                model: "synthetic-model".into(),
                tokens: 100,
                occurred_at: "2026-10-02T12:01:00Z".into(),
                cost_usd: None,
                unpriced_tokens: 100,
                kind: "event".into(),
                source_id: None,
                revision_at: None, project_id: None,
            }],
            ..Preferences::default()
        };
        crate::storage::write_json(&directory.join("cloud-sync.json"), &original).unwrap();
        let credential = Credential {
            token: "synthetic-origin-transfer-session".into(),
            user: User {
                id: original.user_id.clone(),
                email: "qa-transfer@example.test".into(),
            },
            expires_at: (Utc::now() + chrono::Duration::days(1)).to_rfc3339(),
        };
        Self::entry(origins::LEGACY_API_ORIGIN)
            .set_password(&serde_json::to_string(&credential).unwrap())
            .unwrap();
        Self {
            directory,
            entries,
            original,
        }
    }
    fn entry(origin: &str) -> keyring::Entry {
        keyring::Entry::new(SERVICE, origin).unwrap()
    }
    fn load(&self) -> CloudStore {
        // Construction/credential reads never perform HTTP requests.
        CloudStore::load_with_origins(self.directory.clone(), origins::PRODUCTION_API_ORIGIN, None)
    }
    fn saved(&self) -> Preferences {
        crate::storage::read_json(&self.directory.join("cloud-sync.json")).unwrap()
    }
    fn assert_state_preserved(&self, preferences: &Preferences) {
        let mut before = serde_json::to_value(&self.original).unwrap();
        let mut after = serde_json::to_value(preferences).unwrap();
        for value in [&mut before, &mut after] {
            value.as_object_mut().unwrap().remove("origin");
            value.as_object_mut().unwrap().remove("credentialOrigin");
        }
        assert_eq!(before, after);
    }
    fn fail_next_destination_write(&self) {
        Self::entry(origins::PRODUCTION_API_ORIGIN);
        self.entries.lock().unwrap()[&(SERVICE.into(), origins::PRODUCTION_API_ORIGIN.into())]
            .fail_write
            .store(true, Ordering::SeqCst);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}
fn serial() -> MutexGuard<'static, ()> {
    KEYRING_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

#[test]
fn secure_origin_transfer_and_reload_preserve_opt_out_and_durable_queue() {
    let _serial = serial();
    let fixture = Fixture::new();
    let old = Fixture::entry(origins::LEGACY_API_ORIGIN)
        .get_password()
        .unwrap();
    let cloud = fixture.load();
    assert!(matches!(
        Fixture::entry(origins::PRODUCTION_API_ORIGIN).get_password(),
        Err(keyring::Error::NoEntry)
    ));
    assert_eq!(
        cloud.credential().unwrap().user.id,
        fixture.original.user_id
    );
    assert_eq!(
        Fixture::entry(origins::PRODUCTION_API_ORIGIN)
            .get_password()
            .unwrap(),
        old
    );
    assert!(!cloud.sync_enabled().unwrap());
    let saved = fixture.saved();
    fixture.assert_state_preserved(&saved);
    assert_eq!(saved.origin, origins::PRODUCTION_API_ORIGIN);
    assert!(saved.credential_origin.is_none());
    let on_disk = std::fs::read_to_string(&cloud.path).unwrap();
    assert!(!on_disk.contains("synthetic-origin-transfer-session"));
    drop(cloud);
    let reloaded = fixture.load();
    assert_eq!(
        reloaded.credential().unwrap().user.id,
        fixture.original.user_id
    );
    fixture.assert_state_preserved(&reloaded.preferences.lock().unwrap());
    assert!(!reloaded.sync_enabled().unwrap());
}

#[test]
fn secure_origin_write_failure_keeps_old_entry_and_allows_safe_retry() {
    let _serial = serial();
    let fixture = Fixture::new();
    let old = Fixture::entry(origins::LEGACY_API_ORIGIN)
        .get_password()
        .unwrap();
    let cloud = fixture.load();
    fixture.fail_next_destination_write();
    assert!(cloud
        .credential()
        .err()
        .unwrap()
        .contains("OS credential store"));
    assert!(matches!(
        Fixture::entry(origins::PRODUCTION_API_ORIGIN).get_password(),
        Err(keyring::Error::NoEntry)
    ));
    assert_eq!(
        Fixture::entry(origins::LEGACY_API_ORIGIN)
            .get_password()
            .unwrap(),
        old
    );
    fixture.assert_state_preserved(&fixture.saved());
    assert_eq!(
        fixture.saved().credential_origin.as_deref(),
        Some(origins::LEGACY_API_ORIGIN)
    );
    assert!(cloud.credential().is_ok());
    assert!(fixture.saved().credential_origin.is_none());
    fixture.assert_state_preserved(&fixture.saved());
}

#[test]
fn preference_save_failure_retains_fallback_marker_across_reload_until_safe_retry() {
    let _serial = serial();
    let fixture = Fixture::new();
    let blocked = fixture.directory.join("cloud-sync.tmp");
    std::fs::create_dir(&blocked).unwrap();
    let cloud = fixture.load();
    assert!(cloud.status().unwrap().sync_error.is_some());
    assert!(cloud
        .credential()
        .err()
        .unwrap()
        .contains("settings could not be saved"));
    fixture.assert_state_preserved(&cloud.preferences.lock().unwrap());
    assert_eq!(
        cloud
            .preferences
            .lock()
            .unwrap()
            .credential_origin
            .as_deref(),
        Some(origins::LEGACY_API_ORIGIN)
    );
    assert!(Fixture::entry(origins::PRODUCTION_API_ORIGIN)
        .get_password()
        .is_ok());
    fixture.assert_state_preserved(&fixture.saved());
    drop(cloud);
    let reloaded = fixture.load();
    assert!(reloaded.credential().is_err());
    assert_eq!(
        reloaded
            .preferences
            .lock()
            .unwrap()
            .credential_origin
            .as_deref(),
        Some(origins::LEGACY_API_ORIGIN)
    );
    std::fs::remove_dir(&blocked).unwrap();
    assert!(reloaded.credential().is_ok());
    fixture.assert_state_preserved(&fixture.saved());
    assert!(fixture.saved().credential_origin.is_none());
    drop(reloaded);
    assert!(fixture.load().credential().is_ok());
}

fn response_server(status: u16) -> (Api, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let api = Api::new(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        loop {
            let mut line = String::new();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
        }
        let body = if status == 401 {
            r#"{"error":"Synthetic session revoked"}"#
        } else {
            r#"{"ok":true}"#
        };
        write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    (api, server)
}

#[test]
fn logout_and_revocation_cannot_revive_old_origin_entry_after_reload() {
    let _serial = serial();
    for revoked in [false, true] {
        let fixture = Fixture::new();
        let mut cloud = fixture.load();
        let credential = cloud.credential().unwrap();
        let (api, server) = response_server(if revoked { 401 } else { 200 });
        // All request paths below use only this owned loopback responder.
        cloud.api = Some(api);
        if revoked {
            assert!(cloud
                .request("GET", "/api/auth/me", Some(&credential.token), None)
                .is_err());
        } else {
            assert!(cloud.logout().unwrap().user.is_none());
        }
        server.join().unwrap();
        assert!(Fixture::entry(origins::LEGACY_API_ORIGIN)
            .get_password()
            .is_ok());
        assert!(matches!(
            Fixture::entry(origins::PRODUCTION_API_ORIGIN).get_password(),
            Err(keyring::Error::NoEntry)
        ));
        let saved = fixture.saved();
        assert!(saved.session_revoked);
        assert!(!saved.sync_enabled);
        assert!(saved.credential_origin.is_none());
        drop(cloud);
        let reloaded = fixture.load();
        assert!(reloaded.credential().is_err());
        assert!(!reloaded.sync_enabled().unwrap());
        assert!(reloaded.preferences.lock().unwrap().pending.is_empty());
    }
}

#[test]
fn same_origin_upgrade_reload_preserves_login_consent_and_queue_after_server_failure() {
    let _serial = serial();
    for enabled in [false, true] {
        let fixture = Fixture::new();
        let cloud = fixture.load();
        let credential = cloud.credential().unwrap();
        cloud.set_sync(enabled).unwrap();
        let before = serde_json::to_value(fixture.saved()).unwrap();
        drop(cloud);

        // Replacing the executable constructs a new store against the same
        // persistent directory and OS credential entry, independent of version.
        let mut upgraded = fixture.load();
        assert_eq!(upgraded.credential().unwrap().token, credential.token);
        assert_eq!(upgraded.sync_enabled().unwrap(), enabled);
        let (api, server) = response_server(503);
        upgraded.api = Some(api);
        assert!(upgraded.refresh_account().is_err());
        server.join().unwrap();
        let status = upgraded.status().unwrap();
        assert_eq!(status.user.unwrap().id, credential.user.id);
        assert_eq!(status.sync_enabled, enabled);
        assert_eq!(serde_json::to_value(fixture.saved()).unwrap(), before);
        drop(upgraded);

        let reloaded = fixture.load();
        assert_eq!(reloaded.credential().unwrap().token, credential.token);
        assert_eq!(reloaded.sync_enabled().unwrap(), enabled);
        assert!(!fixture.saved().session_revoked);
    }
}


#[test]
fn qa_never_reads_writes_or_deletes_production_credentials_even_with_copied_preferences() {
    let _serial = serial();
    let f = Fixture::new();
    let encoded = Fixture::entry(origins::LEGACY_API_ORIGIN).get_password().unwrap();
    Fixture::entry(origins::PRODUCTION_API_ORIGIN).set_password(&encoded).unwrap();
    let production_calls = || {
        let entries = f.entries.lock().unwrap();
        let mut values = entries.iter().filter(|((service, _), _)| !service.starts_with("ai.devclash.desktop.qa.cloud."))
            .map(|(key, value)| (key.clone(), value.reads.load(Ordering::SeqCst), value.writes.load(Ordering::SeqCst), value.deletes.load(Ordering::SeqCst)))
            .collect::<Vec<_>>();
        values.sort(); values
    };
    let before = production_calls();
    let service = identity::service(&f.directory, "ai.devclash.diagnostics.qa", false);
    let load = || CloudStore::load_scoped(f.directory.clone(), origins::PRODUCTION_API_ORIGIN, None, service.clone());
    let qa = load();
    assert!(qa.credential().is_err());
    assert_eq!(production_calls(), before);
    // Explicit QA sign-in survives a new process/store and QA sign-out touches only QA.
    qa.entry().unwrap().set_password(&encoded).unwrap();
    drop(qa);
    let restarted = load();
    assert_eq!(restarted.credential().unwrap().token, "synthetic-origin-transfer-session");
    restarted.forget_session().unwrap();
    assert_eq!(production_calls(), before);
    assert_eq!(Fixture::entry(origins::PRODUCTION_API_ORIGIN).get_password().unwrap(), encoded);
}

#[test]
fn release_reload_preserves_session_sync_preferences_and_does_not_rewrite_keychain_acl() {
    let _serial = serial();
    let f = Fixture::new();
    let first = f.load();
    let credential = first.credential().unwrap();
    let preferences = serde_json::to_value(f.saved()).unwrap();
    let entry = f.entries.lock().unwrap()[&(SERVICE.into(), origins::PRODUCTION_API_ORIGIN.into())].clone();
    let writes = entry.writes.load(Ordering::SeqCst);
    let deletes = entry.deletes.load(Ordering::SeqCst);
    drop(first);
    let upgraded = f.load();
    assert_eq!(upgraded.credential().unwrap().token, credential.token);
    assert_eq!(serde_json::to_value(f.saved()).unwrap(), preferences);
    assert_eq!(entry.writes.load(Ordering::SeqCst), writes);
    assert_eq!(entry.deletes.load(Ordering::SeqCst), deletes);
}
