//! 动画宠物（桌宠外置化，issue #308）。
//!   遮挡（issue #649，上游 tauri-apps/tao#1134 已按「Wayland 不支持」补文档结案）。

use crate::config::{store_dat_file_name, STORE_PET_WINDOW_STATE_KEY};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{Duration, Instant};
#[cfg(not(target_os = "macos"))]
use tauri::PhysicalPosition;
use tauri::{Manager, Runtime, WebviewUrl, WebviewWindowBuilder, Window};
use tauri_plugin_store::StoreExt;
use crate::{AppHandle, WebviewWindow};

pub const PET_WINDOW_LABEL: &str = "pet";
pub const PET_SPRITE_BASE_WIDTH: f64 = 220.0;
const PET_WINDOW_PAD_X: f64 = 32.0;
const PET_WINDOW_TOP_PAD: f64 = 72.0;
const PET_WINDOW_BOTTOM_PAD: f64 = 10.0;
const PET_WINDOW_MIN_WIDTH: f64 = 420.0;
const PET_BUILTIN_ASPECT: f64 = 9.0 / 16.0;
const PET_CUSTOM_ASPECT: f64 = 208.0 / 192.0;
pub const PET_SIZE_MIN_PERCENT: f64 = 50.0;
pub const PET_SIZE_MAX_PERCENT: f64 = 200.0;
pub const PET_SIZE_DEFAULT_PERCENT: f64 = 100.0;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PetWindowPosition {
    pub x: Option<i32>,
    pub y: Option<i32>,
    #[cfg(target_os = "macos")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_factor: Option<f64>,
}

#[cfg(target_os = "macos")]
impl PetWindowPosition {
    fn logical_position(&self, fallback_scale: f64) -> Option<(f64, f64)> {
        let scale = self
            .scale_factor
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(fallback_scale);
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        Some((f64::from(self.x?) / scale, f64::from(self.y?) / scale))
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy)]
struct PetRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[cfg(target_os = "macos")]
impl PetRect {
    fn from_physical(position: (i32, i32), size: (u32, u32), scale: f64) -> Self {
        Self {
            x: f64::from(position.0) / scale,
            y: f64::from(position.1) / scale,
            width: f64::from(size.0) / scale,
            height: f64::from(size.1) / scale,
        }
    }

    fn intersects(&self, other: &Self) -> bool {
        self.x < other.x + other.width
            && self.x + self.width > other.x
            && self.y < other.y + other.height
            && self.y + self.height > other.y
    }

    fn constrained_position(&self, monitors: &[Self]) -> Option<(f64, f64)> {
        let center_x = self.x + self.width / 2.0;
        let center_y = self.y + self.height / 2.0;
        let distance = |monitor: &Self| {
            let dx = (monitor.x - center_x)
                .max(center_x - monitor.x - monitor.width)
                .max(0.0);
            let dy = (monitor.y - center_y)
                .max(center_y - monitor.y - monitor.height)
                .max(0.0);
            dx * dx + dy * dy
        };
        let monitor = monitors
            .iter()
            .find(|monitor| {
                center_x >= monitor.x
                    && center_x < monitor.x + monitor.width
                    && center_y >= monitor.y
                    && center_y < monitor.y + monitor.height
            })
            .or_else(|| {
                monitors
                    .iter()
                    .min_by(|a, b| distance(a).total_cmp(&distance(b)))
            })?;
        Some((
            self.x.clamp(
                monitor.x,
                (monitor.x + monitor.width - self.width).max(monitor.x),
            ),
            self.y.clamp(
                monitor.y,
                (monitor.y + monitor.height - self.height).max(monitor.y),
            ),
        ))
    }
}

#[cfg(target_os = "macos")]
impl From<&tauri::Monitor> for PetRect {
    fn from(monitor: &tauri::Monitor) -> Self {
        let position = monitor.position();
        let size = monitor.size();
        Self::from_physical(
            (position.x, position.y),
            (size.width, size.height),
            monitor.scale_factor(),
        )
    }
}

pub fn get_pet_window_position<R: Runtime>(app: &AppHandle<R>) -> PetWindowPosition {
    let store = app
        .store(store_dat_file_name())
        .expect("Failed to load store for pet window position");
    let raw = store.get(STORE_PET_WINDOW_STATE_KEY);
    raw.as_ref()
        .and_then(|value| {
            serde_json::from_value(crate::config::unwrap_json_value(value).into_owned()).ok()
        })
        .unwrap_or_default()
}

