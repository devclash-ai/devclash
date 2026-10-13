use super::{
    origins::{LEGACY_API_ORIGIN, PRODUCTION_API_ORIGIN},
    Credential, Preferences,
};
use chrono::{DateTime, Utc};

pub fn align_preferences(preferences: &mut Preferences, origin: Option<&str>) -> bool {
    if origin == Some(PRODUCTION_API_ORIGIN) && preferences.origin == LEGACY_API_ORIGIN {
        preferences.origin = PRODUCTION_API_ORIGIN.into();
        preferences.credential_origin = (!preferences.user_id.is_empty()
            && !preferences.session_revoked)
            .then(|| LEGACY_API_ORIGIN.into());
        return true;
    }
    if origin != Some(preferences.origin.as_str()) {
        *preferences = Preferences::default();
    } else if origin != Some(PRODUCTION_API_ORIGIN)
        || preferences.credential_origin.as_deref() != Some(LEGACY_API_ORIGIN)
        || preferences.user_id.is_empty()
        || preferences.session_revoked
    {
        preferences.credential_origin = None;
    }
    false
}

pub fn previous_credential_origin<'a>(
    preferences: &'a Preferences,
    origin: &str,
) -> Option<&'a str> {
    (origin == PRODUCTION_API_ORIGIN
        && preferences.origin == PRODUCTION_API_ORIGIN
        && !preferences.user_id.is_empty()
        && !preferences.session_revoked
        && preferences.credential_origin.as_deref() == Some(LEGACY_API_ORIGIN))
    .then_some(LEGACY_API_ORIGIN)
}

pub fn validate_saved_account(encoded: &str, user_id: &str) -> Result<(), String> {
    let credential: Credential = serde_json::from_str(encoded)
        .map_err(|_| "Your saved session could not be read. Sign in again.".to_string())?;
    if user_id.is_empty() || credential.user.id != user_id || credential.token.is_empty() {
        return Err("Sign in to save your usage history.".into());
    }
    if DateTime::parse_from_rfc3339(&credential.expires_at).map_or(true, |date| date <= Utc::now())
    {
        return Err("Your session expired. Sign in again.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::{records::CloudRecord, Period, User};
    use std::collections::HashSet;
    #[test]
    fn official_move_preserves_queue_consent_rank_period_retry_and_acknowledgements() {
        let mut prefs = Preferences {
            origin: LEGACY_API_ORIGIN.into(),
            user_id: "synthetic-account".into(),
            sync_enabled: true,
            acknowledged: HashSet::from(["synthetic-ack".into()]),
            retry_attempts: 3,
            next_retry_at: Some("2026-10-03T12:00:00Z".into()),
            rank_period: Period::Month,
            ranked: true,
            public_profile: true,
            pending: vec![CloudRecord {
                id: "synthetic-event".into(),
                provider: "Claude".into(),
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
        let mut before = serde_json::to_value(&prefs).unwrap();
        assert!(align_preferences(&mut prefs, Some(PRODUCTION_API_ORIGIN)));
        assert_eq!(
            previous_credential_origin(&prefs, PRODUCTION_API_ORIGIN),
            Some(LEGACY_API_ORIGIN)
        );
        let serialized = serde_json::to_vec(&prefs).unwrap();
        let mut reloaded: Preferences = serde_json::from_slice(&serialized).unwrap();
        assert!(!align_preferences(
            &mut reloaded,
            Some(PRODUCTION_API_ORIGIN)
        ));
        assert_eq!(
            previous_credential_origin(&reloaded, PRODUCTION_API_ORIGIN),
            Some(LEGACY_API_ORIGIN)
        );
        let mut after = serde_json::to_value(&reloaded).unwrap();
        for value in [&mut before, &mut after] {
            value.as_object_mut().unwrap().remove("origin");
            value.as_object_mut().unwrap().remove("credentialOrigin");
        }
        assert_eq!(before, after);
    }
    #[test]
    fn custom_reverse_and_revoked_origins_never_reuse_production_credentials() {
        for (old, new) in [
            (LEGACY_API_ORIGIN, "https://other.example"),
            (PRODUCTION_API_ORIGIN, LEGACY_API_ORIGIN),
            ("http://127.0.0.1:1440", PRODUCTION_API_ORIGIN),
        ] {
            let mut prefs = Preferences {
                origin: old.into(),
                user_id: "synthetic-account".into(),
                sync_enabled: true,
                credential_origin: Some(LEGACY_API_ORIGIN.into()),
                ..Preferences::default()
            };
            assert!(!align_preferences(&mut prefs, Some(new)));
            assert!(!prefs.sync_enabled);
            assert!(prefs.user_id.is_empty());
            assert!(previous_credential_origin(&prefs, new).is_none());
        }
        let mut revoked = Preferences {
            origin: LEGACY_API_ORIGIN.into(),
            user_id: "synthetic-account".into(),
            session_revoked: true,
            ..Preferences::default()
        };
        assert!(align_preferences(&mut revoked, Some(PRODUCTION_API_ORIGIN)));
        assert!(previous_credential_origin(&revoked, PRODUCTION_API_ORIGIN).is_none());
    }
    #[test]
    fn migrated_credential_must_match_saved_account_and_still_be_valid() {
        let mut credential = Credential {
            token: "synthetic-test-token".into(),
            user: User {
                id: "synthetic-account".into(),
                email: "qa@example.test".into(),
            },
            expires_at: (Utc::now() + chrono::Duration::days(1)).to_rfc3339(),
        };
        assert!(validate_saved_account(
            &serde_json::to_string(&credential).unwrap(),
            "synthetic-account"
        )
        .is_ok());
        assert!(validate_saved_account(
            &serde_json::to_string(&credential).unwrap(),
            "different-account"
        )
        .is_err());
        credential.expires_at = (Utc::now() - chrono::Duration::days(1)).to_rfc3339();
        assert!(validate_saved_account(
            &serde_json::to_string(&credential).unwrap(),
            "synthetic-account"
        )
        .is_err());
        assert!(validate_saved_account("unreadable", "synthetic-account").is_err());
    }
}
