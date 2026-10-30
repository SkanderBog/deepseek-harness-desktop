use crate::service::core;
use crate::{AppHandle, WebviewWindow};

#[tauri::command]
pub async fn get_cores(app_handle: AppHandle) -> Vec<core::HarnessCore> {
    core::list(&app_handle).await
}

#[tauri::command]
pub async fn set_active_core(
    window: WebviewWindow,
    app_handle: AppHandle,
    id: String,
) -> Result<core::HarnessCore, String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, core::set_active(&app_handle, &id)).await
}

#[tauri::command]
pub async fn download_core(
    window: WebviewWindow,
    app_handle: AppHandle,
    tag: String,
) -> Result<core::HarnessCore, String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, core::download_version(&app_handle, &tag))
        .await
}

#[tauri::command]
pub async fn remove_core(app_handle: AppHandle, id: String) -> Result<(), String> {
    core::remove_version(&app_handle, &id).await
}

#[tauri::command]
pub async fn update_local_core(
    window: WebviewWindow,
    app_handle: AppHandle,
) -> Result<String, String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {
        core::update_local_core(app_handle).await
    })
    .await
}