/// 照写会把上一次在 X11 会话里存下的有效位置覆盖成原点（issue #649）。
pub fn save_pet_window_position<R: Runtime>(app: &AppHandle<R>, position: &PetWindowPosition) {
    let store = app
        .store(store_dat_file_name())
        .expect("Failed to load store for pet window position");
    let serialized =
        serde_json::to_value(position).expect("Failed to serialize pet window position");
    store.set(STORE_PET_WINDOW_STATE_KEY, serialized);
    store.save().expect("Failed to save pet window position");
}

pub fn save_pet_window_geometry<R: Runtime>(window: &Window<R>) {
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    let Some(pos) = window.outer_position().ok() else {
        return;
    };
    let app = window.app_handle().clone();
    save_pet_window_position_throttled(
        &app,
        &PetWindowPosition {
            x: Some(pos.x),
            y: Some(pos.y),
            #[cfg(target_os = "macos")]
            scale_factor: window
                .scale_factor()
                .ok()
                .filter(|scale| scale.is_finite() && *scale > 0.0),
        },
    );
}

const PET_GEOMETRY_SAVE_INTERVAL: Duration = Duration::from_millis(500);

static PET_GEOMETRY_SAVE: Mutex<Option<(Instant, PetWindowPosition)>> = Mutex::new(None);

fn save_pet_window_position_throttled<R: Runtime>(
    app: &AppHandle<R>,
    position: &PetWindowPosition,
) {
    let mut pending = PET_GEOMETRY_SAVE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let now = Instant::now();
    if let Some((last, slot)) = pending.as_mut() {
        *slot = position.clone();
        if now.duration_since(*last) < PET_GEOMETRY_SAVE_INTERVAL {
            return;
        }
        *last = now;
    } else {
        *pending = Some((now, position.clone()));
    }
    drop(pending);
    save_pet_window_position(app, position);
}

pub fn get_pet_size_percent<R: Runtime>(app: &AppHandle<R>) -> f64 {
    crate::config::get_store_dat_setting(app)
        .pet_size
        .unwrap_or(PET_SIZE_DEFAULT_PERCENT)
        .clamp(PET_SIZE_MIN_PERCENT, PET_SIZE_MAX_PERCENT)
}

pub fn pet_window_aspect<R: Runtime>(app: &AppHandle<R>) -> f64 {
    let setting = crate::config::get_store_dat_setting(app);
    let active = setting
        .active_pet
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if active.map(|value| !value.contains(':')).unwrap_or(true) {
        PET_BUILTIN_ASPECT
    } else {
        PET_CUSTOM_ASPECT
    }
}

pub fn pet_window_logical_size(percent: f64, aspect: f64) -> (f64, f64) {
    let scale = percent / 100.0;
    (
        ((PET_SPRITE_BASE_WIDTH * scale) + PET_WINDOW_PAD_X).max(PET_WINDOW_MIN_WIDTH),
        (PET_SPRITE_BASE_WIDTH * aspect * scale) + PET_WINDOW_TOP_PAD + PET_WINDOW_BOTTOM_PAD,
    )
}

pub fn apply_pet_size<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(PET_WINDOW_LABEL) else {
        return;
    };
    let (width, height) =
        pet_window_logical_size(get_pet_size_percent(app), pet_window_aspect(app));
    let size: tauri::Size = tauri::LogicalSize::new(width, height).into();
    let _ = window.set_size(size);
}

#[allow(clippy::too_many_arguments)]
#[cfg(any(not(target_os = "macos"), test))]
fn clamp_window_position(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    monitor_x: i32,
    monitor_y: i32,
    monitor_width: u32,
    monitor_height: u32,
) -> (i32, i32) {
    let left = i64::from(monitor_x);
    let top = i64::from(monitor_y);
    let right = left + i64::from(monitor_width);
    let bottom = top + i64::from(monitor_height);
    let max_x = (right - i64::from(width)).max(left);
    let max_y = (bottom - i64::from(height)).max(top);
    (
        i64::from(x).clamp(left, max_x) as i32,
        i64::from(y).clamp(top, max_y) as i32,
    )
}

