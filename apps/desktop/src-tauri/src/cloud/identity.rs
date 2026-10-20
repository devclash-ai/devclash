use std::path::Path;

/// Production must retain its historical service across versions and certificate renewal.
/// Alternate bundles, debug builds and explicit data profiles get a stable private scope.
pub(super) fn service(directory: &Path, identifier: &str, isolated: bool) -> String {
    if identifier == "ai.devclash.desktop" && !isolated {
        return super::SERVICE.into();
    }
    let directory = directory.canonicalize().unwrap_or_else(|_| directory.to_path_buf());
    let scope = super::records::hash(&format!("{identifier}\n{}", directory.to_string_lossy()));
    format!("ai.devclash.desktop.qa.cloud.{scope}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn production_identity_is_stable_while_qa_bundles_and_profiles_are_separate() {
        let a = Path::new("/synthetic/profile-a");
        let b = Path::new("/synthetic/profile-b");
        assert_eq!(service(a, "ai.devclash.desktop", false), super::super::SERVICE);
        assert_eq!(service(b, "ai.devclash.desktop", false), super::super::SERVICE);
        let qa = service(a, "ai.devclash.desktop", true);
        assert_ne!(qa, super::super::SERVICE);
        assert_eq!(qa, service(a, "ai.devclash.desktop", true));
        assert_ne!(qa, service(b, "ai.devclash.desktop", true));
        assert_ne!(qa, service(a, "ai.devclash.diagnostics.qa", false));
        assert_ne!(service(a, "ai.devclash.diagnostics.qa", false), super::super::SERVICE);
        assert!(!qa.contains("synthetic"));
    }
}
