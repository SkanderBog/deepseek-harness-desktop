
use crate::config;
use crate::service::cli;
use crate::service::update;
use crate::{AppHandle, WebviewWindow};

#[tauri::command]
pub async fn get_app_config(
    window: WebviewWindow,
    app_handle: AppHandle,
) -> Result<config::Setting, String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {
        let mut setting = config::get_store_dat_setting(&app_handle);
        setting.active_profile = crate::service::profile::active_profile(&app_handle);
        setting.port = crate::service::workflow::instances::port(&app_handle);
        Ok(setting)
    })
    .await
}

#[tauri::command]
pub fn get_effective_heap_limit_mb(app_handle: AppHandle) -> Option<u32> {
    crate::service::workflow::effective_heap_limit_mb(
        config::get_store_dat_setting(&app_handle).harness_max_heap_mb,
    )
}

#[tauri::command]
pub fn is_dev_build() -> bool {
    cfg!(debug_assertions)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn update_app_config(
    window: WebviewWindow,
    app_handle: AppHandle,
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
    if let Some(enabled) = cli_link_enabled {
        if enabled {
            cli::ensure(&app_handle)?;
        } else {
            cli::remove(&app_handle)?;
        }
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
            // 一路顶高（issue #91，见 workflow::launch 的端口自愈逻辑）
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
        if let Some(enabled) = cli_link_enabled {
            setting.cli_link_enabled = enabled;
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
    if auto_update == Some(false) {
        update::clear_pending_installer(&app_handle);
    }
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    let mut setting = setting;
    setting.active_profile = profile.clone();
    setting.port = crate::service::workflow::instances::scope_sync(profile, || {
        crate::service::workflow::instances::port(&app_handle)
    });
    Ok(setting)
}

#[tauri::command]
pub fn get_launch_on_login(app_handle: AppHandle) -> Result<bool, String> {
    crate::desktop::autostart::is_enabled(&app_handle)
}

#[tauri::command]
pub fn set_launch_on_login(app_handle: AppHandle, enabled: bool) -> Result<bool, String> {
    crate::desktop::autostart::set_enabled(&app_handle, enabled)
}

#[tauri::command]
pub fn get_cli_link_status(app_handle: AppHandle) -> Result<cli::CliLinkStatus, String> {
    Ok(cli::get_status(&app_handle))
}

#[tauri::command]
pub fn set_language(app_handle: AppHandle, lang: String) {
    let mut setting = config::get_store_dat_setting(&app_handle);
    setting.language = lang.clone();
    config::set_store_dat_setting(&app_handle, setting);
    config::i18n::set_language(config::i18n::lang_from_setting(&lang));
    #[cfg(target_os = "macos")]
    if let Err(error) = crate::desktop::builder::install_macos_menu(&app_handle) {
        log::warn!("[menu] failed to refresh macOS menu language: {error}");
    }
}

#[tauri::command]
pub async fn toggle_sidebar() -> Result<bool, String> {
    Ok(true)
}

#[tauri::command]
pub async fn test_proxy(app_handle: AppHandle) -> Result<config::proxy::ProxyTestResult, String> {
    Ok(config::proxy::test(
        &config::get_store_dat_setting(&app_handle).proxy_url,
        config::PROXY_TEST_URL,
        config::PROXY_TEST_TIMEOUT,
    )
    .await)
}

#[tauri::command]
pub fn get_dsh_theme(app_handle: AppHandle) -> config::DshTheme {
    config::get_dsh_theme(&app_handle)
}
