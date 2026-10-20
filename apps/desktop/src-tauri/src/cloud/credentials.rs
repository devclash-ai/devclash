use super::*;
impl CloudStore {
    fn read_saved_credential(&self) -> Result<String, String> {
        let entry = self.entry()?;
        match entry.get_password() {
            Ok(encoded) => {
                let mut preferences = self
                    .preferences
                    .lock()
                    .map_err(|_| "Cloud preferences unavailable.")?;
                if !preferences.user_id.is_empty() {
                    origin_migration::validate_saved_account(&encoded, &preferences.user_id)?;
                }
                self.finish_origin_migration(&mut preferences)?;
                Ok(encoded)
            }
            // QA never probes production or legacy services, even with copied preferences.
            Err(keyring::Error::NoEntry) if self.credential_service != SERVICE => {
                Err("Sign in to this isolated QA profile to save usage history.".into())
            }
            Err(keyring::Error::NoEntry) => {
                let mut preferences = self
                    .preferences
                    .lock()
                    .map_err(|_| "Cloud preferences unavailable.")?;
                if preferences.user_id.is_empty()
                    || preferences.session_revoked
                    || self
                        .api
                        .as_ref()
                        .is_none_or(|api| api.origin != preferences.origin)
                {
                    return Err("Sign in to save your usage history.".into());
                }
                let previous =
                    origin_migration::previous_credential_origin(&preferences, &self.api()?.origin);
                let credential_origin = previous.unwrap_or(&preferences.origin);
                let encoded = if previous.is_some() {
                    let old = keyring::Entry::new(SERVICE, credential_origin)
                        .map_err(credential_error)?;
                    match old.get_password() {
                        Err(keyring::Error::NoEntry) => read_legacy_credential(credential_origin)?,
                        result => result.map_err(credential_error)?,
                    }
                } else {
                    read_legacy_credential(credential_origin)?
                };
                origin_migration::validate_saved_account(&encoded, &preferences.user_id)?;
                entry.set_password(&encoded).map_err(|_| {
                    eprintln!("Cloud credential migration could not be saved");
                    "Your OS credential store could not reconnect the session. Unlock it and try again.".to_string()
                })?;
                self.finish_origin_migration(&mut preferences)?;
                Ok(encoded)
            }
            Err(error) => Err(credential_error(error)),
        }
    }
    fn finish_origin_migration(&self, preferences: &mut Preferences) -> Result<(), String> {
        if let Some(previous) = preferences.credential_origin.take() {
            if let Err(error) = self.save(preferences) {
                preferences.credential_origin = Some(previous);
                eprintln!("Cloud credential origin migration could not be saved");
                return Err(error);
            }
        }
        Ok(())
    }
    pub(super) fn credential(&self) -> Result<Credential, String> {
        if self
            .preferences
            .lock()
            .map_err(|_| "Cloud preferences unavailable.")?
            .session_revoked
        {
            return Err("Sign in to save your usage history.".into());
        }
        let mut stored = self
            .credential
            .lock()
            .map_err(|_| "Cloud session unavailable.")?;
        if stored.is_none() {
            let value = self.read_saved_credential()?;
            *stored = Some(serde_json::from_str(&value).map_err(|_| {
                "Your saved session could not be read. Sign out and sign in again."
            })?);
        }
        let credential = stored
            .clone()
            .ok_or("Sign in to save your usage history.")?;
        if DateTime::parse_from_rfc3339(&credential.expires_at)
            .map_or(true, |date| date <= Utc::now())
        {
            drop(stored);
            self.forget_session()?;
            return Err("Your session expired. Sign in again.".into());
        }
        Ok(credential)
    }
}

fn credential_error(error: keyring::Error) -> String {
    match error {
        keyring::Error::NoEntry => "Sign in to save your usage history.".into(),
        _ => {
            eprintln!("Cloud credential read failed");
            "Could not read your session from the OS credential store. Unlock it and try again."
                .into()
        }
    }
}

fn read_legacy_credential(origin: &str) -> Result<String, String> {
    keyring::Entry::new("app.usagerank.desktop.cloud", origin)
        .map_err(credential_error)?
        .get_password()
        .map_err(credential_error)
}
