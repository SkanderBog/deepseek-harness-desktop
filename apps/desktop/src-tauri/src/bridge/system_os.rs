use crate::logger;
use tauri::{AppHandle, Runtime};
use tauri_plugin_opener::OpenerExt;

/// 在系统文件管理器中定位指定文件（Session 日志下载完成后的"在文件夹中显示"）
#[tauri::command]
pub fn reveal_in_folder<R: Runtime>(app_handle: AppHandle<R>, path: String) -> Result<(), String> {
    if !crate::bridge::guard::is_allowed_path(&app_handle, std::path::Path::new(&path)) {
        return Err(format!("REVEAL_PATH_REJECTED: {path}"));
    }
    tauri_plugin_opener::reveal_item_in_dir(&path).map_err(|e| format!("REVEAL_FAILED: {e}"))
}

/// 在系统文件管理器中打开指定目录（核心版本「打开目录」按钮；目录用 open 而非
/// reveal——reveal 是定位父目录，open 是直接打开该目录本身）。
#[tauri::command]
pub fn open_dir<R: Runtime>(app_handle: AppHandle<R>, path: String) -> Result<(), String> {
    if !crate::bridge::guard::is_allowed_path(&app_handle, std::path::Path::new(&path)) {
        return Err(format!("OPEN_DIR_REJECTED: {path}"));
    }
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(|e| format!("OPEN_DIR_FAILED: {e}"))
}

/// 在系统浏览器中打开任意 http(s) 链接（更新说明 / 关于对话框仓库链接等）
#[tauri::command]
pub async fn open_external_url<R: Runtime>(
    app_handle: AppHandle<R>,
    url: String,
) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(format!("EXTERNAL_URL_INVALID: {url}"));
    }
    app_handle
        .opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

/// 前端日志透传：前端 `console.*` 劫持经此命令落盘到 `desktop.frontdesk.log`
/// （与持有后端 + `dsh` target 的 `desktop.log` 分离），见 `logger/mod.rs`。
#[tauri::command]
pub fn log_frontend(level: String, target: String, message: String) {
    let lvl = logger::FrontendLevel::from_str(&level);
    logger::log_frontend(lvl, &target, &message);
}
