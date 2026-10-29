use tauri::menu::{Menu, MenuEvent, MenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, Runtime, WebviewWindow, WebviewWindowBuilder};
use tauri_runtime_electron::ElectronRuntime;

use crate::utils::show_main_window;

pub const MAIN_WINDOW_LABEL: &str = "main";

/// 构建系统托盘。
///
/// 托盘是可选壳层能力：Linux 桌面没有 SNI watcher（例如未装 AppIndicator 扩展的
/// GNOME）或会话总线不可用时只记日志，不让应用启动失败。
pub fn tray<R: Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<()> {
    #[cfg(target_os = "linux")]
    if let Err(error) = build_tray(app) {
        log::warn!("[tray] LINUX_TRAY_FAILED: {error}");
    }
    #[cfg(not(target_os = "linux"))]
    build_tray(app)?;
    Ok(())
}

/// Linux 后端由 Cargo.toml 里 `tray-icon` 的 `ksni` 特性选定（tauri 自己只开
/// libappindicator，而后者在 Linux 上不上报托盘事件，见 issue #386）；tauri 在
/// `App::run` 里给同一个 tray-icon / muda 实例装全局处理器，因此 Linux 上同样能收到
/// 单击与菜单事件。
fn build_tray<R: Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<()> {
    // 平台差异的托盘图标策略：
    // - macOS：使用 scoped template 透明图标（NSImage template），由系统按菜单栏
    //   深浅/半透明材质自动着色，呈现与系统一致的半透明玻璃观感，而非彩色方块。
    // - 其他平台：沿用默认窗口图标。
    #[cfg(target_os = "macos")]
    let icon = tauri::image::Image::from_bytes(include_bytes!("../../icons/macos-tray.png"))?;
    #[cfg(not(target_os = "macos"))]
    let icon = app.default_window_icon().unwrap().clone();

    // 构建菜单
    let open_label = crate::config::i18n::t("menu.open_panel");
    let quit_label = crate::config::i18n::t("menu.quit");
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "open", open_label, true, None::<&str>)?,
            &MenuItem::with_id(app, "quit", quit_label, true, None::<&str>)?,
        ],
    )?;

    fn handle_menu_event<R: Runtime>(app: &tauri::AppHandle<R>, event: &MenuEvent) {
        match event.id().as_ref() {
            "open" => show_main_window(app),
            "quit" => {
                app.exit(0);
            }
            _ => {}
        }
    }

    fn handle_tray_icon_event<R: Runtime>(tray: &tauri::tray::TrayIcon<R>, event: &TrayIconEvent) {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            ..
        } = event
        {
            show_main_window(tray.app_handle());
        }
    }

    // 构建托盘图标。macOS 上把模板图标记为 NSImage template，由系统按菜单栏
    // 深浅/半透明材质自动着色，呈现与系统一致的半透明玻璃观感。
    #[cfg(target_os = "macos")]
    let _ = TrayIconBuilder::new()
        .icon(icon)
        .icon_as_template(true)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip(crate::desktop::product_name(app))
        .on_menu_event(move |app, event| handle_menu_event(app, &event))
        .on_tray_icon_event(move |tray, event| handle_tray_icon_event(tray, &event))
        .build(app)?;

    #[cfg(not(target_os = "macos"))]
    let _ = TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip(crate::desktop::product_name(app))
        .on_menu_event(move |app, event| handle_menu_event(app, &event))
        .on_tray_icon_event(move |tray, event| handle_tray_icon_event(tray, &event))
        .build(app)?;

    Ok(())
}

pub fn build_main_window(
    app: &tauri::AppHandle<ElectronRuntime>,
) -> tauri::Result<WebviewWindow<ElectronRuntime>> {
    let _geometry_restore = crate::config::GeometryRestoreGuard::begin();
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|config| config.label == MAIN_WINDOW_LABEL)
        .ok_or(tauri::Error::WindowNotFound)?;
    let handle = app.clone();
    let window = WebviewWindowBuilder::from_config(app, config)?
        .initialization_script(format!(
            "window.__DSH_STORE_FILE__ = {};",
            serde_json::json!(crate::config::store_dat_file_name())
        ))
        .visible(false)
        .on_download(crate::desktop::window::on_download)
        .build()?;
    tauri_runtime_electron::on_new_window(window.as_ref(), move |url, features| {
        crate::desktop::window::on_new_window(handle.clone(), url, features)
    })?;
    crate::config::restore_main_window(app, &window);
    let zoom_factor = crate::config::get_store_dat_setting(app).zoom_factor;
    if zoom_factor != crate::config::default_zoom_factor() {
        if let Err(error) = window.set_zoom(zoom_factor) {
            log::warn!("[zoom] startup zoom was not applied: {error}");
        }
    }
    window.show()?;
    Ok(window)
}