pub fn move_pet_window<R: Runtime>(
    app: &AppHandle<R>,
    delta_x: i32,
    delta_y: i32,
) -> Result<(), String> {
    let window = app
        .get_webview_window(PET_WINDOW_LABEL)
        .ok_or_else(|| "PET_WINDOW_NOT_FOUND: pet window has not been created".to_string())?;
    let current = window.outer_position().map_err(|error| {
        format!("PET_WINDOW_POSITION_FAILED: failed to read pet window position: {error}")
    })?;
    let size = window.outer_size().map_err(|error| {
        format!("PET_WINDOW_SIZE_FAILED: failed to read pet window size: {error}")
    })?;
    let monitors = app.available_monitors().map_err(|error| {
        format!("PET_MONITOR_LIST_FAILED: failed to list visible monitors: {error}")
    })?;
    if monitors.is_empty() {
        return Err("PET_MONITOR_NOT_FOUND: no visible monitor is available".to_string());
    }
    #[cfg(target_os = "macos")]
    {
        let scale = window.scale_factor().map_err(|error| {
            format!("PET_WINDOW_SCALE_FAILED: failed to read pet window scale: {error}")
        })?;
        let desired = PetRect::from_physical(
            (
                current.x.saturating_add(delta_x),
                current.y.saturating_add(delta_y),
            ),
            (size.width, size.height),
            scale,
        );
        let monitors: Vec<_> = monitors.iter().map(PetRect::from).collect();
        let (x, y) = desired
            .constrained_position(&monitors)
            .expect("非空显示器列表应能选择目标显示器");
        window
            .set_position(tauri::LogicalPosition::new(x, y))
            .map_err(|error| {
                format!("PET_WINDOW_MOVE_FAILED: failed to move pet window: {error}")
            })?;
        persist_pet_window_position(app)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let desired_x = current.x.saturating_add(delta_x);
        let desired_y = current.y.saturating_add(delta_y);
        let center_x = i64::from(desired_x) + i64::from(size.width) / 2;
        let center_y = i64::from(desired_y) + i64::from(size.height) / 2;
        let monitor = monitors
            .iter()
            .find(|monitor| {
                let pos = monitor.position();
                let area = monitor.size();
                let right = i64::from(pos.x) + i64::from(area.width);
                let bottom = i64::from(pos.y) + i64::from(area.height);
                center_x >= i64::from(pos.x)
                    && center_x < right
                    && center_y >= i64::from(pos.y)
                    && center_y < bottom
            })
            .or_else(|| {
                monitors.iter().min_by_key(|monitor| {
                    let pos = monitor.position();
                    let area = monitor.size();
                    let left = i64::from(pos.x);
                    let top = i64::from(pos.y);
                    let right = left + i64::from(area.width);
                    let bottom = top + i64::from(area.height);
                    let dx = if center_x < left {
                        left - center_x
                    } else if center_x >= right {
                        center_x - right + 1
                    } else {
                        0
                    };
                    let dy = if center_y < top {
                        top - center_y
                    } else if center_y >= bottom {
                        center_y - bottom + 1
                    } else {
                        0
                    };
                    dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy))
                })
            })
            .expect("非空显示器列表应能选择目标显示器");
        let monitor_pos = monitor.position();
        let monitor_size = monitor.size();
        let (x, y) = clamp_window_position(
            desired_x,
            desired_y,
            size.width,
            size.height,
            monitor_pos.x,
            monitor_pos.y,
            monitor_size.width,
            monitor_size.height,
        );
        window
            .set_position(tauri::Position::Physical(PhysicalPosition::new(x, y)))
            .map_err(|error| {
                format!("PET_WINDOW_MOVE_FAILED: failed to move pet window: {error}")
            })?;
        save_pet_window_position(
            app,
            &PetWindowPosition {
                x: Some(x),
                y: Some(y),
            },
        );
        Ok(())
    }
}

pub fn persist_pet_window_position<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let window = app
        .get_webview_window(PET_WINDOW_LABEL)
        .ok_or_else(|| "PET_WINDOW_NOT_FOUND: pet window has not been created".to_string())?;
    let current = window.outer_position().map_err(|error| {
        format!("PET_WINDOW_POSITION_FAILED: failed to read pet window position: {error}")
    })?;
    save_pet_window_position(
        app,
        &PetWindowPosition {
            x: Some(current.x),
            y: Some(current.y),
            #[cfg(target_os = "macos")]
            scale_factor: window
                .scale_factor()
                .ok()
                .filter(|scale| scale.is_finite() && *scale > 0.0),
        },
    );
    PET_GEOMETRY_SAVE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    Ok(())
}

