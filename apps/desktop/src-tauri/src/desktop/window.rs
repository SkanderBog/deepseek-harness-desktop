use tauri::{webview::DownloadEvent, Emitter, Runtime, Url, Webview, WebviewWindow};
use tauri_plugin_opener::OpenerExt;
use tauri_runtime_electron::new_window::{NewWindowFeatures, NewWindowResponse};

use crate::config;
use crate::desktop::payload::DownloadFinishedPayload;

pub fn on_new_window(
    app_handle: tauri::AppHandle<tauri_runtime_electron::ElectronRuntime>,
    url: Url,
    _features: NewWindowFeatures,
) -> NewWindowResponse {
    if matches!(url.scheme(), "http" | "https") {
        if let Err(e) = app_handle.opener().open_url(url.to_string(), None::<&str>) {
            log::warn!("[new-window] open in browser failed {e}");
        }
    } else {
        log::debug!("[new-window] denied {url}");
    }
    NewWindowResponse::Deny
}

pub fn on_download<R: Runtime>(webview: Webview<R>, event: DownloadEvent<'_>) -> bool {
    match event {
        DownloadEvent::Requested { url, destination } => {
            *destination = config::unique_download_path(destination);
            log::info!("[download] requested {} -> {}", url, destination.display());
            true
        }
        DownloadEvent::Finished { url, path, success } => {
            let path_str = path.as_ref().map(|p| p.to_string_lossy().to_string());
            let payload = DownloadFinishedPayload {
                url: url.to_string(),
                path: path_str,
                success,
            };
            let _ = webview.emit("harness-download-finished", payload);
            log::info!(
                "[download] finished {} success={} path={:?}",
                url,
                success,
                path
            );
            true
        }
        _ => true,
    }
}

#[tauri::command]
pub fn toggle_devtools(window: WebviewWindow<tauri_runtime_electron::ElectronRuntime>) {
    if window.is_devtools_open() {
        window.close_devtools();
    } else {
        window.open_devtools();
    }
}
