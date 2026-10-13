//! Synthetic macOS Desktop fixtures. Never read the real Keychain or provider files.
use super::*;
use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

const ACCOUNT: &str = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";
const FOREIGN: &str = "ffffffff-ffff-4fff-8fff-ffffffffffff";
const ORGANIZATION: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const KEY: [u8; 16] = [42; 16];
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

fn encrypted(plaintext: &[u8]) -> Vec<u8> {
    let mut value = b"v10".to_vec();
    value.extend(
        cbc::Encryptor::<aes::Aes128>::new_from_slices(&KEY, &[b' '; 16])
            .unwrap()
            .encrypt_padded_vec_mut::<Pkcs7>(plaintext),
    );
    value
}
fn cache_key(account: &str) -> String {
    format!("acct:{account}|9d1c250a-e61b-44d9-88ed-5944d1962f5e:{ORGANIZATION}:https://api.anthropic.com:user:profile user:inference")
}
fn token(expired: bool) -> Value {
    json!({
        "token":"synthetic-desktop-access",
        "refreshToken":"never-borrow-this-synthetic-refresh",
        "expiresAt":chrono::Utc::now().timestamp_millis()+if expired {-1} else {3_600_000}
    })
}

struct Fixture {
    root: PathBuf,
    config: Value,
}
impl Fixture {
    fn new(entry: Value, account: Option<&str>) -> Self {
        let root = std::env::temp_dir().join(format!(
            "devclash-mac-desktop-auth-{}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("Network")).unwrap();
        let cache = json!({cache_key(ACCOUNT):entry});
        let mut config = json!({
            "oauth:tokenCacheV2":STANDARD.encode(encrypted(&serde_json::to_vec(&cache).unwrap()))
        });
        if let Some(account) = account {
            config["lastKnownAccountUuid"] = json!(account);
        }
        let fixture = Self { root, config };
        fixture.save_config();
        let db = fixture.database();
        db.execute_batch("CREATE TABLE cookies(host_key TEXT,name TEXT,value TEXT,encrypted_value BLOB,last_update_utc INTEGER); CREATE TABLE meta(key TEXT,value TEXT); INSERT INTO meta VALUES('version','24');").unwrap();
        let mut clear = Sha256::digest(b".claude.ai").to_vec();
        clear.extend(ORGANIZATION.as_bytes());
        db.execute(
            "INSERT INTO cookies VALUES('.claude.ai','lastActiveOrg','',?1,100)",
            [encrypted(&clear)],
        )
        .unwrap();
        fixture
    }
    fn save_config(&self) {
        std::fs::write(
            self.root.join("config.json"),
            serde_json::to_vec(&self.config).unwrap(),
        )
        .unwrap();
    }
    fn database(&self) -> Connection {
        Connection::open(self.root.join("Network/Cookies")).unwrap()
    }
    fn load(&self) -> Result<Option<super::super::auth::Credentials>, String> {
        load_with_key(&self.root, self.config.clone(), &KEY, mac_crypto::decrypt)
    }
    fn load_error(&self) -> String {
        self.load().err().expect("Fixture must reject credentials")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn mac_app_only_login_reads_scoped_access_token_and_host_bound_cookie() {
    let fixture = Fixture::new(token(false), Some(ACCOUNT));
    let config = signed_in_config(&fixture.root).unwrap().unwrap();
    let credentials = load_with_key(&fixture.root, config, &KEY, mac_crypto::decrypt)
        .unwrap()
        .unwrap();
    assert_eq!(credentials.token, "synthetic-desktop-access");
    assert!(credentials.account.is_none());
}

#[test]
fn mac_expired_missing_account_and_foreign_account_logins_are_rejected() {
    assert!(Fixture::new(token(true), Some(ACCOUNT))
        .load_error()
        .contains("stale"));
    assert_eq!(Fixture::new(token(false), None).load_error(), UNREADABLE);
    assert!(Fixture::new(token(false), Some(FOREIGN))
        .load_error()
        .contains("No usable"));
}

#[test]
fn mac_scoped_v2_tombstone_does_not_revive_scoped_or_legacy_v1_tokens() {
    let mut fixture = Fixture::new(Value::Null, Some(ACCOUNT));
    let legacy = cache_key(ACCOUNT).split_once('|').unwrap().1.to_owned();
    let old_cache = json!({cache_key(ACCOUNT):token(false),legacy:token(false)});
    fixture.config["oauth:tokenCache"] =
        json!(STANDARD.encode(encrypted(&serde_json::to_vec(&old_cache).unwrap())));
    fixture.save_config();
    assert!(fixture.load_error().contains("No usable"));
}

#[test]
fn mac_logout_with_retained_cache_stops_before_key_loading_and_cannot_fallback() {
    let fixture = Fixture::new(token(false), Some(ACCOUNT));
    fixture
        .database()
        .execute("DELETE FROM cookies", [])
        .unwrap();
    // This preflight runs before the production Keychain call in load().
    assert!(signed_in_config(&fixture.root).unwrap().is_none());
    assert!(fixture.load_error().contains("No active"));
    std::fs::remove_file(fixture.root.join("Network/Cookies")).unwrap();
    assert!(signed_in_config(&fixture.root).unwrap().is_none());
}

#[test]
fn mac_account_or_token_cache_changed_after_snapshot_cannot_return_old_token() {
    for change_account in [true, false] {
        let fixture = Fixture::new(token(false), Some(ACCOUNT));
        let snapshot = signed_in_config(&fixture.root).unwrap().unwrap();
        let mut changed = snapshot.clone();
        if change_account {
            changed["lastKnownAccountUuid"] = json!(FOREIGN);
        } else {
            changed["oauth:tokenCacheV2"] = json!("changed-synthetic-cache");
        }
        std::fs::write(
            fixture.root.join("config.json"),
            serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();
        assert!(
            load_with_key(&fixture.root, snapshot, &KEY, mac_crypto::decrypt)
                .err()
                .unwrap()
                .contains("changed during Refresh")
        );
    }
}

#[test]
fn mac_invalid_cookie_hash_is_not_replaced_by_unique_scoped_organization() {
    let fixture = Fixture::new(token(false), Some(ACCOUNT));
    let mut wrong_host = Sha256::digest(b"foreign.example").to_vec();
    wrong_host.extend(ORGANIZATION.as_bytes());
    fixture
        .database()
        .execute(
            "UPDATE cookies SET encrypted_value=?1",
            [encrypted(&wrong_host)],
        )
        .unwrap();
    assert_eq!(fixture.load_error(), UNREADABLE);
}
