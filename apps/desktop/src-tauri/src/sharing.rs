use std::io::Write;
use tauri::Emitter;
#[tauri::command]
pub fn open_share_dialog(app: tauri::AppHandle) -> Result<(), String> {
    crate::windows::show_dashboard(&app)?;
    app.emit_to("main", "open-share-dialog", ()).map_err(|error| { eprintln!("Share dialog event failed: {error}"); "The share dialog could not be opened.".into() })
}
fn draft_url(text: &str) -> Result<String, String> {
    if text.trim().is_empty() || text.len() > 12000 { return Err("Enter a shorter post before opening X.".into()); }
    let mut url = reqwest::Url::parse("https://x.com/intent/post").map_err(|_| "The X draft address is invalid.")?;
    url.query_pairs_mut().append_pair("text", text);
    Ok(url.into())
}
#[tauri::command]
pub async fn open_share_draft(text: String) -> Result<(), String> {
    let url = draft_url(&text)?;
    open::that(url).map_err(|error| { eprintln!("Share browser launch failed: {error}"); "Could not open X in your browser. Copy the post text and try again.".into() })
}
#[tauri::command]
pub async fn save_share_image(bytes: Vec<u8>) -> Result<String, String> {
    if bytes.len() > 12 * 1024 * 1024 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") { return Err("The share image is invalid or too large.".into()); }
    let name = format!("DevClash-Share-{}.png", chrono::Utc::now().format("%Y%m%d-%H%M%S-%f"));
    let directory = dirs::download_dir().ok_or("Downloads folder could not be located.")?;
    let path = directory.join(&name);
    let mut options = std::fs::OpenOptions::new(); options.write(true).create_new(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    let mut file = options.open(&path).map_err(|error| { eprintln!("Share file creation failed: {error}"); "Could not save to Downloads. Check folder access and free space." })?;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        eprintln!("Share image write failed: {error}"); drop(file); let _ = std::fs::remove_file(&path);
        return Err("The share image could not be saved. Check free space and try again.".into());
    }
    Ok(name)
}
#[cfg(test)] mod tests {
    #[test] fn draft_cannot_change_destination() {
        let text = "Hi @DevClashAI &url=https://example.com/#private";
        let url = reqwest::Url::parse(&super::draft_url(text).unwrap()).unwrap();
        assert_eq!(url.host_str(), Some("x.com")); assert_eq!(url.path(), "/intent/post");
        assert_eq!(url.query_pairs().collect::<Vec<_>>(), vec![("text".into(), text.into())]);
        assert!(super::draft_url("").is_err()); assert!(super::draft_url(&"x".repeat(12001)).is_err());
    }
}
