//! Limit unattended NSIS updates to the registered installation owned by this user.

#[derive(Clone)]
struct Registration {
    directory: String,
    binary: String,
}

fn normalized_path(value: &str) -> Option<String> {
    let path = value
        .trim()
        .trim_matches('"')
        .replace('/', "\\")
        .to_lowercase();
    let path = if let Some(unc) = path.strip_prefix("\\\\?\\unc\\") {
        format!("\\\\{unc}")
    } else {
        path.strip_prefix("\\\\?\\").unwrap_or(&path).to_owned()
    };
    let unc = path.starts_with("\\\\");
    let drive = path.as_bytes().get(1) == Some(&b':') && path.as_bytes().get(2) == Some(&b'\\');
    if !unc && !drive {
        return None;
    }
    let mut parts = Vec::new();
    let root_parts = if unc { 2 } else { 1 };
    for part in path.split('\\').filter(|part| !part.is_empty()) {
        match part {
            "." => {}
            ".." => {
                if parts.len() <= root_parts {
                    return None;
                }
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    if parts.len() < root_parts {
        return None;
    }
    Some(format!(
        "{}{}",
        if unc { "\\\\" } else { "" },
        parts.join("\\")
    ))
}

fn ownership_matches(
    executable: &str,
    user: &[Registration],
    saved: &[String],
    machine: &[Registration],
) -> bool {
    let Some(executable) = normalized_path(executable) else {
        return false;
    };
    let Some((directory, binary)) = executable.rsplit_once('\\') else {
        return false;
    };
    let same_directory = |value: &str| normalized_path(value).as_deref() == Some(directory);
    !user.is_empty()
        && user.iter().all(|entry| {
            same_directory(&entry.directory) && entry.binary.eq_ignore_ascii_case(binary)
        })
        && !saved.is_empty()
        && saved.iter().all(|value| same_directory(value))
        && !machine.iter().any(|entry| same_directory(&entry.directory))
}

fn registry_paths(product: &str, publisher: Option<&str>, identifier: &str) -> (String, String) {
    // Match Tauri's NSIS manufacturer: publisher, otherwise the identifier's second component.
    let manufacturer =
        publisher.unwrap_or_else(|| identifier.split('.').nth(1).unwrap_or(identifier));
    (
        format!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{product}"),
        format!("Software\\{manufacturer}\\{product}"),
    )
}

#[cfg(target_os = "windows")]
pub fn is_current_user_installation(app: &tauri::AppHandle, executable: &std::path::Path) -> bool {
    use winreg::{enums::*, RegKey};
    let config = app.config();
    let product = config
        .product_name
        .as_deref()
        .unwrap_or(&app.package_info().name);
    let (registration, location) = registry_paths(
        product,
        config.bundle.publisher.as_deref(),
        &config.identifier,
    );
    let read = || -> std::io::Result<bool> {
        let canonical_directory = |value: String| -> std::io::Result<String> {
            std::fs::canonicalize(value.trim().trim_matches('"'))
                .map(|path| path.to_string_lossy().into_owned())
        };
        let mut user = Vec::new();
        let mut machine = Vec::new();
        let mut saved = Vec::new();
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            for (hive, entries) in [
                (HKEY_CURRENT_USER, &mut user),
                (HKEY_LOCAL_MACHINE, &mut machine),
            ] {
                match RegKey::predef(hive).open_subkey_with_flags(&registration, KEY_READ | view) {
                    Ok(key) => entries.push(Registration {
                        directory: canonical_directory(key.get_value("InstallLocation")?)?,
                        binary: key.get_value("MainBinaryName")?,
                    }),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            match RegKey::predef(HKEY_CURRENT_USER)
                .open_subkey_with_flags(&location, KEY_READ | view)
            {
                Ok(key) => saved.push(canonical_directory(key.get_value("")?)?),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        let executable = std::fs::canonicalize(executable)?;
        Ok(ownership_matches(
            &executable.to_string_lossy(),
            &user,
            &saved,
            &machine,
        ))
    };
    match read() {
        Ok(owned) => owned,
        Err(error) => {
            eprintln!("Windows update installation ownership could not be verified: {error}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const EXE: &str = r"C:\Users\QA\DevClash\devclash-desktop.exe";
    const DIRECTORY: &str = r"C:\Users\QA\DevClash";
    fn entry(directory: &str) -> Registration {
        Registration {
            directory: directory.into(),
            binary: "devclash-desktop.exe".into(),
        }
    }
    fn check(user: Vec<Registration>, saved: Vec<String>, machine: Vec<Registration>) -> bool {
        ownership_matches(EXE, &user, &saved, &machine)
    }
    #[test]
    fn matching_current_user_installation_is_owned() {
        assert!(check(
            vec![entry(DIRECTORY)],
            vec![DIRECTORY.into()],
            vec![]
        ));
    }
    #[test]
    fn saved_destination_mismatch_is_not_owned() {
        assert!(!check(
            vec![entry(DIRECTORY)],
            vec![r"C:\Other".into()],
            vec![]
        ));
    }
    #[test]
    fn missing_registration_or_saved_destination_is_not_owned() {
        assert!(!check(vec![], vec![DIRECTORY.into()], vec![]));
        assert!(!check(vec![entry(DIRECTORY)], vec![], vec![]));
        let mut wrong_binary = entry(DIRECTORY);
        wrong_binary.binary = "other.exe".into();
        assert!(!check(vec![wrong_binary], vec![DIRECTORY.into()], vec![]));
    }
    #[test]
    fn matching_machine_installation_disqualifies_automatic_updates() {
        assert!(!check(
            vec![entry(DIRECTORY)],
            vec![DIRECTORY.into()],
            vec![entry(DIRECTORY)]
        ));
        assert!(check(
            vec![entry(DIRECTORY)],
            vec![DIRECTORY.into()],
            vec![entry(r"C:\Program Files\DevClash")]
        ));
    }
    #[test]
    fn case_quotes_separators_and_extended_paths_match() {
        assert!(check(
            vec![entry(r#""c:/USERS/qa/DevClash/""#)],
            vec![r"\\?\C:\Users\QA\.\DevClash\".into()],
            vec![]
        ));
        assert_eq!(
            normalized_path(r"\\?\UNC\SERVER\Share\DevClash\"),
            normalized_path(r"\\server\share\devclash")
        );
        assert_eq!(
            normalized_path(r"C:\Users\QA\other\..\DevClash"),
            normalized_path(DIRECTORY)
        );
        assert!(normalized_path(r"C:relative").is_none());
        assert!(normalized_path(r"C:\..\DevClash").is_none());
    }
    #[test]
    fn inconsistent_registry_views_are_not_owned() {
        assert!(!check(
            vec![entry(DIRECTORY), entry(r"C:\Other")],
            vec![DIRECTORY.into()],
            vec![]
        ));
        assert!(!check(
            vec![entry(DIRECTORY)],
            vec![DIRECTORY.into(), r"C:\Other".into()],
            vec![]
        ));
    }
    #[test]
    fn qa_runtime_without_publisher_uses_shipping_manufacturer() {
        // Tauri's generated runtime config omits bundle.publisher. The QA
        // fixture uses the shipping default manufacturer and its own product.
        let (registration, location) =
            registry_paths("DevClash Updater QA", None, "ai.devclash.updater.qa");
        assert_eq!(
            registration,
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall\DevClash Updater QA"
        );
        assert_eq!(location, r"Software\devclash\DevClash Updater QA");
    }
    #[test]
    fn production_and_qa_use_the_nsis_registration_names() {
        let (registration, location) = registry_paths("DevClash", None, "ai.devclash.desktop");
        assert!(registration.ends_with(r"\Uninstall\DevClash"));
        assert_eq!(location, r"Software\devclash\DevClash");
        let (_, qa) = registry_paths(
            "DevClash Updater QA",
            Some("DevClash Updater QA"),
            "ai.devclash.updater.qa",
        );
        assert_eq!(qa, r"Software\DevClash Updater QA\DevClash Updater QA");
    }
}
