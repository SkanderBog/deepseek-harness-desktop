use std::path::{Path, PathBuf};

use tauri::{Manager, Runtime};

use crate::service::core::active_core_dir;
use crate::{AppHandle, WebviewWindow};

#[derive(Debug, PartialEq, Eq)]
pub enum PatchOutcome {
    AlreadyPatched,
    AnchorMissing,
    Patched(String),
}

pub fn patch_dsh(
    app_handle: &AppHandle,
    rel_path: &str,
    patch: impl FnOnce(&str) -> PatchOutcome,
) -> Result<(), String> {
    patch_core_file(&active_core_dir(app_handle), rel_path, patch)
}

pub fn patch_core_file(
    core_dir: &Path,
    rel_path: &str,
    patch: impl FnOnce(&str) -> PatchOutcome,
) -> Result<(), String> {
    let target = core_dir.join(rel_path);
    if !target.exists() {
        log::info!("dsh patch target not found, skip: {}", target.display());
        return Ok(());
    }
    let source = std::fs::read_to_string(&target)
        .map_err(|e| format!("DSH_PATCH_READ: {} failed: {e}", target.display()))?;
    match patch(&source) {
        PatchOutcome::AlreadyPatched => {
            log::info!("dsh patch already applied: {}", target.display());
        }
        PatchOutcome::AnchorMissing => {
            log::warn!("dsh patch anchor missing, skip: {}", target.display());
        }
        PatchOutcome::Patched(patched) => {
            std::fs::write(&target, patched)
                .map_err(|e| format!("DSH_PATCH_WRITE: {} failed: {e}", target.display()))?;
            log::info!("dsh patch applied: {}", target.display());
        }
    }
    Ok(())
}

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

pub fn operation_window<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    use crate::service::workflow::instances;
    let profile = instances::current_profile()?;
    app.get_webview_window("main")
        .filter(|window| instances::window_uses_profile(window.label(), &profile))
        .or_else(|| {
            app.webview_windows().into_values().find(|window| {
                (window.label().starts_with("window-") || window.label().starts_with("remote-"))
                    && instances::window_uses_profile(window.label(), &profile)
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
        log::warn!("[window] main window not found, skip show");
    }
}

pub fn decode_process_line(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }
    #[cfg(windows)]
    if let Some(text) = decode_multibyte(bytes, windows_sys::Win32::Globalization::CP_ACP) {
        return text;
    }
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(windows)]
pub(crate) fn decode_multibyte(bytes: &[u8], codepage: u32) -> Option<String> {
    use windows_sys::Win32::Globalization::MultiByteToWideChar;

    if bytes.is_empty() {
        return Some(String::new());
    }
    if bytes.len() > i32::MAX as usize {
        return None;
    }
    let len = bytes.len() as i32;
    let needed =
        unsafe { MultiByteToWideChar(codepage, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0) };
    if needed <= 0 {
        return None;
    }
    let mut wide = vec![0u16; needed as usize];
    let written =
        unsafe { MultiByteToWideChar(codepage, 0, bytes.as_ptr(), len, wide.as_mut_ptr(), needed) };
    if written <= 0 {
        return None;
    }
    wide.truncate(written as usize);
    Some(String::from_utf16_lossy(&wide))
}

#[cfg(windows)]
pub(crate) fn encode_multibyte(text: &str, codepage: u32) -> Option<Vec<u8>> {
    use windows_sys::Win32::Globalization::{WideCharToMultiByte, CP_UTF8, WC_NO_BEST_FIT_CHARS};

    if text.is_empty() {
        return Some(Vec::new());
    }
    if codepage == CP_UTF8 {
        return Some(text.as_bytes().to_vec());
    }
    let wide: Vec<u16> = text.encode_utf16().collect();
    if wide.len() > i32::MAX as usize {
        return None;
    }
    let len = wide.len() as i32;
    let mut used_default = 0i32;
    let needed = unsafe {
        WideCharToMultiByte(
            codepage,
            WC_NO_BEST_FIT_CHARS,
            wide.as_ptr(),
            len,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            &mut used_default,
        )
    };
    if needed <= 0 {
        return None;
    }
    let mut bytes = vec![0u8; needed as usize];
    used_default = 0;
    let written = unsafe {
        WideCharToMultiByte(
            codepage,
            WC_NO_BEST_FIT_CHARS,
            wide.as_ptr(),
            len,
            bytes.as_mut_ptr(),
            needed,
            std::ptr::null(),
            &mut used_default,
        )
    };
    if written <= 0 || used_default != 0 {
        return None;
    }
    bytes.truncate(written as usize);
    Some(bytes)
}

#[cfg(windows)]
pub(crate) fn short_path(path: &Path) -> Option<PathBuf> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let needed = unsafe { GetShortPathNameW(wide.as_ptr(), std::ptr::null_mut(), 0) };
    if needed == 0 {
        return None;
    }
    let mut buf = vec![0u16; needed as usize];
    let written = unsafe { GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), needed) };
    if written == 0 || written >= needed {
        return None;
    }
    buf.truncate(written as usize);
    Some(PathBuf::from(std::ffi::OsString::from_wide(&buf)))
}

