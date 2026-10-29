#[tauri::command]
pub async fn read_clipboard_text() -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || -> Result<Option<String>, String> {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| format!("CLIPBOARD_TEXT_ACCESS: {e}"))?;
        match clipboard.get_text() {
            Ok(text) => Ok(Some(text)),
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(e) => Err(format!("CLIPBOARD_TEXT_READ: {e}")),
        }
    })
    .await
    .map_err(|e| format!("CLIPBOARD_TEXT_TASK: {e}"))?
}

#[tauri::command]
pub async fn write_clipboard_text(text: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| format!("CLIPBOARD_TEXT_ACCESS: {e}"))?;
        clipboard
            .set_text(text)
            .map_err(|e| format!("CLIPBOARD_TEXT_WRITE: {e}"))
    })
    .await
    .map_err(|e| format!("CLIPBOARD_TEXT_TASK: {e}"))?
}
