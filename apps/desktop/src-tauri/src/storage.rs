use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

/// Only call for the app-owned data root, never a shared parent directory.
pub fn private_directory(path: &Path) -> Result<(), String> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|e| e.to_string())?;
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Local data directory must be an ordinary directory.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn private_file(path: &Path, write: bool) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(!write).write(write).create(write);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Local storage must be an ordinary file.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.nlink() != 1 {
            return Err("Local storage must not be a linked file.".into());
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    // Never truncate before checking the opened file and securing its mode.
    if write {
        file.set_len(0).map_err(|e| e.to_string())?;
    }
    Ok(file)
}
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let mut error = "Saved data is unavailable".to_string();
    for candidate in [
        path.to_path_buf(),
        path.with_extension("tmp"),
        path.with_extension("bak"),
    ] {
        match private_file(&candidate, false).and_then(|mut file| {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())
        }) {
            Ok(value) => return Ok(value),
            Err(reason) => error = reason,
        }
    }
    Err(error)
}
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    static WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = WRITES
        .lock()
        .map_err(|_| "Local storage could not be opened.".to_owned())?;
    let temporary = path.with_extension("tmp");
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    let mut file = private_file(&temporary, true)?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    #[cfg(target_os = "windows")]
    if path.exists() {
        let backup = path.with_extension("bak");
        if backup.exists() {
            std::fs::remove_file(&backup).map_err(|e| e.to_string())?;
        }
        std::fs::rename(path, &backup).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&temporary, path).map_err(|e| e.to_string())?;
    let backup = path.with_extension("bak");
    if backup.exists() {
        std::fs::remove_file(backup).map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_save_recovers_a_complete_pending_queue() {
        let path =
            std::env::temp_dir().join(format!("devclash-storage-{}.json", std::process::id()));
        std::fs::write(&path, b"{broken").unwrap();
        std::fs::write(path.with_extension("tmp"), b"[1,2]").unwrap();
        assert_eq!(read_json::<Vec<u64>>(&path).unwrap(), vec![1, 2]);
        write_json(&path, &vec![3u64]).unwrap();
        assert_eq!(read_json::<Vec<u64>>(&path).unwrap(), vec![3]);
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn saved_and_recovered_data_is_private_without_changing_shared_parents() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("devclash-private-{}", std::process::id()));
        let parent_mode = std::fs::metadata(std::env::temp_dir())
            .unwrap()
            .permissions()
            .mode();
        private_directory(&root).unwrap();
        assert_eq!(
            std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let path = root.join("cache.json");
        write_json(&path, &vec![1u64]).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(read_json::<Vec<u64>>(&path).unwrap(), vec![1]);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::write(path.with_extension("tmp"), b"[2]").unwrap();
        std::fs::set_permissions(
            path.with_extension("tmp"),
            std::fs::Permissions::from_mode(0o666),
        )
        .unwrap();
        write_json(&path, &vec![3u64]).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(std::env::temp_dir())
                .unwrap()
                .permissions()
                .mode(),
            parent_mode
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_linked_temporary_files_without_truncating_the_target() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!("devclash-links-{}", std::process::id()));
        private_directory(&root).unwrap();
        let victim = root.join("unrelated.json");
        std::fs::write(&victim, b"preserve").unwrap();
        let path = root.join("cache.json");
        symlink(&victim, path.with_extension("tmp")).unwrap();
        assert!(write_json(&path, &vec![1u64]).is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"preserve");
        std::fs::remove_file(path.with_extension("tmp")).unwrap();
        std::fs::hard_link(&victim, path.with_extension("tmp")).unwrap();
        assert!(write_json(&path, &vec![1u64]).is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"preserve");
        std::fs::remove_dir_all(root).unwrap();
    }
}
