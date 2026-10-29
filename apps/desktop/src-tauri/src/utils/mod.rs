use tauri::{AppHandle, Manager, Runtime, WebviewWindow};

pub fn show_window<R: Runtime>(window: &WebviewWindow<R>) {
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

pub fn shell_window<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    app.get_webview_window("main").or_else(|| {
        app.webview_windows().into_values().find(|window| {
            window.label().starts_with("window-") || window.label().starts_with("remote-")
        })
    })
}

pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = shell_window(app) {
        #[cfg(target_os = "macos")]
        if let Err(error) = app.show() {
            log::warn!("[window] APP_SHOW_FAILED: {error}");
        }
        #[cfg(target_os = "macos")]
        crate::desktop::activation::set_regular_policy(app);
        show_window(&window);
        #[cfg(target_os = "macos")]
        crate::desktop::activation::rearm_pending_accessory_if_fullscreen(&window);
    } else {
        log::warn!("[window] MAIN_WINDOW_NOT_FOUND");
    }
}
