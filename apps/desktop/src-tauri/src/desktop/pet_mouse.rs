//! macOS 仅订阅鼠标事件，避免后台线程调用 HIToolbox 导致崩溃（issue #397）；

use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{Emitter, State};
use crate::{WebviewWindow};

pub const PET_MOUSE_MOVE_EVENT: &str = "device-mouse-move";
pub const PET_MOUSE_BUTTON_EVENT: &str = "device-mouse-button";
const THROTTLE_INTERVAL: Duration = Duration::from_millis(16);
#[cfg(target_os = "windows")]
const CURSOR_POLL_INTERVAL: Duration = Duration::from_millis(8);

/// 崩溃链（issue #397）。
#[cfg(target_os = "macos")]
const MACOS_MOUSE_EVENTS: &[core_graphics::event::CGEventType] = &[
    core_graphics::event::CGEventType::MouseMoved,
    core_graphics::event::CGEventType::LeftMouseDragged,
    core_graphics::event::CGEventType::RightMouseDragged,
    core_graphics::event::CGEventType::ScrollWheel,
    core_graphics::event::CGEventType::OtherMouseDragged,
    core_graphics::event::CGEventType::LeftMouseDown,
    core_graphics::event::CGEventType::LeftMouseUp,
];

/// 接收窗口必须是**可变**的：收起桌宠会销毁窗口（issue #469），重新显示时创建的是
#[derive(Default)]
pub struct PetMouseStreamState {
    started: Arc<AtomicBool>,
    emitter: Arc<Mutex<Option<WebviewWindow>>>,
    revision: Arc<AtomicUsize>,
}

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
struct MouseCursorPos {
    x: f64,
    y: f64,
}

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
struct MouseButtonState {
    pressed: bool,
}

#[derive(Default)]
struct MouseStateStore {
    position: Mutex<Option<MouseCursorPos>>,
    pressed: Mutex<Option<bool>>,
}

impl MouseStateStore {
    fn write_position(&self, position: MouseCursorPos) {
        *self.position.lock().expect("pet mouse store poisoned") = Some(position);
    }

    fn write_pressed(&self, pressed: bool) {
        *self.pressed.lock().expect("pet mouse store poisoned") = Some(pressed);
    }

    fn read_position(&self) -> Option<MouseCursorPos> {
        *self.position.lock().expect("pet mouse store poisoned")
    }

    fn read_pressed(&self) -> Option<bool> {
        *self.pressed.lock().expect("pet mouse store poisoned")
    }
}

fn should_emit<T: PartialEq>(last: Option<&T>, current: &T, rebound: bool) -> bool {
    rebound || last != Some(current)
}

#[tauri::command]
pub fn start_pet_mouse_stream(window: WebviewWindow, state: State<'_, PetMouseStreamState>) {
    bind_pet_mouse_emitter(&state.emitter, &state.revision, window);
    if state.started.swap(true, Ordering::SeqCst) {
        return;
    }
    let started_on_error = state.started.clone();
    let shared: Arc<MouseStateStore> = Arc::default();

    let store = shared.clone();
    thread::spawn(move || {
        if let Err(error) = listen_mouse(store.clone()) {
            log::error!("[pet-mouse] mouse listen failed: {error:?}");
            started_on_error.store(false, Ordering::SeqCst);
        }
    });

    let emitter = state.emitter.clone();
    let revision = state.revision.clone();
    thread::spawn(move || {
        let mut last_sent: Option<MouseCursorPos> = None;
        let mut last_button: Option<bool> = None;
        let mut bound_revision = revision.load(Ordering::SeqCst);
        loop {
            let current_revision = revision.load(Ordering::SeqCst);
            let rebound = current_revision != bound_revision;
            bound_revision = current_revision;
            let position = shared
                .read_position()
                .filter(|pos| should_emit(last_sent.as_ref(), pos, rebound));
            let button = shared
                .read_pressed()
                .filter(|pressed| should_emit(last_button.as_ref(), pressed, rebound));
            if let Some(pos) = position {
                last_sent = Some(pos);
            }
            if let Some(pressed) = button {
                last_button = Some(pressed);
            }
            if position.is_some() || button.is_some() {
                let target = emitter
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone();
                if let Some(target) = target {
                    if let Some(pos) = position {
                        let _ = target.emit_to(
                            crate::desktop::pet::PET_WINDOW_LABEL,
                            PET_MOUSE_MOVE_EVENT,
                            pos,
                        );
                    }
                    if let Some(pressed) = button {
                        let _ = target.emit_to(
                            crate::desktop::pet::PET_WINDOW_LABEL,
                            PET_MOUSE_BUTTON_EVENT,
                            MouseButtonState { pressed },
                        );
                    }
                }
            }
            thread::sleep(THROTTLE_INTERVAL);
        }
    });
}

