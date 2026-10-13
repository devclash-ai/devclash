use serde::{de::DeserializeOwned, Serialize};
use std::{io::Write, path::Path};
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let mut error = "Saved data is unavailable".to_string();
    for candidate in [
        path.to_path_buf(),
        path.with_extension("tmp"),
        path.with_extension("bak"),
    ] {
        match std::fs::read(&candidate)
            .map_err(|e| e.to_string())
            .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string()))
        {
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
    let mut file = std::fs::File::create(&temporary).map_err(|e| e.to_string())?;
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
}
