use super::*;
use serde_json::json;

const ACCOUNT: &str = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";
const FOREIGN: &str = "ffffffff-ffff-4fff-8fff-ffffffffffff";
const ORG: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const OTHER_ORG: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const OTHER_CLIENT: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const NOW: f64 = 1_800_000_000_000.;

fn key(client: &str, org: &str, scopes: &str) -> String {
    format!("{client}:{org}:{HOST}:{scopes}")
}
fn legacy() -> String {
    key(PRODUCTION_CLIENT, ORG, "user:profile user:inference")
}
fn scoped() -> String {
    format!("acct:{ACCOUNT}|{}", legacy())
}
fn token(name: &str, remaining_ms: f64) -> Value {
    json!({"token":name,"expiresAt":NOW+remaining_ms,"refreshToken":"never-borrow-this"})
}
fn live(name: &str) -> Value {
    token(name, 3_600_000.)
}
fn choose(v2: Option<&Value>, v1: Option<&Value>) -> Result<String, String> {
    select(v2, v1, Some(ACCOUNT), ORG, NOW)
}

#[test]
fn legacy_and_scoped_keys_work_in_either_version_without_borrowing_refresh_token() {
    for cache_key in [legacy(), scoped()] {
        let cache = json!({cache_key:live("access-only")});
        assert_eq!(choose(Some(&cache), None).unwrap(), "access-only");
        assert_eq!(choose(None, Some(&cache)).unwrap(), "access-only");
    }
    let cache = json!({legacy():live("legacy-without-account")});
    assert_eq!(
        select(Some(&cache), None, None, ORG, NOW).unwrap(),
        "legacy-without-account"
    );
}

#[test]
fn scoped_entries_require_a_matching_valid_account_and_uuid_case_is_ignored() {
    let cache = json!({scoped():live("private")});
    for account in [None, Some(""), Some("invalid"), Some(FOREIGN)] {
        assert_eq!(
            select(Some(&cache), None, account, ORG, NOW).unwrap_err(),
            NOT_FOUND
        );
    }
    let cache = json!({format!("acct:{}|{}",ACCOUNT.to_uppercase(),legacy().replace(PRODUCTION_CLIENT,&PRODUCTION_CLIENT.to_uppercase()).replace(ORG,&ORG.to_uppercase())):live("matching")});
    assert_eq!(
        select(
            Some(&cache),
            None,
            Some(&ACCOUNT.to_uppercase()),
            &ORG.to_uppercase(),
            NOW
        )
        .unwrap(),
        "matching"
    );
}

#[test]
fn malformed_keys_wrong_org_host_and_missing_profile_scope_are_ineligible() {
    let malformed = [
        format!("acct:|{}", legacy()),
        format!("acct:invalid|{}", legacy()),
        format!("acct:{ACCOUNT}{}", legacy()),
        format!("acct:{ACCOUNT}||{}", legacy()),
        format!("acct:{ACCOUNT}|acct:{ACCOUNT}|{}", legacy()),
        format!("acct:{ACCOUNT}|"),
        key("invalid", ORG, "user:profile"),
        key(PRODUCTION_CLIENT, "invalid", "user:profile"),
        key(PRODUCTION_CLIENT, OTHER_ORG, "user:profile"),
        legacy().replace(HOST, "https://example.com"),
        key(PRODUCTION_CLIENT, ORG, "user:inference"),
    ];
    for key in malformed {
        assert_eq!(
            choose(Some(&json!({key:live("wrong")})), None).unwrap_err(),
            NOT_FOUND
        );
    }
}

#[test]
fn scoped_alias_overrides_legacy_even_if_expired_invalid_or_deleted() {
    for version in [1, 2] {
        for (entry, expected) in [
            (live("current"), Ok("current".to_owned())),
            (token("expired", -1.), Err(STALE.to_owned())),
            (json!({"token":" "}), Err(INVALID.to_owned())),
            (Value::Null, Err(NOT_FOUND.to_owned())),
        ] {
            let cache = json!({legacy():token("older",86_400_000.),scoped():entry});
            let result = if version == 2 {
                choose(Some(&cache), None)
            } else {
                choose(None, Some(&cache))
            };
            assert_eq!(result, expected);
        }
    }
}

#[test]
fn v2_presence_suppresses_v1_across_all_alias_formats() {
    for v2_key in [legacy(), scoped()] {
        for v1_key in [legacy(), scoped()] {
            let v1 = json!({v1_key:token("resurrected",86_400_000.)});
            for (entry, expected) in [
                (live("v2"), Ok("v2".to_owned())),
                (Value::Null, Err(NOT_FOUND.to_owned())),
                (token("expired", -1.), Err(STALE.to_owned())),
                (json!({"token":" "}), Err(INVALID.to_owned())),
            ] {
                let v2 = json!({v2_key.clone():entry});
                assert_eq!(choose(Some(&v2), Some(&v1)), expected);
            }
        }
    }
}

