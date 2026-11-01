use crate::WebviewWindow;
use tauri::Runtime;

pub fn apply_native_zoom<R: Runtime>(
    window: &WebviewWindow<R>,
    zoom_factor: f64,
) -> Result<(), String> {
    window
        .set_zoom(zoom_factor)
        .map_err(|error| format!("ZOOM_APPLY_FAILED: {error}"))
}