fn bind_pet_mouse_emitter(
    slot: &Arc<Mutex<Option<WebviewWindow>>>,
    revision: &Arc<AtomicUsize>,
    window: WebviewWindow,
) {
    let mut current = slot.lock().unwrap_or_else(|error| error.into_inner());
    *current = Some(window);
    revision.fetch_add(1, Ordering::SeqCst);
}

fn listen_mouse(store: Arc<MouseStateStore>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        listen_mouse_macos(store)
    }
    #[cfg(target_os = "windows")]
    {
        listen_mouse_windows(store)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        listen_mouse_rdev(store)
    }
}

#[cfg(target_os = "windows")]
fn listen_mouse_windows(store: Arc<MouseStateStore>) -> Result<(), String> {
    loop {
        if let Some(pos) = read_cursor_pos() {
            store.write_position(pos);
        }
        store.write_pressed(read_left_button_pressed());
        thread::sleep(CURSOR_POLL_INTERVAL);
    }
}

#[cfg(target_os = "windows")]
fn read_cursor_pos() -> Option<MouseCursorPos> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut point = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut point) } != 0 {
        Some(MouseCursorPos {
            x: point.x as f64,
            y: point.y as f64,
        })
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn read_left_button_pressed() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};

    unsafe { GetAsyncKeyState(VK_LBUTTON as i32) as u16 & 0x8000 != 0 }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn listen_mouse_rdev(store: Arc<MouseStateStore>) -> Result<(), String> {
    use rdev::{Button, EventType};

    let callback = move |event: rdev::Event| match event.event_type {
        EventType::MouseMove { x, y } => store.write_position(MouseCursorPos { x, y }),
        EventType::ButtonPress(Button::Left) => store.write_pressed(true),
        EventType::ButtonRelease(Button::Left) => store.write_pressed(false),
        _ => {}
    };
    rdev::listen(callback).map_err(|e| format!("rdev::listen: {e:?}"))
}

/// 系统单方面禁用事件 tap 时投递的带外事件类型（issue #559）。
#[cfg(target_os = "macos")]
fn tap_disabled_notice(event_type: core_graphics::event::CGEventType) -> bool {
    use core_graphics::event::CGEventType;
    matches!(
        event_type,
        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
    )
}

/// macOS CGEventTap 监听 mouse 事件（绕过 rdev 的 keyboard/TSM 路径，issue #397）。
/// # 为什么自己持有 tap（issue #559）
#[cfg(target_os = "macos")]
fn listen_mouse_macos(store: Arc<MouseStateStore>) -> Result<(), String> {
    use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
    use core_graphics::event::{
        CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
        CallbackResult,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    let tap_slot: Rc<RefCell<Option<CGEventTap<'static>>>> = Rc::new(RefCell::new(None));
    let rearm = tap_slot.clone();
    let event_tap: CGEventTap<'static> = unsafe {
        CGEventTap::new_unchecked(
            CGEventTapLocation::HID,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            MACOS_MOUSE_EVENTS.to_vec(),
            move |_proxy, event_type, event| {
                // 被系统禁用：立刻恢复，否则鼠标流永久停摆（issue #559）。
                if tap_disabled_notice(event_type) {
                    log::warn!(
                        "[pet-mouse] event tap disabled by system ({event_type:?}); re-enabling"
                    );
                    if let Some(tap) = rearm.borrow().as_ref() {
                        tap.enable();
                    }
                    return CallbackResult::Keep;
                }
                match event_type {
                    CGEventType::LeftMouseDown => store.write_pressed(true),
                    CGEventType::LeftMouseUp => store.write_pressed(false),
                    _ => {}
                }
                if matches!(
                    event_type,
                    CGEventType::MouseMoved
                        | CGEventType::LeftMouseDragged
                        | CGEventType::RightMouseDragged
                        | CGEventType::OtherMouseDragged
                        | CGEventType::LeftMouseDown
                        | CGEventType::LeftMouseUp
                ) {
                    let point = event.location();
                    store.write_position(MouseCursorPos {
                        x: point.x,
                        y: point.y,
                    });
                }
                CallbackResult::Keep
            },
        )
    }
    .map_err(|()| {
        "CGEventTap creation failed (accessibility permission denied or HID unavailable)"
            .to_string()
    })?;

    let loop_source = event_tap
        .mach_port()
        .create_runloop_source(0)
        .map_err(|()| "CGEventTap run loop source creation failed".to_string())?;
    CFRunLoop::get_current().add_source(&loop_source, unsafe { kCFRunLoopCommonModes });
    event_tap.enable();
    *tap_slot.borrow_mut() = Some(event_tap);
    CFRunLoop::run_current();
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use core_graphics::event::CGEventType;

    fn mask_codes() -> Vec<u32> {
        MACOS_MOUSE_EVENTS.iter().map(|e| *e as u32).collect()
    }

    /// SIGTRAP（issue #397）。
    #[test]
    fn macos_mouse_events_exclude_keyboard() {
        let codes = mask_codes();
        for forbidden in [
            CGEventType::KeyDown,
            CGEventType::KeyUp,
            CGEventType::FlagsChanged,
        ] {
            assert!(
                !codes.contains(&(forbidden as u32)),
                "MACOS_MOUSE_EVENTS must not include {forbidden:?} (issue #397)"
            );
        }
    }

    #[test]
    fn macos_mouse_events_include_mouse_moved() {
        assert!(mask_codes().contains(&(CGEventType::MouseMoved as u32)));
    }

    /// 系统禁用事件 tap 时投递的是带外类型，必须能识别出来并重新启用（issue #559）：
    #[test]
    fn tap_disabled_notices_are_recognized() {
        assert_eq!(CGEventType::TapDisabledByTimeout as u32, 0xFFFF_FFFE);
        assert_eq!(CGEventType::TapDisabledByUserInput as u32, 0xFFFF_FFFF);
        assert!(tap_disabled_notice(CGEventType::TapDisabledByTimeout));
        assert!(tap_disabled_notice(CGEventType::TapDisabledByUserInput));
        assert!(!tap_disabled_notice(CGEventType::MouseMoved));
        assert!(!tap_disabled_notice(CGEventType::ScrollWheel));
    }
}