pub fn builder() -> tauri::Builder<ElectronRuntime> {
    let builder = tauri::Builder::<ElectronRuntime>::new();
    #[cfg(all(not(windows), not(debug_assertions)))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        show_main_window(app);
    }));
    let builder = builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_http::init());
    #[cfg(target_os = "macos")]
    let in_app_bundle = std::env::current_exe()
        .map(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"))
        .unwrap_or(false);
    #[cfg(not(target_os = "macos"))]
    let in_app_bundle = true;
    let builder = if in_app_bundle {
        builder.plugin(tauri_plugin_notifications::init())
    } else {
        log::warn!("[notification] native notifications require a packaged macOS application");
        builder
    };
    #[cfg(not(debug_assertions))]
    let builder = builder.plugin(tauri_plugin_deep_link::init());
    builder
        .setup(|app| {
            crate::config::detect_first_install(app.handle());
            build_main_window(app.handle())?;
            let setting = crate::config::get_store_dat_setting(app.handle());
            crate::config::i18n::set_language(crate::config::i18n::lang_from_setting(
                &setting.language,
            ));
            crate::desktop::autostart::init(app.handle())?;
            if let Err(error) = crate::desktop::tray(app.handle()) {
                log::warn!("[tray] TRAY_FAILED: {error}");
            }
            #[cfg(not(debug_assertions))]
            crate::desktop::deep_link::init(app.handle());
            Ok(())
        })
        .on_window_event(on_window_event)
}

pub(crate) fn on_window_event<R: Runtime>(window: &tauri::Window<R>, event: &tauri::WindowEvent) {
    if window.label() != MAIN_WINDOW_LABEL {
        return;
    }
    match event {
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
            crate::config::save_geometry(window);
            #[cfg(target_os = "macos")]
            if matches!(event, tauri::WindowEvent::Resized(_)) {
                crate::desktop::activation::on_window_resized(window);
            }
        }
        tauri::WindowEvent::CloseRequested { api, .. } => {
            let close_action =
                crate::config::get_store_dat_setting(window.app_handle()).close_action;
            if close_action == crate::config::CLOSE_ACTION_QUIT {
                window.app_handle().exit(0);
                return;
            }
            api.prevent_close();
            crate::config::save_geometry(window);
            #[cfg(target_os = "macos")]
            {
                crate::desktop::activation::on_window_hidden(window, &close_action);
                if let Err(error) = window.app_handle().hide() {
                    crate::desktop::activation::set_regular_policy(window.app_handle());
                    log::error!("[activation] APP_HIDE_FAILED: {error}");
                }
            }
            #[cfg(not(target_os = "macos"))]
            if let Err(error) = window.hide() {
                log::error!("[window] WINDOW_HIDE_FAILED: {error}");
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
    use tauri_plugin_store::StoreExt;

    fn isolated_app() -> (tempfile::TempDir, tauri::App<MockRuntime>) {
        let scratch = tempfile::tempdir().unwrap();
        let mut context = mock_context(noop_assets());
        context.config_mut().app.app_directories_override = Some(
            tauri::utils::config::AppDirectoriesOverride::Root(scratch.path().to_path_buf()),
        );
        let app = mock_builder()
            .plugin(tauri_plugin_store::Builder::default().build())
            .build(context)
            .unwrap();
        assert_eq!(app.path().app_data_dir().unwrap(), scratch.path());
        (scratch, app)
    }

    #[test]
    fn main_close_to_tray_prevents_destruction() {
        let (_scratch, app) = isolated_app();
        let store = app.store(crate::config::store_dat_file_name()).unwrap();
        store.set(
            "setting",
            serde_json::json!({
                "installed": false, "port": 4099, "auto_start": false,
                "language": "en", "close_action": "tray"
            }),
        );
        let window =
            WebviewWindowBuilder::new(&app, MAIN_WINDOW_LABEL, tauri::WebviewUrl::default())
                .visible(true)
                .build()
                .unwrap();
        let (signal_tx, signal_rx) = mpsc::channel();
        let event = tauri_runtime::window::WindowEvent::CloseRequested { signal_tx }.into();

        on_window_event(&app.get_window(window.label()).unwrap(), &event);

        assert_eq!(signal_rx.try_recv(), Ok(true), "主窗口关闭必须被阻止");
        assert!(app.get_webview_window(MAIN_WINDOW_LABEL).is_some());
    }

    #[test]
    fn secondary_close_is_not_intercepted_by_main_window_policy() {
        let (_scratch, app) = isolated_app();
        let store = app.store(crate::config::store_dat_file_name()).unwrap();
        store.set(
            "setting",
            serde_json::json!({
                "installed": false, "port": 4099, "auto_start": false,
                "language": "en", "close_action": "tray"
            }),
        );
        let window =
            WebviewWindowBuilder::new(&app, "window-secondary", tauri::WebviewUrl::default())
                .build()
                .unwrap();
        let (signal_tx, signal_rx) = mpsc::channel();
        let event = tauri_runtime::window::WindowEvent::CloseRequested { signal_tx }.into();

        on_window_event(&app.get_window(window.label()).unwrap(), &event);

        assert_eq!(signal_rx.try_recv(), Err(mpsc::TryRecvError::Empty));
    }
}
