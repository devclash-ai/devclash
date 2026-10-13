//! Only the explicit dashboard Retry action can reach these interactive reads.
#[derive(Debug, PartialEq, Eq)]
enum Source {
    Code(String),
    Desktop,
}

fn permission_required(error: &str) -> bool {
    error.starts_with("Keychain access is required.")
}

fn blocked_source(
    services: impl IntoIterator<Item = String>,
    mut code: impl FnMut(&str) -> Result<(), String>,
    desktop: impl FnOnce() -> Result<(), String>,
) -> Option<Source> {
    // Match the loader's Code-first preference, selecting only an actual permission denial.
    // Readable, absent or invalid stores are handled by the normal credential loader.
    for service in services {
        if code(&service).is_err_and(|error| permission_required(&error)) {
            return Some(Source::Code(service));
        }
    }
    if desktop().is_err_and(|error| permission_required(&error)) {
        return Some(Source::Desktop);
    }
    None
}

fn retry(
    mut load: impl FnMut() -> Result<(), String>,
    probe: impl FnOnce() -> Option<Source>,
    authorize: impl FnOnce(Source) -> Result<(), String>,
) -> Result<(), String> {
    let reason = match load() {
        Ok(()) => return Ok(()),
        Err(reason) => reason,
    };
    let Some(source) = probe() else {
        return Err(reason);
    };
    authorize(source)?;
    // A granted Keychain read alone is not a usable subscription login. Reuse all validation.
    load()
}

#[cfg(target_os = "macos")]
pub(super) fn authorize() -> Result<(), String> {
    retry(
        || super::auth::load("claude").map(|_| ()),
        || {
            let services = crate::paths::configured("claude")
                .iter()
                .map(|directory| super::auth::claude_service(directory))
                .collect::<Vec<_>>();
            blocked_source(
                services,
                |service| super::mac_keychain::password(service, None).map(|_| ()),
                || super::claude_desktop::load().map(|_| ()),
            )
        },
        |source| match source {
            Source::Code(service) => {
                super::mac_keychain::read(&service, None, true)?.ok_or(
                    "No Claude Code Keychain login was found. Sign in with Claude Code first.",
                )?;
                Ok(())
            }
            Source::Desktop => super::claude_desktop::authorize(),
        },
    )
}

#[cfg(test)]
mod tests;