#[test]
fn foreign_account_cannot_supply_a_token_or_suppress_v1_fallback() {
    for entry in [live("foreign"), Value::Null] {
        let v2 = json!({format!("acct:{FOREIGN}|{}",legacy()):entry});
        assert_eq!(choose(Some(&v2), None).unwrap_err(), NOT_FOUND);
        let v1 = json!({scoped():live("current")});
        assert_eq!(choose(Some(&v2), Some(&v1)).unwrap(), "current");
    }
}

#[test]
fn v1_fallback_for_distinct_key_is_allowed_even_when_v2_has_stale_entries() {
    let v2 = json!({legacy():token("expired",-1.)});
    let v1 = json!({key(OTHER_CLIENT,ORG,"user:profile"):live("distinct")});
    assert_eq!(choose(Some(&v2), Some(&v1)).unwrap(), "distinct");
}

#[test]
fn v2_version_precedes_rank_and_ranking_follows_upstream() {
    let profile = key(PRODUCTION_CLIENT, ORG, "user:profile");
    let v2 = json!({profile.clone():token("v2-profile",300_000.)});
    let v1 = json!({legacy():token("v1-full",86_400_000.)});
    assert_eq!(choose(Some(&v2), Some(&v1)).unwrap(), "v2-profile");

    let nonproduction = key(OTHER_CLIENT, ORG, "user:profile user:inference extra");
    let cache = json!({
        profile.clone():token("profile-only",86_400_000.),
        legacy():token("production-full",300_000.),
        nonproduction.clone():token("nonproduction-full",86_400_000.)
    });
    assert_eq!(choose(Some(&cache), None).unwrap(), "production-full");
    let cache = json!({profile:live("profile-only"),nonproduction.clone():live("full")});
    assert_eq!(choose(Some(&cache), None).unwrap(), "full");
    let cache = json!({key(OTHER_CLIENT,ORG,"user:profile user:inference"):token("fewer",86_400_000.),nonproduction:live("richer")});
    assert_eq!(choose(Some(&cache), None).unwrap(), "richer");
    let cache = json!({key(OTHER_CLIENT,ORG,"user:profile user:inference"):live("shorter"),key("dddddddd-dddd-4ddd-8ddd-dddddddddddd",ORG,"user:profile user:inference"):token("longer",86_400_000.)});
    assert_eq!(choose(Some(&cache), None).unwrap(), "longer");
}

#[test]
fn expiry_requires_more_than_two_minutes_and_stale_precedes_invalid() {
    for remaining in [-1., 0., 119_999., 120_000.] {
        assert_eq!(
            choose(Some(&json!({scoped():token("expiring",remaining)})), None).unwrap_err(),
            STALE
        );
    }
    assert_eq!(
        choose(Some(&json!({scoped():token("usable",120_001.)})), None).unwrap(),
        "usable"
    );
    let cache = json!({legacy():token("stale",-1.),key(OTHER_CLIENT,ORG,"user:profile"):json!({"token":"invalid"})});
    assert_eq!(choose(Some(&cache), None).unwrap_err(), STALE);
}

#[test]
fn malformed_entries_and_bounds_return_fixed_errors_without_secret_contents() {
    for entry in [
        json!("secret-malformed-entry"),
        json!({"token":"secret-no-expiry"}),
        json!({"token":"secret","expiresAt":"9999999999999"}),
        json!({"token":42,"expiresAt":NOW+3_600_000.}),
        token(" \n\t", 3_600_000.),
        live(&"x".repeat(MAX_TOKEN_BYTES + 1)),
    ] {
        assert_eq!(
            choose(Some(&json!({legacy():entry})), None).unwrap_err(),
            INVALID
        );
    }
    assert_eq!(
        choose(
            Some(&json!({legacy():live(&"x".repeat(MAX_TOKEN_BYTES))})),
            None
        )
        .unwrap()
        .len(),
        MAX_TOKEN_BYTES
    );
    for cache in [
        json!([]),
        json!(null),
        json!("secret-cache"),
        json!({"x".repeat(MAX_KEY_BYTES+1):live("secret")}),
    ] {
        assert_eq!(choose(Some(&cache), None).unwrap_err(), INVALID);
    }
    let oversized = Value::Object(
        (0..=MAX_ENTRIES)
            .map(|index| (format!("key{index}"), Value::Null))
            .collect(),
    );
    assert_eq!(choose(Some(&oversized), None).unwrap_err(), INVALID);
    assert_eq!(
        select(None, None, Some(ACCOUNT), "invalid", NOW).unwrap_err(),
        INVALID
    );
    assert_eq!(
        select(None, None, Some(ACCOUNT), ORG, f64::NAN).unwrap_err(),
        INVALID
    );
    assert_eq!(choose(None, None).unwrap_err(), NOT_FOUND);
}

