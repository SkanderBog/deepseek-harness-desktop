
pub use crate::config::{CLOSE_ACTION_QUIT, CLOSE_ACTION_TRAY};

#[cfg(target_os = "macos")]
use std::sync::{Mutex, OnceLock};

use tauri::{Runtime, Window};
#[cfg(target_os = "macos")]
use tauri::{ActivationPolicy, Manager};
use crate::{AppHandle, WebviewWindow};

#[cfg(any(target_os = "macos", test))]
pub fn should_switch_to_accessory(is_fullscreen: bool, close_action: &str) -> bool {
    !is_fullscreen && close_action == CLOSE_ACTION_TRAY
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum PolicyState {
    Regular,
    Accessory,
}

#[cfg(target_os = "macos")]
static CURRENT_POLICY: OnceLock<Mutex<Option<PolicyState>>> = OnceLock::new();

#[cfg(target_os = "macos")]
static PENDING_ACCESSORY: Mutex<bool> = Mutex::new(false);

#[cfg(target_os = "macos")]
fn set_pending_accessory(pending: bool) {
    *PENDING_ACCESSORY
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = pending;
}

#[cfg(target_os = "macos")]
fn take_pending_accessory() -> bool {
    let mut pending = PENDING_ACCESSORY
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    std::mem::replace(&mut *pending, false)
}

#[cfg(target_os = "macos")]
fn apply_policy<R: Runtime>(app: &AppHandle<R>, state: PolicyState) {
    let mut cached = CURRENT_POLICY
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if *cached == Some(state) {
        return;
    }
    let policy = match state {
        PolicyState::Regular => ActivationPolicy::Regular,
        PolicyState::Accessory => ActivationPolicy::Accessory,
    };
    match app.set_activation_policy(policy) {
        Ok(()) => *cached = Some(state),
        Err(error) => log::warn!("[activation] ACTIVATION_POLICY_FAILED: {error}"),
    }
}

#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn on_window_hidden<R: Runtime>(window: &Window<R>, close_action: &str) {
    #[cfg(target_os = "macos")]
    {
        let Ok(is_fullscreen) = window.is_fullscreen() else {
            return;
        };
        if should_switch_to_accessory(is_fullscreen, close_action) {
            apply_policy(window.app_handle(), PolicyState::Accessory);
            return;
        }
        if is_fullscreen {
            set_pending_accessory(true);
            log::warn!("[activation] 全屏态关窗，推迟到退出全屏后再切 Accessory");
        }
    }
}

#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn on_window_resized<R: Runtime>(window: &Window<R>) {
    #[cfg(target_os = "macos")]
    {
        let Ok(false) = window.is_fullscreen() else {
            return;
        };
        if take_pending_accessory() {
            apply_policy(window.app_handle(), PolicyState::Accessory);
        }
    }
}

#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn set_regular_policy<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    {
        set_pending_accessory(false);
        apply_policy(app, PolicyState::Regular);
    }
}

#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn rearm_pending_accessory_if_fullscreen<R: Runtime>(window: &WebviewWindow<R>) {
    #[cfg(target_os = "macos")]
    {
        let Ok(is_fullscreen) = window.is_fullscreen() else {
            return;
        };
        if !is_fullscreen {
            return;
        }
        let close_action = crate::config::get_store_dat_setting(&window.app_handle()).close_action;
        if should_switch_to_accessory(false, &close_action) {
            set_pending_accessory(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{should_switch_to_accessory, CLOSE_ACTION_QUIT, CLOSE_ACTION_TRAY};

    #[test]
    fn close_action_constants_match_store_values() {
        assert_eq!(CLOSE_ACTION_TRAY, "tray", "托盘驻留动作必须是 tray");
        assert_eq!(CLOSE_ACTION_QUIT, "quit", "退出动作必须是 quit");
    }

    #[test]
    fn closes_to_tray_hides_dock_when_not_fullscreen() {
        assert!(
            should_switch_to_accessory(false, CLOSE_ACTION_TRAY),
            "普通关窗应当切 Accessory 隐藏 Dock"
        );
    }

    #[test]
    fn fullscreen_close_keeps_dock_until_exit_fullscreen() {
        assert!(
            !should_switch_to_accessory(true, CLOSE_ACTION_TRAY),
            "全屏态关窗必须保持 regular，退出全屏后再补切"
        );
    }

    #[test]
    fn quit_action_never_switches_policy() {
        assert!(
            !should_switch_to_accessory(false, CLOSE_ACTION_QUIT),
            "要退出的关窗不需要切 Accessory"
        );
        assert!(
            !should_switch_to_accessory(true, CLOSE_ACTION_QUIT),
            "全屏且要退出时同样不切 Accessory"
        );
    }

    #[test]
    fn unknown_close_action_keeps_dock_visible() {
        assert!(
            !should_switch_to_accessory(false, ""),
            "空动作应保守降级为不切，Dock 保留"
        );
        assert!(
            !should_switch_to_accessory(false, "TRAY"),
            "大小写不一致的动作不识别，Dock 保留"
        );
        assert!(
            !should_switch_to_accessory(false, "bogus"),
            "未知动作应保守降级为不切，Dock 保留"
        );
    }
}