pub fn ensure_pet_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<WebviewWindow<R>> {
    if let Some(window) = app.get_webview_window(PET_WINDOW_LABEL) {
        return Ok(window);
    }
    let app_handle = app.clone();
    let (width, height) =
        pet_window_logical_size(get_pet_size_percent(app), pet_window_aspect(app));
    #[allow(unused_mut)]
    let mut builder =
        WebviewWindowBuilder::new(app, PET_WINDOW_LABEL, WebviewUrl::App("pet.html".into()))
            .title("Deepseek Harness Pet")
            .inner_size(width, height)
            .resizable(false)
            .maximizable(false)
            .transparent(true)
            .always_on_top(true)
            .decorations(false)
            .skip_taskbar(true)
            .shadow(false)
            .accept_first_mouse(true)
            .visible(false);

    builder = builder
        .initialization_script_for_all_frames(crate::desktop::notification::NOTIFICATION_SHIM_JS)
        .initialization_script_for_all_frames(crate::desktop::paste::PASTE_SHIM_JS);

    let window = builder.build()?;
    let app_for_pos = app_handle.clone();
    let saved = get_pet_window_position(&app_for_pos);
    #[cfg(target_os = "macos")]
    let saved_position = saved
        .logical_position(window.scale_factor().unwrap_or(1.0))
        .map(|(x, y)| tauri::Position::Logical(tauri::LogicalPosition::new(x, y)));
    #[cfg(not(target_os = "macos"))]
    let saved_position = saved
        .x
        .zip(saved.y)
        .map(|(x, y)| tauri::Position::Physical(PhysicalPosition::new(x, y)));
    if let Some(position) =
        saved_position.filter(|position| position_on_any_monitor(&window, *position))
    {
        let _ = window.set_position(position);
    } else {
        place_pet_at_default(&window);
    }

    Ok(window)
}

fn position_on_any_monitor<R: Runtime>(
    window: &WebviewWindow<R>,
    position: tauri::Position,
) -> bool {
    let app = window.app_handle();
    let Ok(monitors) = app.available_monitors() else {
        return false;
    };
    let scale = window.scale_factor().unwrap_or(1.0);
    let (lw, lh) = pet_window_logical_size(get_pet_size_percent(app), pet_window_aspect(app));
    #[cfg(target_os = "macos")]
    {
        let position = position.to_logical::<f64>(scale);
        let rect = PetRect {
            x: position.x,
            y: position.y,
            width: lw,
            height: lh,
        };
        monitors
            .iter()
            .map(PetRect::from)
            .any(|monitor| rect.intersects(&monitor))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let position = position.to_physical::<i32>(scale);
        let (x, y) = (position.x, position.y);
        let (w, h) = ((lw * scale) as i32, (lh * scale) as i32);
        monitors.iter().any(|monitor| {
            let pos = monitor.position();
            let size = monitor.size();
            x < pos.x + size.width as i32
                && x + w > pos.x
                && y < pos.y + size.height as i32
                && y + h > pos.y
        })
    }
}

fn place_pet_at_default<R: Runtime>(window: &WebviewWindow<R>) {
    let app = window.app_handle();
    let Some(monitor) = app.primary_monitor().ok().flatten() else {
        return;
    };
    let (lw, lh) = pet_window_logical_size(get_pet_size_percent(app), pet_window_aspect(app));
    #[cfg(target_os = "macos")]
    {
        let rect = PetRect::from(&monitor);
        let scale = monitor.scale_factor();
        let x = rect.x + rect.width - lw - 32.0 / scale;
        let y = rect.y + rect.height - lh - 96.0 / scale;
        let _ = window.set_position(tauri::LogicalPosition::new(x, y));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mon_pos = monitor.position();
        let mon_size = monitor.size();
        let (w, h) = (
            (lw * monitor.scale_factor()) as i32,
            (lh * monitor.scale_factor()) as i32,
        );
        let x = mon_pos.x + mon_size.width as i32 - w - 32;
        let y = mon_pos.y + mon_size.height as i32 - h - 96;
        let _ = window.set_position(tauri::Position::Physical(PhysicalPosition::new(x, y)));
    }
}

/// # 隐藏 = 销毁窗口实例（issue #469）
pub fn set_pet_window_visible<R: Runtime>(app: &AppHandle<R>, visible: bool) -> Result<(), String> {
    if !visible {
        if let Some(window) = app.get_webview_window(PET_WINDOW_LABEL) {
            if let Err(error) = persist_pet_window_position(app) {
                log::warn!("[pet] PET_POSITION_PERSIST_FAILED: {error}");
            }
            window
                .destroy()
                .map_err(|error| format!("PET_WINDOW_DESTROY_FAILED: {error}"))?;
        }
        return Ok(());
    }
    let window =
        ensure_pet_window(app).map_err(|error| format!("PET_WINDOW_CREATE_FAILED: {error}"))?;
    window
        .show()
        .map_err(|error| format!("PET_WINDOW_SHOW_FAILED: {error}"))?;
    Ok(())
}