#[test]
fn unique_organization_requires_current_account_scoped_evidence() {
    let scoped_cache = json!({scoped():live("current")});
    for cache_versions in [(Some(&scoped_cache), None), (None, Some(&scoped_cache))] {
        assert_eq!(
            single_organization(cache_versions.0, cache_versions.1, Some(ACCOUNT)).unwrap(),
            ORG
        );
    }
    for account in [None, Some(""), Some("invalid"), Some(FOREIGN)] {
        assert_eq!(
            single_organization(Some(&scoped_cache), None, account).unwrap_err(),
            UNKNOWN_ORGANIZATION
        );
    }
    let legacy_cache = json!({legacy():live("legacy")});
    assert_eq!(
        single_organization(Some(&legacy_cache), None, Some(ACCOUNT)).unwrap_err(),
        UNKNOWN_ORGANIZATION
    );
    let foreign = json!({format!("acct:{FOREIGN}|{}",key(PRODUCTION_CLIENT,OTHER_ORG,"user:profile")):live("foreign")});
    assert_eq!(
        single_organization(Some(&foreign), Some(&scoped_cache), Some(ACCOUNT)).unwrap(),
        ORG
    );
    assert_eq!(
        single_organization(None, None, Some(ACCOUNT)).unwrap_err(),
        UNKNOWN_ORGANIZATION
    );
}

#[test]
fn unique_organization_counts_tombstones_stale_and_invalid_entries_without_ranking() {
    let second = format!(
        "acct:{ACCOUNT}|{}",
        key(PRODUCTION_CLIENT, OTHER_ORG, "user:profile")
    );
    for value in [
        Value::Null,
        token("expired", -1.),
        json!({"token":"invalid"}),
    ] {
        let old = json!({second.clone():value});
        let current = json!({scoped():live("current")});
        assert_eq!(
            single_organization(Some(&old), Some(&current), Some(ACCOUNT)).unwrap_err(),
            UNKNOWN_ORGANIZATION
        );
    }
    let tombstone = json!({scoped():Value::Null});
    let v1 = json!({scoped():live("old")});
    assert_eq!(
        single_organization(Some(&tombstone), Some(&v1), Some(ACCOUNT)).unwrap(),
        ORG
    );
    assert_eq!(choose(Some(&tombstone), Some(&v1)).unwrap_err(), NOT_FOUND);
}

#[test]
fn unique_organization_excludes_legacy_and_keys_without_profile_permission() {
    let current = json!({
        scoped():live("current"),
        key(PRODUCTION_CLIENT,OTHER_ORG,"user:profile"):live("unowned legacy"),
        format!("acct:{ACCOUNT}|{}",key(PRODUCTION_CLIENT,OTHER_ORG,"user:inference")):live("inference"),
        format!("acct:{ACCOUNT}|{}",key(PRODUCTION_CLIENT,OTHER_ORG,"user:profile").replace(HOST,"https://example.com")):live("wrong host")
    });
    assert_eq!(
        single_organization(Some(&current), None, Some(&ACCOUNT.to_uppercase())).unwrap(),
        ORG
    );
}

#[test]
fn scoped_selection_never_borrows_unproven_legacy_tokens_in_a_shared_organization() {
    let foreign_legacy = key(OTHER_CLIENT, ORG, "user:profile extra");
    for (entry, expected) in [
        (Value::Null, NOT_FOUND),
        (token("expired-current", -1.), STALE),
        (json!({"token":"invalid-current"}), INVALID),
    ] {
        let v2 = json!({scoped():entry,foreign_legacy.clone():live("older-account")});
        let v1 = json!({scoped():live("old-scoped-copy"),legacy():live("unscoped-alias"),foreign_legacy.clone():live("older-account-v1")});
        assert_eq!(
            select_scoped(Some(&v2), Some(&v1), Some(ACCOUNT), ORG, NOW).unwrap_err(),
            expected
        );
    }
    let only_legacy = json!({legacy():live("unproven")});
    for versions in [(Some(&only_legacy), None), (None, Some(&only_legacy))] {
        assert_eq!(
            select_scoped(versions.0, versions.1, Some(ACCOUNT), ORG, NOW).unwrap_err(),
            NOT_FOUND
        );
    }
}

#[test]
fn scoped_selection_requires_a_valid_current_account_and_excludes_foreign_accounts() {
    let cache = json!({scoped():live("current"),format!("acct:{FOREIGN}|{}",legacy()):token("foreign",86_400_000.),legacy():token("legacy",86_400_000.)});
    for account in [None, Some(""), Some("malformed")] {
        assert_eq!(
            select_scoped(Some(&cache), None, account, ORG, NOW).unwrap_err(),
            INVALID
        );
    }
    for versions in [(Some(&cache), None), (None, Some(&cache))] {
        assert_eq!(
            select_scoped(
                versions.0,
                versions.1,
                Some(&ACCOUNT.to_uppercase()),
                ORG,
                NOW
            )
            .unwrap(),
            "current"
        );
    }
    let foreign = json!({format!("acct:{FOREIGN}|{}",legacy()):live("foreign")});
    assert_eq!(
        select_scoped(Some(&foreign), None, Some(ACCOUNT), ORG, NOW).unwrap_err(),
        NOT_FOUND
    );
    let v2_legacy = json!({legacy():Value::Null});
    let v1_scoped = json!({scoped():live("proven-current")});
    assert_eq!(
        select_scoped(Some(&v2_legacy), Some(&v1_scoped), Some(ACCOUNT), ORG, NOW).unwrap(),
        "proven-current"
    );
}