#[cfg(all(test, target_os = "windows"))]
mod windows_tests {
    use super::*;

    #[test]
    fn get_cursor_pos_available_in_interactive_session() {
        if let Some(pos) = read_cursor_pos() {
            assert!(pos.x.is_finite() && pos.y.is_finite());
        } else {
            eprintln!("GetCursorPos unavailable (headless session); skipping");
        }
    }
}

#[cfg(test)]
mod button_tests {
    use super::*;

    #[test]
    fn stationary_cursor_is_replayed_after_window_rebind() {
        let store = MouseStateStore::default();
        let position = MouseCursorPos {
            x: -320.0,
            y: 240.0,
        };
        store.write_position(position);
        let sent = store.read_position();
        assert_eq!(sent, Some(position));
        assert_eq!(
            store
                .read_position()
                .filter(|pos| should_emit(sent.as_ref(), pos, false)),
            None
        );
        assert_eq!(
            store
                .read_position()
                .filter(|pos| should_emit(sent.as_ref(), pos, true)),
            Some(position)
        );
    }

    #[test]
    fn unchanged_button_is_replayed_after_window_rebind() {
        for pressed in [false, true] {
            let store = MouseStateStore::default();
            store.write_pressed(pressed);
            let sent = store.read_pressed();
            assert_eq!(sent, Some(pressed));
            assert_eq!(
                store
                    .read_pressed()
                    .filter(|value| should_emit(sent.as_ref(), value, false)),
                None
            );
            assert_eq!(
                store
                    .read_pressed()
                    .filter(|value| should_emit(sent.as_ref(), value, true)),
                Some(pressed)
            );
        }
    }

    #[test]
    fn should_emit_only_on_change_or_rebound() {
        let pressed = MouseButtonState { pressed: true };
        let released = MouseButtonState { pressed: false };
        assert!(should_emit(None, &pressed, false));
        assert!(!should_emit(Some(&pressed), &pressed, false));
        assert!(should_emit(Some(&pressed), &released, false));
        assert!(should_emit(Some(&pressed), &pressed, true));

        let pos = MouseCursorPos { x: 1.0, y: 2.0 };
        assert!(should_emit(None, &pos, false));
        assert!(!should_emit(Some(&pos), &pos, false));
        assert!(should_emit(
            Some(&pos),
            &MouseCursorPos { x: 1.0, y: 3.0 },
            false
        ));
    }

    #[test]
    fn store_keeps_position_and_button_independent() {
        let store = MouseStateStore::default();
        let pos = MouseCursorPos { x: 3.0, y: 4.0 };
        assert_eq!(store.read_position(), None);
        assert_eq!(store.read_pressed(), None);

        store.write_position(pos);
        assert_eq!(store.read_pressed(), None);
        assert_eq!(store.read_position(), Some(pos));
        assert_eq!(store.read_position(), Some(pos));

        store.write_pressed(false);
        assert_eq!(store.read_position(), Some(pos));
        assert_eq!(store.read_pressed(), Some(false));
        assert_eq!(store.read_pressed(), Some(false));

        store.write_pressed(true);
        store.write_position(pos);
        assert_eq!(store.read_position(), Some(pos));
        assert_eq!(store.read_pressed(), Some(true));
    }
}
