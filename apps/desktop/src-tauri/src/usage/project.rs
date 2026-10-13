//! Local identity comes only from an explicitly recorded absolute workspace directory.
//! Normalize lexically: removed workspaces must keep their identity without filesystem probes.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub directory: String,
}

impl Project {
    pub fn from_directory(directory: &str) -> Option<Self> {
        let directory = normalize(directory)?;
        let name = directory
            .rsplit('/')
            .find(|part| !part.is_empty())
            .unwrap_or("/")
            .to_owned();
        Some(Self {
            id: format!("project:{}", crate::cloud::records::hash(&directory)),
            name,
            directory,
        })
    }
}

fn normalize(directory: &str) -> Option<String> {
    if directory.is_empty() || directory.contains('\0') {
        return None;
    }
    let windows_drive = directory.as_bytes().get(1) == Some(&b':')
        && directory.as_bytes()[0].is_ascii_alphabetic()
        && directory
            .as_bytes()
            .get(2)
            .is_some_and(|b| matches!(b, b'/' | b'\\'));
    let unc = directory.starts_with("\\\\") || directory.starts_with("//");
    let normalized_separators = if windows_drive || unc {
        directory.replace('\\', "/")
    } else {
        directory.to_owned()
    };
    let (root, suffix) = if windows_drive {
        (
            format!("{}:/", directory[..1].to_ascii_uppercase()),
            normalized_separators[3..].to_owned(),
        )
    } else if unc {
        let mut parts = normalized_separators[2..]
            .split('/')
            .filter(|s| !s.is_empty());
        let server = parts.next()?;
        let share = parts.next()?;
        if matches!(server, "." | "..") || matches!(share, "." | "..") {
            return None;
        }
        let root = format!("//{server}/{share}");
        (root, parts.collect::<Vec<_>>().join("/"))
    } else if normalized_separators.starts_with('/') {
        ("/".into(), normalized_separators[1..].to_owned())
    } else {
        return None;
    };
    let mut parts = Vec::new();
    for part in suffix.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    if parts.is_empty() {
        Some(root)
    } else {
        Some(format!(
            "{}/{}",
            root.trim_end_matches('/'),
            parts.join("/")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equal_folder_names_in_different_roots_have_distinct_stable_identities() {
        let a = Project::from_directory("/work/first/app").unwrap();
        let b = Project::from_directory("/work/second/app").unwrap();
        assert_eq!(a.name, b.name);
        assert_ne!(a.id, b.id);
        assert_eq!(
            a,
            Project::from_directory("/work/first/./other/../app/").unwrap()
        );
        assert_eq!(
            a.id,
            format!("project:{}", crate::cloud::records::hash("/work/first/app"))
        );
    }
    #[test]
    fn absolute_windows_directories_are_supported_without_host_filesystem_access() {
        let a = Project::from_directory(r"c:\work\app\..\project\").unwrap();
        assert_eq!(a.directory, "C:/work/project");
        assert_eq!(a.name, "project");
        assert_eq!(
            Project::from_directory(r"\\server\share\one\..\app")
                .unwrap()
                .directory,
            "//server/share/app"
        );
        for invalid in ["", "app", "../app", "~/app", r"C:app", "null\0path"] {
            assert!(Project::from_directory(invalid).is_none(), "{invalid:?}");
        }
    }
}
