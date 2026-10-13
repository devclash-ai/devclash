use super::*;
const HEADER: &str =
    "Date,Model,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens\n";
fn verified(text: &str) -> VerifiedUsage {
    VerifiedUsage {
        source: SOURCE.into(),
        account_hash: hash("test-account"),
        fetched_at: "2026-10-01T12:00:00Z".into(),
        records: parse(text).unwrap(),
    }
}
#[test]
fn server_buckets_price_composer_and_preserve_unknown_model_tokens() {
    let text = format!("{HEADER}2026-09-30T12:00:00Z,composer-2.5,1000,1000,2000,1000\n2026-09-30T12:00:00Z,private-model,0,100,0,0\n");
    let records = parse(&text).unwrap();
    let pricing = crate::pricing::ModelPricing::bundled();
    let known = records
        .iter()
        .find(|record| record.event.model == "composer-2.5")
        .unwrap()
        .priced(&pricing);
    let unknown = records
        .iter()
        .find(|record| record.event.model == "private-model")
        .unwrap()
        .priced(&pricing);
    assert!((known.cost_usd.unwrap() - 0.0039).abs() < 1e-9);
    assert_eq!(known.tokens, 5000);
    assert_eq!(unknown.tokens, 100);
    assert_eq!(unknown.unpriced_tokens, 100);
    assert_eq!(unknown.cost_usd, None);
}
#[test]
fn valid_empty_history_is_successful_and_unusable_schema_is_not() {
    assert!(parse(HEADER).unwrap().is_empty());
    assert!(
        parse(&format!("{HEADER}2026-10-01T12:00:00Z,sonnet,0,,,0\n"))
            .unwrap()
            .is_empty()
    );
    assert!(parse("").is_err());
    assert!(parse("<html>sign in</html>").is_err());
    assert!(parse("Date,Model\n").is_err());
    assert!(parse(&format!("{},Date\n", HEADER.trim_end())).is_err());
}
#[test]
fn malformed_dates_counts_totals_and_rows_never_become_partial_history() {
    for row in [
        "2026-10-01 12:00:00,sonnet,1,2,3,4",
        "2026-10-01T12:00:00Z,sonnet,1,2,-3,4",
        "2026-10-01T12:00:00Z,sonnet,1,2,+3,4",
        "2026-10-01T12:00:00Z,sonnet,1,2,3.5,4",
        "2026-10-01T12:00:00Z,sonnet,1,2,3,4,5",
        "2026-10-01T12:00:00Z,sonnet,9007199254740991,2,3,4",
        "2026-10-01T12:00:00Z,sonnet,1,2,\"1,20\",4",
    ] {
        assert!(
            parse(&format!(
                "{HEADER}2026-10-01T12:00:00Z,sonnet,1,2,3,4\n{row}\n"
            ))
            .is_err(),
            "{row}"
        );
    }
    assert!(parse("Date,Model,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens\n2026-10-01T12:00:00Z,sonnet,1,2,3,4,11\n").is_err());
    assert!(parse(&format!("{HEADER}2026-10-01T12:00:00Z,sonnet,1,2,3,\"4\n")).is_err());
    assert!(parse(&format!(
        "{HEADER}2026-10-01T12:00:00Z,{},1,2,3,4\n",
        "x".repeat(129)
    ))
    .is_err());
    assert!(parse(&"x".repeat(MAX_BODY + 1)).is_err());
    assert!(parse(&format!(
        "{HEADER}{}",
        "2026-10-01T12:00:00Z,sonnet,1,2,3,4\n".repeat(MAX_ROWS + 1)
    ))
    .is_err());
}
#[test]
fn api_identity_is_stable_in_utc_and_deduplicates_reordered_history() {
    let a = "2026-10-01T01:30:00+03:00,sonnet,100,20,\"1,200\",5\n";
    let b = "2026-09-30T22:30:00Z,sonnet,100,20,\"1,200\",5\n";
    let records = parse(&format!("{HEADER}{a}{b}")).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].event.timestamp, "2026-09-30T22:30:00.000Z");
    assert_eq!(records[0].event.tokens, 1325);
    assert!(records[0].event.id.starts_with("cursor-api:"));
    assert_eq!(
        records[0].event.id,
        parse(&format!("{HEADER}{b}{a}")).unwrap()[0].event.id
    );
}
#[test]
fn transient_errors_preserve_verified_history_and_attempts_are_throttled() {
    let text = format!("{HEADER}2026-10-01T12:00:00Z,sonnet,1,2,3,4\n");
    let mut cache = Some(verified(&text));
    let mut state = FetchState::default();
    let now = Instant::now();
    let error = state.refresh_with(&mut cache, false, now, &hash("test-account"), || {
        Err("Temporary connection error".into())
    });
    assert_eq!(error.as_deref(), Some("Temporary connection error"));
    assert_eq!(cache.as_ref().unwrap().records.len(), 1);
    state.refresh_with(
        &mut cache,
        false,
        now + std::time::Duration::from_secs(299),
        &hash("test-account"),
        || panic!("Throttled"),
    );
    assert!(state
        .refresh_with(&mut cache, true, now, &hash("test-account"), || Ok(
            verified(HEADER)
        ))
        .is_none());
    assert!(cache.as_ref().unwrap().records.is_empty());
    assert!(cache.as_ref().unwrap().valid());
}
#[test]
fn changed_cursor_account_discards_old_rows_even_if_the_new_request_fails() {
    let mut cache = Some(verified(&format!(
        "{HEADER}2026-10-01T12:00:00Z,sonnet,1,2,3,4\n"
    )));
    let mut state = FetchState::default();
    let now = Instant::now();
    state.refresh_with(&mut cache, false, now, &hash("test-account"), || {
        Ok(verified(HEADER))
    });
    assert!(cache.is_some());
    state.refresh_with(&mut cache, false, now, &hash("different-account"), || {
        Err("New account temporarily offline".into())
    });
    assert!(cache.is_none());
}
#[test]
fn missing_or_expired_other_account_login_clears_cached_usage_without_a_request() {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(br#"{"sub":"auth|different-account","exp":1}"#);
    let expired = auth::cursor_login(&format!("header.{encoded}.signature"));
    assert!(expired.as_ref().err().unwrap().contains("expired"));
    for login in [Err("No Cursor login was found.".into()), expired] {
        let mut cache = Some(verified(&format!(
            "{HEADER}2026-10-01T12:00:00Z,sonnet,1,2,3,4\n"
        )));
        let error = FetchState::default().refresh_login(&mut cache, false, login, |_, _| {
            panic!("Unavailable login must not request history")
        });
        assert!(error.is_some());
        assert!(cache.is_none());
    }
}
#[test]
fn provider_rejected_login_is_not_a_transient_cached_history_fallback() {
    let mut cache = Some(verified(HEADER));
    let mut state = FetchState::default();
    state.refresh_with(
        &mut cache,
        true,
        Instant::now(),
        &hash("test-account"),
        || {
            Err(FetchError {
                message: "Cursor refused this login".into(),
                unauthenticated: true,
            })
        },
    );
    assert!(cache.is_none());
}
#[test]
fn desktop_and_cli_logins_share_one_account_history_and_one_fetch_interval() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let payload = URL_SAFE_NO_PAD.encode(br#"{"sub":"auth|test-account","exp":4102444800}"#);
    let desktop = auth::cursor_login(&format!("desktop.{payload}.signature")).unwrap();
    let cli = auth::cursor_login(&format!("cli.{payload}.different-signature")).unwrap();
    let mut cache = None;
    let mut state = FetchState::default();
    // Client is optional account metadata. Neither client should be filtered
    // out or appended as a second local-log stream.
    let history = format!("{}Client\n2026-10-01T12:00:00Z,sonnet,1,2,3,4,desktop\n2026-10-01T12:01:00Z,sonnet,5,6,7,8,cli\n", HEADER.trim_end().to_owned() + ",");
    assert!(state
        .refresh_login(&mut cache, false, Ok(desktop), |_, account| {
            assert_eq!(account, hash("test-account"));
            Ok(verified(&history))
        })
        .is_none());
    assert_eq!(
        cache
            .as_ref()
            .unwrap()
            .records
            .iter()
            .map(|row| row.event.tokens)
            .sum::<u64>(),
        36
    );
    let ids = cache
        .as_ref()
        .unwrap()
        .records
        .iter()
        .map(|row| row.event.id.clone())
        .collect::<Vec<_>>();
    assert!(state
        .refresh_login(&mut cache, false, Ok(cli), |_, _| panic!(
            "Same account is already current"
        ))
        .is_none());
    assert_eq!(
        cache
            .as_ref()
            .unwrap()
            .records
            .iter()
            .map(|row| row.event.id.clone())
            .collect::<Vec<_>>(),
        ids
    );
}

#[test]
fn account_history_never_guesses_projects_from_client_or_extra_path_columns() {
    let records = parse("Date,Model,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Client,Project,Directory\n2026-10-01T12:00:00Z,sonnet,1,2,3,4,CLI,synthetic-app,/synthetic/app\n").unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].event.tokens, 10);
    assert!(records[0].event.project.is_none());
    assert!(!serde_json::to_string(&records)
        .unwrap()
        .contains("/synthetic/app"));
}
