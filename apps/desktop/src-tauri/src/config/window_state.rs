
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{Manager, PhysicalPosition, PhysicalSize, Position, Runtime, Size, Window};
use tauri_plugin_store::StoreExt;

use super::constants::STORE_WINDOW_STATE_KEY;
use super::setting::store_dat_file_name;
use crate::{AppHandle, WebviewWindow};

pub const DEFAULT_WINDOW_WIDTH: f64 = 1280.0;
pub const DEFAULT_WINDOW_HEIGHT: f64 = 840.0;
pub const MIN_WINDOW_WIDTH: f64 = 860.0;
pub const MIN_WINDOW_HEIGHT: f64 = 620.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub maximized: bool,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub size_is_inner: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            maximized: false,
            x: None,
            y: None,
            width: DEFAULT_WINDOW_WIDTH as u32,
            height: DEFAULT_WINDOW_HEIGHT as u32,
            size_is_inner: true,
        }
    }
}

/// 于是「每次启动都要重新拉伸」（issue #464）。这里按恢复窗口期统一门控所有平台，
static RESTORING_GEOMETRY: AtomicBool = AtomicBool::new(false);

pub struct GeometryRestoreGuard;

impl GeometryRestoreGuard {
    pub fn begin() -> Self {
        RESTORING_GEOMETRY.store(true, Ordering::SeqCst);
        Self
    }
}

impl Drop for GeometryRestoreGuard {
    fn drop(&mut self) {
        RESTORING_GEOMETRY.store(false, Ordering::SeqCst);
    }
}

fn is_restoring_geometry() -> bool {
    RESTORING_GEOMETRY.load(Ordering::SeqCst)
}

pub fn get_window_state<R: Runtime>(app_handle: &AppHandle<R>) -> WindowState {
    let store = app_handle
        .store(store_dat_file_name())
        .expect("Failed to load store for window state");
    let raw = store.get(STORE_WINDOW_STATE_KEY);
    raw.as_ref()
        .and_then(|value| serde_json::from_value(super::unwrap_json_value(value).into_owned()).ok())
        .unwrap_or_default()
}

fn save_window_state<R: Runtime>(app_handle: &AppHandle<R>, state: &WindowState) {
    let store = app_handle
        .store(store_dat_file_name())
        .expect("Failed to load store for window state");
    let serialized = serde_json::to_value(state).expect("Failed to serialize window state");
    store.set(STORE_WINDOW_STATE_KEY, serialized);
    store.save().expect("Failed to save window state");
}

pub fn save_geometry<R: Runtime>(window: &Window<R>) {
    if is_restoring_geometry() {
        return;
    }

    if window.is_fullscreen().unwrap_or(false) {
        return;
    }

    if !window.is_visible().unwrap_or(false) {
        return;
    }

    let pos = window.outer_position().ok();
    let size = window.inner_size().ok();
    let state = WindowState {
        maximized: window.is_maximized().unwrap_or(false),
        x: pos.map(|p| p.x),
        y: pos.map(|p| p.y),
        width: size.map(|s| s.width).unwrap_or(DEFAULT_WINDOW_WIDTH as u32),
        height: size
            .map(|s| s.height)
            .unwrap_or(DEFAULT_WINDOW_HEIGHT as u32),
        size_is_inner: true,
    };
    save_window_state(window.app_handle(), &state);
}

pub fn save_main_window_geometry<R: Runtime>(app_handle: &AppHandle<R>) {
    if let Some(webview_window) = app_handle.get_webview_window("main") {
        let window = webview_window.as_ref().window();
        save_geometry(&window);
    }
}

fn migrate_legacy_outer_size(
    saved: &WindowState,
    current_outer: PhysicalSize<u32>,
    current_inner: PhysicalSize<u32>,
) -> WindowState {
    if saved.size_is_inner {
        return saved.clone();
    }

    let frame_width = current_outer.width.saturating_sub(current_inner.width);
    let frame_height = current_outer.height.saturating_sub(current_inner.height);
    WindowState {
        width: saved.width.saturating_sub(frame_width),
        height: saved.height.saturating_sub(frame_height),
        size_is_inner: true,
        ..saved.clone()
    }
}

fn is_degenerate_size(saved: &WindowState) -> bool {
    (saved.width as f64) <= MIN_WINDOW_WIDTH && (saved.height as f64) <= MIN_WINDOW_HEIGHT
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScreenRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl ScreenRect {
    fn right(&self) -> i32 {
        self.x.saturating_add(self.width as i32)
    }

    fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height as i32)
    }

    fn intersects(&self, other: &ScreenRect) -> bool {
        self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }
}

