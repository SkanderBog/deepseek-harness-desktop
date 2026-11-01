#[tauri::command]
pub async fn read_clipboard_text() -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(|| {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|error| format!("CLIPBOARD_TEXT_ACCESS: {error}"))?;
        match clipboard.get_text() {
            Ok(text) => Ok(Some(text)),
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(error) => Err(format!("CLIPBOARD_TEXT_READ: {error}")),
        }
    })
    .await
    .map_err(|error| format!("CLIPBOARD_TEXT_TASK: {error}"))?
}

#[tauri::command]
pub async fn write_clipboard_text(text: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|error| format!("CLIPBOARD_TEXT_ACCESS: {error}"))?;
        clipboard
            .set_text(text)
            .map_err(|error| format!("CLIPBOARD_TEXT_WRITE: {error}"))
    })
    .await
    .map_err(|error| format!("CLIPBOARD_TEXT_TASK: {error}"))?
}
