use crate::config;
use tauri::{AppHandle, Runtime};

#[tauri::command]
pub async fn get_app_config<R: Runtime>(
    app_handle: AppHandle<R>,
) -> Result<config::Setting, String> {
    Ok(config::get_store_dat_setting(&app_handle))
}

#[tauri::command]
pub fn is_dev_build() -> bool {
    cfg!(debug_assertions)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn update_app_config<R: Runtime>(
    app_handle: AppHandle<R>,
    proxy_url: Option<String>,
    appearance: Option<config::Appearance>,
    port: Option<u16>,
    zoom_factor: Option<f64>,
    harness_max_heap_mb: Option<u32>,
    auto_start: Option<bool>,
    auto_update: Option<bool>,
    cli_link_enabled: Option<bool>,
    close_action: Option<String>,
    backup_retention_count: Option<u32>,
    backup_include_credentials: Option<bool>,
) -> Result<config::Setting, String> {
    let proxy_url = proxy_url
        .as_deref()
        .map(config::proxy::normalize_proxy_url)
        .transpose()?;
    if let Some(port) = port {
        if port == 0 {
            return Err("port must be a positive number".to_string());
        }
    }
    if cli_link_enabled.is_some() {
        return Err(
            "CLI_LINK_UNAVAILABLE: command-line integration has not been migrated".to_string(),
        );
    }
    let setting = config::update_store_dat_setting(&app_handle, |setting| {
        if let Some(proxy_url) = proxy_url {
            setting.proxy_url = proxy_url;
        }
        if let Some(appearance) = appearance {
            setting.appearance = appearance;
        }
        if let Some(port) = port {
            setting.port = port;
            setting.manual_port = Some(port);
        }
        if let Some(zoom_factor) = zoom_factor {
            setting.zoom_factor = zoom_factor;
        }
        if let Some(mb) = harness_max_heap_mb {
            setting.harness_max_heap_mb = (mb != 0).then_some(mb);
        }
        if let Some(auto_start) = auto_start {
            setting.auto_start = auto_start;
        }
        if let Some(auto_update) = auto_update {
            setting.auto_update = auto_update;
        }
        if let Some(action) = close_action {
            setting.close_action = action;
        }
        if let Some(count) = backup_retention_count {
            setting.backup_retention_count = count;
        }
        if let Some(include) = backup_include_credentials {
            setting.backup_include_credentials = include;
        }
    });
    if zoom_factor.is_some() {
        use tauri::Manager;
        for window in app_handle.webview_windows().into_values() {
            window.set_zoom(setting.zoom_factor).map_err(|error| error.to_string())?;
        }
    }
    Ok(setting)
}

#[tauri::command]
pub fn get_launch_on_login<R: Runtime>(app_handle: AppHandle<R>) -> Result<bool, String> {
    crate::desktop::autostart::is_enabled(&app_handle)
}

#[tauri::command]
pub fn set_launch_on_login<R: Runtime>(app_handle: AppHandle<R>, enabled: bool) -> Result<bool, String> {
    crate::desktop::autostart::set_enabled(&app_handle, enabled)
}

#[tauri::command]
pub fn set_language<R: Runtime>(app_handle: AppHandle<R>, lang: String) {
    let mut setting = config::get_store_dat_setting(&app_handle);
    setting.language = lang.clone();
    config::set_store_dat_setting(&app_handle, setting);
    config::i18n::set_language(config::i18n::lang_from_setting(&lang));
}
