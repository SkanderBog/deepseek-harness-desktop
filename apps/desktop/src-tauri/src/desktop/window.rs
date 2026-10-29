use tauri::{webview::{DownloadEvent, Webview}, Emitter, Runtime, Url};
use tauri_plugin_opener::OpenerExt;
use tauri_runtime_electron::{
    new_window::{NewWindowFeatures, NewWindowResponse},
    ElectronRuntime,
};

use crate::config;
use crate::desktop::payload::DownloadFinishedPayload;
use crate::{AppHandle, WebviewWindow};

pub fn on_new_window(
    app_handle: AppHandle<ElectronRuntime>,
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

const NAVIGATION_GLOBAL: &str = "__DSH_NAVIGATION__";

const PRESENTATION_FULL_SIDEBAR: &str = "full-sidebar";

#[tauri::command]
pub async fn create_app_window(
    app_handle: AppHandle<ElectronRuntime>,
    window: WebviewWindow<ElectronRuntime>,
    session_id: Option<String>,
    presentation: Option<String>,
) -> Result<(), String> {
    if let Some(value) = presentation.as_deref() {
        if value != PRESENTATION_FULL_SIDEBAR {
            return Err("WINDOW_CREATE_FAILED: unsupported presentation".into());
        }
        if session_id.is_none() {
            return Err("WINDOW_CREATE_FAILED: presentation requires sessionId".into());
        }
    }
    log::info!(
        "[window] create_app_window from {}: session={session_id:?} presentation={presentation:?}",
        window.label()
    );
    let navigation = session_id
        .as_deref()
        .map(|session_id| session_navigation_script(session_id, presentation.as_deref()))
        .transpose()?;
    let _transition = crate::service::workflow::acquire_core_transition().await?;
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::desktop::builder::build_extra_window(
        &app_handle,
        "index.html".into(),
        &profile,
        navigation.as_deref(),
    )
    .map(|_| ())
    .map_err(|error| format!("WINDOW_CREATE_FAILED: {error}"))
}

fn session_navigation_script(
    session_id: &str,
    presentation: Option<&str>,
) -> Result<String, String> {
    if session_id.trim().is_empty() || session_id.len() > 512 {
        return Err("WINDOW_CREATE_FAILED: invalid sessionId".into());
    }
    let payload = match presentation {
        None => serde_json::json!({ "sessionId": session_id }),
        Some(PRESENTATION_FULL_SIDEBAR) => serde_json::json!({
            "sessionId": session_id,
            "presentation": PRESENTATION_FULL_SIDEBAR,
        }),
        Some(_) => return Err("WINDOW_CREATE_FAILED: unsupported presentation".into()),
    };
    Ok(format!("globalThis.{NAVIGATION_GLOBAL} = {payload};"))
}

#[cfg(test)]
mod session_navigation_tests {
    use super::{session_navigation_script, NAVIGATION_GLOBAL, PRESENTATION_FULL_SIDEBAR};

    fn payload_of(script: &str) -> serde_json::Value {
        let assignment = format!("globalThis.{NAVIGATION_GLOBAL} = ");
        let payload = script
            .strip_prefix(assignment.as_str())
            .and_then(|rest| rest.strip_suffix(';'))
            .expect("script is one global assignment");
        serde_json::from_str(payload).expect("payload is json")
    }

    #[test]
    fn session_navigation_script_assigns_the_session_target_to_the_shared_global() {
        assert_eq!(
            session_navigation_script("session &/?#", None).unwrap(),
            r#"globalThis.__DSH_NAVIGATION__ = {"sessionId":"session &/?#"};"#
        );
    }

    #[test]
    fn session_navigation_script_escapes_values_through_json_serialization() {
        assert_eq!(
            session_navigation_script("a\"b\\c\nd", None).unwrap(),
            r#"globalThis.__DSH_NAVIGATION__ = {"sessionId":"a\"b\\c\nd"};"#
        );
    }

    #[test]
    fn session_navigation_script_carries_the_full_sidebar_presentation() {
        let payload = payload_of(
            &session_navigation_script("session-1", Some(PRESENTATION_FULL_SIDEBAR)).unwrap(),
        );
        assert_eq!(payload["sessionId"], "session-1");
        assert_eq!(payload["presentation"], PRESENTATION_FULL_SIDEBAR);
    }

    #[test]
    fn session_navigation_script_rejects_unknown_presentations() {
        assert_eq!(
            session_navigation_script("session-1", Some("detached")),
            Err("WINDOW_CREATE_FAILED: unsupported presentation".into())
        );
    }

    #[test]
    fn session_navigation_script_rejects_empty_blank_or_oversized_identifiers() {
        for id in ["", " ", " \t\n", &"s".repeat(513)] {
            assert_eq!(
                session_navigation_script(id, None),
                Err("WINDOW_CREATE_FAILED: invalid sessionId".into())
            );
        }
        assert!(session_navigation_script(&"s".repeat(512), None).is_ok());
    }
}

#[tauri::command]
pub fn quit_app(app_handle: AppHandle<ElectronRuntime>) {
    app_handle.exit(0);
}

#[tauri::command]
pub fn toggle_devtools(window: WebviewWindow<ElectronRuntime>) {
    if window.is_devtools_open() {
        window.close_devtools();
    } else {
        window.open_devtools();
    }
}
