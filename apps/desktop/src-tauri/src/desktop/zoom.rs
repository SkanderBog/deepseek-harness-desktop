use crate::WebviewWindow;
#[cfg(target_os = "macos")]
use objc2_foundation::{NSOperatingSystemVersion, NSProcessInfo};
use tauri::Runtime;

#[cfg(any(target_os = "macos", test))]
const MINIMUM_MACOS_ZOOM_MAJOR: isize = 11;

#[cfg(any(target_os = "macos", test))]
fn macos_zoom_support_error(
    is_at_least_macos_11: bool,
    major: isize,
    minor: isize,
) -> Option<String> {
    if is_at_least_macos_11 {
        return None;
    }
    Some(format!(
        "ZOOM_UNSUPPORTED_OS: macOS {major}.{minor} does not support native WebView zoom; macOS 11 or newer is required"
    ))
}

#[cfg(target_os = "macos")]
fn current_platform_zoom_support_error() -> Option<String> {
    let process_info = NSProcessInfo::processInfo();
    let minimum = NSOperatingSystemVersion {
        majorVersion: MINIMUM_MACOS_ZOOM_MAJOR,
        minorVersion: 0,
        patchVersion: 0,
    };
    let is_supported = process_info.isOperatingSystemAtLeastVersion(minimum);
    let version = process_info.operatingSystemVersion();
    macos_zoom_support_error(is_supported, version.majorVersion, version.minorVersion)
}

#[cfg(not(target_os = "macos"))]
fn current_platform_zoom_support_error() -> Option<String> {
    None
}

pub fn apply_native_zoom<R: Runtime>(
    window: &WebviewWindow<R>,
    zoom_factor: f64,
) -> Result<(), String> {
    if let Some(error) = current_platform_zoom_support_error() {
        return Err(error);
    }
    window
        .set_zoom(zoom_factor)
        .map_err(|error| format!("ZOOM_APPLY_FAILED: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{MINIMUM_MACOS_ZOOM_MAJOR, macos_zoom_support_error};

    #[test]
    fn native_zoom_requires_macos_11_or_newer() {
        assert_eq!(MINIMUM_MACOS_ZOOM_MAJOR, 11);
        let error = macos_zoom_support_error(false, 10, 15).expect("Catalina must be rejected");
        assert!(error.starts_with("ZOOM_UNSUPPORTED_OS:"));
        assert!(error.contains("macOS 10.15"));
        assert!(macos_zoom_support_error(true, 11, 0).is_none());
        assert!(macos_zoom_support_error(true, 15, 4).is_none());
        assert!(macos_zoom_support_error(true, 10, 16).is_none());
    }
}