pub fn init_pet_window<R: Runtime>(app: &AppHandle<R>) {
    if !crate::config::get_store_dat_setting(app).pet_enabled {
        return;
    }
    if let Err(error) = set_pet_window_visible(app, true) {
        log::error!("PET_WINDOW_INIT_FAILED: failed to create pet window: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pet_window_position_serde_roundtrip() {
        let pos = PetWindowPosition {
            x: Some(120),
            y: Some(240),
            ..PetWindowPosition::default()
        };
        let json = serde_json::to_string(&pos).expect("serialize");
        let parsed: PetWindowPosition = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.x, Some(120));
        assert_eq!(parsed.y, Some(240));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn pet_window_position_preserves_scale_for_cross_monitor_restore() {
        let saved: PetWindowPosition =
            serde_json::from_str(r#"{"x":-5760,"y":820,"scale_factor":2.0}"#).expect("deserialize");
        let serialized = serde_json::to_value(saved).expect("serialize");
        assert_eq!(serialized["scale_factor"], serde_json::json!(2.0));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_saved_position_uses_original_scale_after_restart_on_another_monitor() {
        let saved: PetWindowPosition =
            serde_json::from_str(r#"{"x":3600,"y":600,"scale_factor":2.0}"#).expect("deserialize");
        assert_eq!(saved.logical_position(1.0), Some((1800.0, 300.0)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_legacy_position_uses_current_scale_as_limited_fallback() {
        let saved: PetWindowPosition =
            serde_json::from_str(r#"{"x":3000,"y":600}"#).expect("deserialize");
        assert_eq!(saved.logical_position(2.0), Some((1500.0, 300.0)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_invalid_saved_scale_uses_valid_fallback() {
        for scale in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            let saved = PetWindowPosition {
                x: Some(-4800),
                y: Some(-400),
                scale_factor: Some(scale),
            };
            assert_eq!(saved.logical_position(2.0), Some((-2400.0, -200.0)));
            assert_eq!(saved.logical_position(0.0), Some((-4800.0, -400.0)));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_incomplete_saved_position_cannot_be_restored() {
        let saved = PetWindowPosition {
            x: Some(120),
            ..PetWindowPosition::default()
        };
        assert_eq!(saved.logical_position(2.0), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_move_from_retina_to_standard_screen_uses_each_monitor_scale() {
        let desired = PetRect::from_physical((3600, 600), (840, 412), 2.0);
        let monitors = [
            PetRect::from_physical((0, 0), (3420, 2224), 2.0),
            PetRect::from_physical((1710, 0), (2560, 1440), 1.0),
        ];
        assert_eq!(
            desired.constrained_position(&monitors),
            Some((1800.0, 300.0))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_move_to_negative_retina_screen_clamps_in_logical_points() {
        let desired = PetRect::from_physical((-2540, 1100), (420, 206), 1.0);
        let monitors = [
            PetRect::from_physical((0, 0), (1920, 1080), 1.0),
            PetRect::from_physical((-5120, -400), (5120, 2880), 2.0),
        ];
        assert_eq!(
            desired.constrained_position(&monitors),
            Some((-2540.0, 1034.0))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_move_reaches_third_screen_above_primary() {
        let desired = PetRect::from_physical((200, -1600), (840, 412), 2.0);
        let monitors = [
            PetRect::from_physical((0, 0), (1920, 1080), 1.0),
            PetRect::from_physical((-2560, 0), (2560, 1440), 1.0),
            PetRect::from_physical((0, -2160), (3840, 2160), 2.0),
        ];
        assert_eq!(
            desired.constrained_position(&monitors),
            Some((100.0, -800.0))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_window_in_screen_gap_uses_nearest_monitor() {
        let desired = PetRect::from_physical((3800, 600), (840, 412), 2.0);
        let monitors = [
            PetRect::from_physical((0, 0), (3420, 2224), 2.0),
            PetRect::from_physical((2200, 0), (1920, 1080), 1.0),
        ];
        assert_eq!(
            desired.constrained_position(&monitors),
            Some((2200.0, 300.0))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_visibility_does_not_confuse_retina_primary_with_standard_secondary() {
        let window = PetRect::from_physical((3600, 600), (840, 412), 2.0);
        let primary = PetRect::from_physical((0, 0), (3420, 2224), 2.0);
        let secondary = PetRect::from_physical((1710, 0), (2560, 1440), 1.0);
        assert!(!window.intersects(&primary));
        assert!(window.intersects(&secondary));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_oversized_window_aligns_with_monitor_origin() {
        let window = PetRect::from_physical((100, 100), (900, 700), 1.0);
        let monitor = PetRect::from_physical((-1600, -1200), (1600, 1200), 2.0);
        assert_eq!(
            window.constrained_position(&[monitor]),
            Some((-800.0, -600.0))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_window_cannot_be_constrained_without_a_monitor() {
        let window = PetRect::from_physical((0, 0), (840, 412), 2.0);
        assert_eq!(window.constrained_position(&[]), None);
    }

    #[test]
    fn pet_window_position_defaults_to_none() {
        let parsed: PetWindowPosition = serde_json::from_str("{}").expect("deserialize");
        assert!(parsed.x.is_none());
        assert!(parsed.y.is_none());

        let default = PetWindowPosition::default();
        assert!(default.x.is_none() && default.y.is_none());
    }

    #[test]
    fn pet_window_logical_size_scales_with_percent() {
        for percent in [50.0, 100.0, 200.0] {
            let (width, height) = pet_window_logical_size(percent, PET_CUSTOM_ASPECT);
            let scale = percent / 100.0;
            assert_eq!(
                width,
                (PET_SPRITE_BASE_WIDTH * scale + PET_WINDOW_PAD_X).max(PET_WINDOW_MIN_WIDTH)
            );
            assert_eq!(
                height,
                PET_SPRITE_BASE_WIDTH * PET_CUSTOM_ASPECT * scale + 82.0
            );
        }
        let (_, builtin_height) = pet_window_logical_size(100.0, PET_BUILTIN_ASPECT);
        assert_eq!(
            builtin_height,
            PET_SPRITE_BASE_WIDTH * PET_BUILTIN_ASPECT + 82.0
        );
    }

    #[test]
    fn pet_window_aspect_matches_builtin_and_custom() {
        assert_eq!(PET_BUILTIN_ASPECT, 9.0 / 16.0);
        assert_eq!(PET_CUSTOM_ASPECT, 208.0 / 192.0);
        let is_builtin = |active: Option<&str>| {
            active
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| !v.contains(':'))
                .unwrap_or(true)
        };
        assert!(is_builtin(None));
        assert!(is_builtin(Some("   ")));
        assert!(is_builtin(Some("maid-deepseek-whale")));
        assert!(is_builtin(Some("another-preset")));
        assert!(!is_builtin(Some("codex:blue_whale")));
        assert!(!is_builtin(Some("chat:cat")));
    }

    #[test]
    fn pet_window_width_crosses_bubble_minimum_at_large_sizes() {
        assert_eq!(
            pet_window_logical_size(50.0, PET_BUILTIN_ASPECT),
            (420.0, 143.875)
        );
        assert_eq!(
            pet_window_logical_size(100.0, PET_BUILTIN_ASPECT),
            (420.0, 205.75)
        );
        assert_eq!(
            pet_window_logical_size(200.0, PET_BUILTIN_ASPECT),
            (472.0, 329.5)
        );
    }

    #[test]
    fn clamp_window_position_handles_extreme_coordinates_and_sizes() {
        assert_eq!(
            clamp_window_position(i32::MAX, i32::MIN, 20, 10, -1920, -1080, 1920, 1080),
            (-20, -1080)
        );
        assert_eq!(
            clamp_window_position(0, 0, u32::MAX, u32::MAX, i32::MIN, i32::MIN, 0, 0),
            (i32::MIN, i32::MIN)
        );
    }

    #[test]
    fn clamp_window_position_keeps_entire_pet_on_monitor() {
        assert_eq!(
            clamp_window_position(1900, 1000, 252, 244, 0, 0, 1920, 1080),
            (1668, 836)
        );
        assert_eq!(
            clamp_window_position(-2000, -500, 252, 244, -1920, 0, 1920, 1080),
            (-1920, 0)
        );
        assert_eq!(
            clamp_window_position(100, 100, 900, 700, 0, 0, 800, 600),
            (0, 0),
            "窗口大于显示器时应贴齐显示器左上角"
        );
    }
}
