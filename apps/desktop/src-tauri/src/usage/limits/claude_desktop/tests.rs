use super::*;
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use serde_json::json;

// Existing fixtures exercise Windows' authenticated format on every host.
fn decode_cache(config: &Value, field: &str, key: &[u8]) -> Result<Option<Value>, String> {
    super::decode_cache(config, field, key, crypto::decrypt)
}
fn active_org(root: &Path, key: &[u8]) -> Result<Option<String>, CookieError> {
    super::active_org(root, key, crypto::decrypt)
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "devclash-desktop-auth-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn encrypted(clear: &[u8], key: &[u8]) -> Vec<u8> {
    let nonce = [7; 12];
    let mut value = b"v10".to_vec();
    value.extend(nonce);
    value.extend(
        Aes256Gcm::new_from_slice(key)
            .unwrap()
            .encrypt(Nonce::from_slice(&nonce), clear)
            .unwrap(),
    );
    value
}

#[test]
fn finds_native_and_msix_claude_data_without_scanning_other_apps() {
    let fixture = Fixture::new();
    let local = fixture.0.join("Local");
    let roaming = fixture.0.join("Roaming");
    std::fs::create_dir_all(local.join("Packages/Claude_test/LocalCache/Roaming/Claude")).unwrap();
    std::fs::create_dir_all(local.join("Packages/Other_test")).unwrap();
    assert_eq!(
        roots(&roaming, &local),
        vec![
            roaming.join("Claude"),
            local.join("Packages/Claude_test/LocalCache/Roaming/Claude")
        ]
    );
}

#[test]
fn bounded_reader_rejects_large_and_malformed_config() {
    let fixture = Fixture::new();
    let path = fixture.0.join("config.json");
    std::fs::write(&path, b"{\"lastKnownAccountUuid\":null}").unwrap();
    assert!(read_json(&path).is_ok());
    std::fs::write(&path, vec![b' '; LIMIT + 1]).unwrap();
    assert_eq!(read_json(&path).unwrap_err(), UNREADABLE);
    std::fs::write(&path, b"invalid-private-content").unwrap();
    assert_eq!(read_json(&path).unwrap_err(), UNREADABLE);
}

#[test]
fn decodes_only_authenticated_bounded_json_caches() {
    let key = [4; 32];
    let clear = br#"{"fixture":{"token":"synthetic"}}"#;
    let value = json!({"oauth:tokenCacheV2": STANDARD.encode(encrypted(clear, &key))});
    assert_eq!(
        decode_cache(&value, "oauth:tokenCacheV2", &key)
            .unwrap()
            .unwrap()["fixture"]["token"],
        "synthetic"
    );
    assert!(decode_cache(&value, "oauth:tokenCache", &key)
        .unwrap()
        .is_none());
    assert_eq!(
        decode_cache(
            &json!({"oauth:tokenCacheV2":"invalid-base64-private"}),
            "oauth:tokenCacheV2",
            &key
        )
        .unwrap_err(),
        UNREADABLE
    );
    let invalid =
        json!({"oauth:tokenCacheV2": STANDARD.encode(encrypted(b"invalid-private-json", &key))});
    assert_eq!(
        decode_cache(&invalid, "oauth:tokenCacheV2", &key).unwrap_err(),
        UNREADABLE
    );
}

fn cookie_db(root: &Path) -> Connection {
    std::fs::create_dir_all(root.join("Network")).unwrap();
    let db = Connection::open(root.join("Network/Cookies")).unwrap();
    db.execute_batch("CREATE TABLE cookies(host_key TEXT,name TEXT,value TEXT,encrypted_value BLOB,last_update_utc INTEGER); CREATE TABLE meta(key TEXT,value TEXT); INSERT INTO meta VALUES('version','24');").unwrap();
    db
}

#[test]
fn active_organization_cookie_restricts_host_and_verifies_hash() {
    let fixture = Fixture::new();
    let key = [4; 32];
    let organization = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    let db = cookie_db(&fixture.0);
    db.execute(
        "INSERT INTO cookies VALUES(?1,'lastActiveOrg',?2,x'',100)",
        ["foreign.example", organization],
    )
    .unwrap();
    assert_eq!(active_org(&fixture.0, &key).unwrap(), None);
    let mut clear = Sha256::digest(b".claude.ai").to_vec();
    clear.extend(organization.as_bytes());
    db.execute(
        "INSERT INTO cookies VALUES('.claude.ai','lastActiveOrg','',?1,1)",
        [encrypted(&clear, &key)],
    )
    .unwrap();
    assert_eq!(
        active_org(&fixture.0, &key).unwrap().as_deref(),
        Some(organization)
    );
    clear[0] ^= 1;
    db.execute(
        "UPDATE cookies SET encrypted_value=?1 WHERE host_key='.claude.ai'",
        [encrypted(&clear, &key)],
    )
    .unwrap();
    let cookie = active_org(&fixture.0, &key);
    let account = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
    let cache = json!({format!("acct:{account}|9d1c250a-e61b-44d9-88ed-5944d1962f5e:{organization}:https://api.anthropic.com:user:profile"):null});
    assert_eq!(
        resolve_organization(cookie, Some(&cache), None, Some(account)).unwrap_err(),
        UNREADABLE
    );
}

#[test]
fn busy_cookie_allows_only_unique_account_and_invalid_schema_never_falls_back() {
    let account = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
    let organization = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    let cache = json!({format!("acct:{account}|9d1c250a-e61b-44d9-88ed-5944d1962f5e:{organization}:https://api.anthropic.com:user:profile"):null});
    assert_eq!(
        resolve_organization(
            Err(CookieError::Unavailable),
            Some(&cache),
            None,
            Some(account)
        )
        .unwrap(),
        organization
    );
    let fixture = Fixture::new();
    let db = cookie_db(&fixture.0);
    db.execute(
        "INSERT INTO cookies VALUES('claude.ai','lastActiveOrg',?1,x'',1)",
        ["x".repeat(129)],
    )
    .unwrap();
    assert_eq!(
        resolve_organization(
            active_org(&fixture.0, &[4; 32]),
            Some(&cache),
            None,
            Some(account)
        )
        .unwrap_err(),
        UNREADABLE
    );
    db.execute("DELETE FROM cookies", []).unwrap();
    db.execute(
        "INSERT INTO cookies VALUES('claude.ai','lastActiveOrg','',?1,1)",
        [encrypted(organization.as_bytes(), &[4; 32])],
    )
    .unwrap();
    db.execute("UPDATE meta SET value='invalid'", []).unwrap();
    assert_eq!(
        resolve_organization(
            active_org(&fixture.0, &[4; 32]),
            Some(&cache),
            None,
            Some(account)
        )
        .unwrap_err(),
        UNREADABLE
    );
    db.execute("DELETE FROM meta", []).unwrap();
    assert_eq!(
        resolve_organization(
            active_org(&fixture.0, &[4; 32]),
            Some(&cache),
            None,
            Some(account)
        )
        .unwrap_err(),
        UNREADABLE
    );
}