fn union_rect<I>(rects: I) -> Option<ScreenRect>
where
    I: IntoIterator<Item = ScreenRect>,
{
    let mut iter = rects.into_iter();
    let mut union = iter.next()?;
    for rect in iter {
        let left = union.x.min(rect.x);
        let top = union.y.min(rect.y);
        let right = union.right().max(rect.right());
        let bottom = union.bottom().max(rect.bottom());
        union = ScreenRect {
            x: left,
            y: top,
            width: (right - left) as u32,
            height: (bottom - top) as u32,
        };
    }
    Some(union)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResolvedGeometry {
    size: PhysicalSize<u32>,
    position: Option<PhysicalPosition<i32>>,
}

///   否则用户「拉伸过的尺寸」会被整体丢弃、每次启动回到 1280×840（issue #464）；
fn resolve_geometry_in(
    saved: &WindowState,
    screens: &[ScreenRect],
    primary: Option<ScreenRect>,
) -> Option<ResolvedGeometry> {
    if is_degenerate_size(saved) {
        return None;
    }

    let union = union_rect(screens.iter().copied());
    let mut width = saved.width.max(MIN_WINDOW_WIDTH as u32);
    let mut height = saved.height.max(MIN_WINDOW_HEIGHT as u32);
    if let Some(union) = union {
        width = width.min(union.width.max(MIN_WINDOW_WIDTH as u32));
        height = height.min(union.height.max(MIN_WINDOW_HEIGHT as u32));
    }
    let size = PhysicalSize::new(width, height);

    Some(ResolvedGeometry {
        size,
        position: resolve_position(saved, size, screens, union, primary),
    })
}

fn resolve_position(
    saved: &WindowState,
    size: PhysicalSize<u32>,
    screens: &[ScreenRect],
    union: Option<ScreenRect>,
    primary: Option<ScreenRect>,
) -> Option<PhysicalPosition<i32>> {
    let (Some(sx), Some(sy)) = (saved.x, saved.y) else {
        return None;
    };
    let union = union?;

    let window = ScreenRect {
        x: sx,
        y: sy,
        width: size.width,
        height: size.height,
    };
    if screens.iter().any(|screen| window.intersects(screen)) {
        let max_x = union.right().saturating_sub(size.width as i32);
        let max_y = union.bottom().saturating_sub(size.height as i32);
        let nx = (sx as i64).clamp(union.x as i64, (max_x as i64).max(union.x as i64));
        let ny = (sy as i64).clamp(union.y as i64, (max_y as i64).max(union.y as i64));
        return Some(PhysicalPosition::new(nx as i32, ny as i32));
    }

    let primary = primary?;
    Some(PhysicalPosition::new(
        primary.x + (primary.width as i32 - size.width as i32) / 2,
        primary.y + (primary.height as i32 - size.height as i32) / 2,
    ))
}

fn screens_of<R: Runtime>(app: &AppHandle<R>) -> Vec<ScreenRect> {
    app.available_monitors()
        .map(|monitors| {
            monitors
                .iter()
                .map(|monitor| ScreenRect {
                    x: monitor.position().x,
                    y: monitor.position().y,
                    width: monitor.size().width,
                    height: monitor.size().height,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn primary_screen<R: Runtime>(app: &AppHandle<R>) -> Option<ScreenRect> {
    let monitor = app.primary_monitor().ok().flatten()?;
    Some(ScreenRect {
        x: monitor.position().x,
        y: monitor.position().y,
        width: monitor.size().width,
        height: monitor.size().height,
    })
}

fn resolve_geometry<R: Runtime>(
    app: &AppHandle<R>,
    saved: &WindowState,
) -> Option<ResolvedGeometry> {
    resolve_geometry_in(saved, &screens_of(app), primary_screen(app))
}

/// （issue #464）。
pub fn restore_main_window<R: Runtime>(app: &AppHandle<R>, window: &WebviewWindow<R>) {
    let mut saved = get_window_state(app);
    if !saved.size_is_inner {
        if let (Ok(outer), Ok(inner)) = (window.outer_size(), window.inner_size()) {
            saved = migrate_legacy_outer_size(&saved, outer, inner);
        }
    }
    if let Some(geometry) = resolve_geometry(app, &saved) {
        let _ = window.set_size(Size::Physical(geometry.size));
        if let Some(position) = geometry.position {
            let _ = window.set_position(Position::Physical(position));
        }
    }
    if saved.maximized {
        let _ = window.maximize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_geometry_does_not_sample_extra_or_remote_windows_without_main() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        for label in ["window-geometry-test", "remote-geometry-test"] {
            tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::default())
                .build()
                .unwrap();
        }
        assert!(crate::utils::shell_window(app.handle()).is_some());
        save_main_window_geometry(app.handle());
    }

    #[test]
    fn window_state_default_matches_builder_default() {
        let state = WindowState::default();
        assert!(!state.maximized);
        assert!(state.x.is_none());
        assert!(state.y.is_none());
        assert_eq!(state.width, DEFAULT_WINDOW_WIDTH as u32);
        assert_eq!(state.height, DEFAULT_WINDOW_HEIGHT as u32);
        assert!(state.size_is_inner);
    }

    #[test]
    fn window_state_serde_roundtrip() {
        let state = WindowState {
            maximized: true,
            x: Some(100),
            y: Some(-200),
            width: 1920,
            height: 1080,
            size_is_inner: true,
        };
        let json = serde_json::to_string(&state).expect("serialize");
        let parsed: WindowState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.maximized, state.maximized);
        assert_eq!(parsed.x, state.x);
        assert_eq!(parsed.y, state.y);
        assert_eq!(parsed.width, state.width);
        assert_eq!(parsed.height, state.height);
    }

    #[test]
    fn window_state_ignores_missing_fields() {
        let parsed: WindowState = serde_json::from_str("{}").expect("deserialize");
        assert!(!parsed.maximized);
        assert!(parsed.x.is_none());
        assert!(parsed.y.is_none());
        assert_eq!(parsed.width, DEFAULT_WINDOW_WIDTH as u32);
        assert_eq!(parsed.height, DEFAULT_WINDOW_HEIGHT as u32);
        assert!(!parsed.size_is_inner);
    }

    #[test]
    fn legacy_outer_size_is_converted_for_set_size() {
        let saved = WindowState {
            width: 1137,
            height: 792,
            size_is_inner: false,
            ..WindowState::default()
        };

        let migrated = migrate_legacy_outer_size(
            &saved,
            PhysicalSize::new(1293, 848),
            PhysicalSize::new(1267, 833),
        );

        assert_eq!(migrated.width, 1111);
        assert_eq!(migrated.height, 777);
        assert!(migrated.size_is_inner);
    }

    #[test]
    fn inner_size_is_not_converted_twice() {
        let saved = WindowState {
            width: 1111,
            height: 777,
            size_is_inner: true,
            ..WindowState::default()
        };

        let migrated = migrate_legacy_outer_size(
            &saved,
            PhysicalSize::new(1293, 848),
            PhysicalSize::new(1267, 833),
        );

        assert_eq!(migrated, saved);
    }

    #[test]
    fn degenerate_size_is_detected() {
        let at_min = WindowState {
            width: MIN_WINDOW_WIDTH as u32,
            height: MIN_WINDOW_HEIGHT as u32,
            ..WindowState::default()
        };
        assert!(is_degenerate_size(&at_min));

        let below_min = WindowState {
            width: 200,
            height: 200,
            ..WindowState::default()
        };
        assert!(is_degenerate_size(&below_min));

        let normal = WindowState {
            width: DEFAULT_WINDOW_WIDTH as u32,
            height: DEFAULT_WINDOW_HEIGHT as u32,
            ..WindowState::default()
        };
        assert!(!is_degenerate_size(&normal));

        let half_normal = WindowState {
            width: DEFAULT_WINDOW_WIDTH as u32,
            height: MIN_WINDOW_HEIGHT as u32,
            ..WindowState::default()
        };
        assert!(!is_degenerate_size(&half_normal));
    }

    fn rect(x: i32, y: i32, width: u32, height: u32) -> ScreenRect {
        ScreenRect {
            x,
            y,
            width,
            height,
        }
    }

    fn saved(x: Option<i32>, y: Option<i32>, width: u32, height: u32) -> WindowState {
        WindowState {
            x,
            y,
            width,
            height,
            size_is_inner: true,
            ..WindowState::default()
        }
    }

    #[test]
    fn union_rect_spans_every_screen() {
        let union = union_rect([rect(0, 0, 1920, 1080), rect(1920, -200, 1280, 1024)])
            .expect("union of two screens");
        assert_eq!(union, rect(0, -200, 3200, 1280));

        assert_eq!(
            union_rect([rect(0, 0, 1920, 1080)]),
            Some(rect(0, 0, 1920, 1080))
        );

        assert_eq!(union_rect(Vec::<ScreenRect>::new()), None);
    }

    #[test]
    fn size_is_restored_even_without_saved_position() {
        // issue #464：没有位置记录（或位置不可用）时，用户拉伸过的尺寸必须照常恢复，
        let resolved = resolve_geometry_in(
            &saved(None, None, 1500, 1000),
            &[rect(0, 0, 1920, 1080)],
            Some(rect(0, 0, 1920, 1080)),
        )
        .expect("geometry resolved without position");

        assert_eq!(resolved.size, PhysicalSize::new(1500, 1000));
        assert_eq!(resolved.position, None);
    }

    #[test]
    fn size_is_restored_even_without_monitor_info() {
        let resolved = resolve_geometry_in(&saved(Some(100), Some(100), 1500, 1000), &[], None)
            .expect("geometry resolved without monitors");

        assert_eq!(resolved.size, PhysicalSize::new(1500, 1000));
        assert_eq!(resolved.position, None);
    }

    #[test]
    fn restored_size_never_goes_below_the_minimum() {
        let resolved = resolve_geometry_in(
            &saved(Some(0), Some(0), 700, 1000),
            &[rect(0, 0, 1920, 1080)],
            Some(rect(0, 0, 1920, 1080)),
        )
        .expect("geometry resolved");

        assert_eq!(
            resolved.size,
            PhysicalSize::new(MIN_WINDOW_WIDTH as u32, 1000)
        );
    }

    #[test]
    fn restored_size_is_clamped_into_the_visible_union() {
        let resolved = resolve_geometry_in(
            &saved(Some(0), Some(0), 3840, 2160),
            &[rect(0, 0, 1920, 1080)],
            Some(rect(0, 0, 1920, 1080)),
        )
        .expect("geometry resolved");

        assert_eq!(resolved.size, PhysicalSize::new(1920, 1080));
    }

    #[test]
    fn visible_position_is_clamped_into_the_visible_union() {
        let resolved = resolve_geometry_in(
            &saved(Some(1000), Some(20), 1500, 1000),
            &[rect(0, 0, 1920, 1080)],
            Some(rect(0, 0, 1920, 1080)),
        )
        .expect("geometry resolved");

        assert_eq!(resolved.size, PhysicalSize::new(1500, 1000));
        assert_eq!(resolved.position, Some(PhysicalPosition::new(420, 20)));
    }

    #[test]
    fn offscreen_position_falls_back_to_primary_center() {
        let resolved = resolve_geometry_in(
            &saved(Some(9000), Some(5000), 1500, 1000),
            &[rect(0, 0, 1920, 1080)],
            Some(rect(0, 0, 1920, 1080)),
        )
        .expect("geometry resolved");

        assert_eq!(resolved.size, PhysicalSize::new(1500, 1000));
        assert_eq!(resolved.position, Some(PhysicalPosition::new(210, 40)));
    }

    #[test]
    fn offscreen_position_without_primary_screen_keeps_the_size() {
        let resolved = resolve_geometry_in(
            &saved(Some(9000), Some(5000), 1500, 1000),
            &[rect(0, 0, 1920, 1080)],
            None,
        )
        .expect("geometry resolved");

        assert_eq!(resolved.size, PhysicalSize::new(1500, 1000));
        assert_eq!(resolved.position, None);
    }

    #[test]
    fn position_in_the_gap_between_offset_screens_is_recentered() {
        let screens = [rect(0, 0, 1920, 1080), rect(1920, 1080, 1280, 1024)];
        let resolved = resolve_geometry_in(
            &saved(Some(600), Some(1300), 1000, 700),
            &screens,
            Some(rect(0, 0, 1920, 1080)),
        )
        .expect("geometry resolved");

        assert_eq!(resolved.size, PhysicalSize::new(1000, 700));
        assert_eq!(resolved.position, Some(PhysicalPosition::new(460, 190)));
    }

    #[test]
    fn position_on_an_offset_secondary_screen_is_kept() {
        let screens = [rect(0, 0, 1920, 1080), rect(1920, 1080, 1280, 1024)];
        let resolved = resolve_geometry_in(
            &saved(Some(2000), Some(1200), 1000, 700),
            &screens,
            Some(rect(0, 0, 1920, 1080)),
        )
        .expect("geometry resolved");

        assert_eq!(resolved.size, PhysicalSize::new(1000, 700));
        assert_eq!(resolved.position, Some(PhysicalPosition::new(2000, 1200)));
    }

    #[test]
    fn degenerate_size_still_falls_back_to_builder_defaults() {
        let resolved = resolve_geometry_in(
            &saved(
                Some(10),
                Some(10),
                MIN_WINDOW_WIDTH as u32,
                MIN_WINDOW_HEIGHT as u32,
            ),
            &[rect(0, 0, 1920, 1080)],
            Some(rect(0, 0, 1920, 1080)),
        );

        assert!(resolved.is_none());
    }

    #[test]
    fn geometry_restore_guard_gates_sampling_until_dropped() {
        assert!(!is_restoring_geometry());
        {
            let _guard = GeometryRestoreGuard::begin();
            assert!(is_restoring_geometry());
        }
        assert!(!is_restoring_geometry());
    }
}
