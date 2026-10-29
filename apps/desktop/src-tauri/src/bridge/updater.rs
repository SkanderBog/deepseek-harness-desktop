
use crate::service::update;
use crate::{AppHandle};

#[tauri::command]
pub async fn check_desktop_update(
    app_handle: AppHandle,
) -> Result<Option<update::DesktopUpdateInfo>, String> {
    update::check(&app_handle).await
}

#[tauri::command]
pub async fn download_desktop_update(
    app_handle: AppHandle,
) -> Result<update::DesktopUpdateInfo, String> {
    update::download(&app_handle).await
}

#[tauri::command]
pub async fn open_desktop_installer(app_handle: AppHandle, path: String) -> Result<(), String> {
    update::open_installer(&app_handle, path).await
}

#[tauri::command]
pub fn is_desktop_distro_managed() -> bool {
    update::distro_managed()
}

#[tauri::command]
pub async fn get_desktop_about(app_handle: AppHandle) -> Result<update::DesktopAboutInfo, String> {
    Ok(update::about(&app_handle).await)
}
