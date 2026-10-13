use super::*;
const ACCESS: &str = "Keychain access is required. Authorize this provider.";

fn services() -> Vec<String> {
    vec!["native-code".into(), "second-code".into()]
}

#[test]
fn desktop_only_prompts_desktop_and_code_only_targets_the_blocked_service() {
    assert_eq!(
        blocked_source(services(), |_| Ok(()), || Err(ACCESS.into())),
        Some(Source::Desktop)
    );
    let mut probed = vec![];
    let source = blocked_source(
        services(),
        |service| {
            probed.push(service.to_owned());
            if service == "second-code" {
                Err(ACCESS.into())
            } else {
                Ok(())
            }
        },
        || panic!("Code denial must not probe or prompt Desktop"),
    );
    assert_eq!(source, Some(Source::Code("second-code".into())));
    assert_eq!(probed, services());
}

#[test]
fn both_blocked_selects_only_the_preferred_code_source() {
    let source = blocked_source(
        services(),
        |_| Err(ACCESS.into()),
        || panic!("A blocked preferred Code source is sufficient"),
    );
    assert_eq!(source, Some(Source::Code("native-code".into())));
    let mut loads = 0;
    retry(
        || {
            loads += 1;
            if loads == 1 {
                Err(ACCESS.into())
            } else {
                Ok(())
            }
        },
        || source,
        |source| {
            assert_eq!(source, Source::Code("native-code".into()));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(loads, 2);
}

#[test]
fn already_readable_login_skips_probes_and_authorization() {
    retry(
        || Ok(()),
        || panic!("No extra Keychain probe is needed"),
        |_| panic!("No prompt is needed"),
    )
    .unwrap();
}

#[test]
fn denial_is_returned_without_further_login_attempts() {
    let mut loads = 0;
    let result = retry(
        || {
            loads += 1;
            Err(ACCESS.into())
        },
        || Some(Source::Desktop),
        |_| Err("Authorization was denied.".into()),
    );
    assert_eq!(result.unwrap_err(), "Authorization was denied.");
    assert_eq!(loads, 1);
}

#[test]
fn absent_and_expired_logins_keep_the_loader_error_without_a_prompt() {
    for reason in ["No saved login was found.", "Claude login has expired."] {
        let source = blocked_source(services(), |_| Ok(()), || Err(reason.into()));
        assert_eq!(source, None);
        let result = retry(
            || Err(reason.into()),
            || source,
            |_| panic!("No blocked login"),
        );
        assert_eq!(result.unwrap_err(), reason);
    }
}

#[test]
fn authorization_success_still_requires_a_usable_login() {
    for reason in [
        "No saved login was found.",
        "Claude login has expired.",
        ACCESS,
    ] {
        let mut loads = 0;
        let result = retry(
            || {
                loads += 1;
                Err(if loads == 1 { ACCESS } else { reason }.into())
            },
            || Some(Source::Desktop),
            |_| Ok(()),
        );
        assert_eq!(result.unwrap_err(), reason);
        assert_eq!(loads, 2);
    }
}

#[test]
fn custom_or_unreadable_roots_cannot_authorize_native_logins() {
    use std::path::PathBuf;
    let defaults = vec![PathBuf::from("/synthetic/home/.claude")];
    assert!(super::super::require_automatic_discovery(&defaults, &defaults).is_ok());
    for configured in [vec![PathBuf::from("/synthetic/custom")], vec![]] {
        assert!(
            super::super::require_automatic_discovery(&configured, &defaults)
                .unwrap_err()
                .contains("Automatic Discovery")
        );
    }
}