#[cfg(windows)]
pub(crate) fn console_code_page() -> u32 {
    use windows_sys::Win32::Globalization::{GetACP, GetOEMCP};

    let oem = unsafe { GetOEMCP() };
    if oem != 0 {
        return oem;
    }
    let ansi = unsafe { GetACP() };
    if ansi != 0 {
        return ansi;
    }
    65001
}

#[cfg(test)]
mod tests {
    use super::decode_process_line;
    #[cfg(windows)]
    use super::{decode_multibyte, encode_multibyte, short_path};

    #[tokio::test]
    async fn operation_window_never_falls_back_to_another_profile() {
        use crate::service::workflow::instances;
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        for label in ["main", "window-operation-test", "remote-operation-test"] {
            tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::default())
                .build()
                .unwrap();
        }
        instances::bind_window("main", "operation-main-profile");
        instances::bind_window("window-operation-test", "operation-extra-profile");
        instances::bind_window("remote-operation-test", "operation-remote-profile");
        assert!(super::operation_window(app.handle()).is_none());
        for (profile, expected) in [
            ("operation-main-profile", Some("main")),
            ("operation-extra-profile", Some("window-operation-test")),
            ("operation-remote-profile", Some("remote-operation-test")),
            ("operation-missing-profile", None),
        ] {
            instances::scope(profile.into(), async {
                let window = super::operation_window(app.handle());
                assert_eq!(window.as_ref().map(|window| window.label()), expected);
            })
            .await;
        }
        for label in ["main", "window-operation-test", "remote-operation-test"] {
            instances::release_window(label);
        }
    }

    #[test]
    fn keeps_valid_utf8_untouched() {
        let text = "系统找不到指定的路径。";
        assert_eq!(decode_process_line(text.as_bytes()), text);
    }

    #[test]
    fn invalid_bytes_still_produce_a_line() {
        assert!(!decode_process_line(&[0xFF, 0xFE, 0x80]).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn decodes_gbk_with_an_explicit_codepage() {
        let gbk = [
            0xCF, 0xB5, 0xCD, 0xB3, 0xD5, 0xD2, 0xB2, 0xBB, 0xB5, 0xBD, 0xD6, 0xB8, 0xB6, 0xA8,
            0xB5, 0xC4, 0xC2, 0xB7, 0xBE, 0xB6, 0xA1, 0xA3,
        ];
        assert_eq!(
            decode_multibyte(&gbk, 936).as_deref(),
            Some("系统找不到指定的路径。")
        );
    }

    #[cfg(windows)]
    #[test]
    fn encodes_back_to_the_same_codepage_bytes() {
        let text = "系统找不到指定的路径。";
        let gbk = encode_multibyte(text, 936).expect("gbk");
        assert!(!gbk.is_ascii());
        assert_eq!(decode_multibyte(&gbk, 936).as_deref(), Some(text));
        assert_eq!(
            encode_multibyte(text, 65001).as_deref(),
            Some(text.as_bytes())
        );
    }

    #[cfg(windows)]
    #[test]
    fn rejects_characters_outside_the_codepage() {
        assert_eq!(encode_multibyte("小蔡", 437), None);
    }

    #[cfg(windows)]
    #[test]
    fn short_path_resolves_existing_paths() {
        let temp = std::env::temp_dir();
        let short = short_path(&temp).expect("temp dir always resolves");
        assert!(short.exists());
    }
}
