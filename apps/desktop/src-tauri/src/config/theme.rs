use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use tauri::Emitter;
#[cfg(target_os = "macos")]
use tauri::Manager;

use super::runtime::get_dsh_data_path;
use crate::AppHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DshTheme {
    Dark,
    Light,
    System,
}

const DEFAULT_THEME: DshTheme = DshTheme::System;

static LAST_EMITTED: OnceLock<Mutex<Option<DshTheme>>> = OnceLock::new();

pub fn get_dsh_theme(app_handle: &AppHandle) -> DshTheme {
    let home = get_dsh_data_path(app_handle);
    let profile = crate::service::profile::profile_dir_of(
        app_handle,
        &crate::service::profile::active_profile(app_handle),
    );
    read_dsh_theme(&home, &profile)
}

fn read_dsh_theme(home: &Path, profile_dir: &Path) -> DshTheme {
    if let Ok(content) = fs::read_to_string(profile_dir.join("cordis.patch.yml")) {
        if let Some(theme) = parse_profile_theme(&content) {
            return theme;
        }
    }
    for name in [LEGACY_SETTINGS_FILE, IMPORTED_SETTINGS_FILE] {
        if let Ok(content) = fs::read_to_string(home.join(name)) {
            if let Some(theme) = parse_theme_preference(&content) {
                return theme;
            }
        }
    }
    log::debug!("no dsh theme preference found; following the system appearance");
    DEFAULT_THEME
}

const LEGACY_SETTINGS_FILE: &str = "settings.yaml";

const IMPORTED_SETTINGS_FILE: &str = "settings.yaml.imported";

const UI_THEME_ENTRY_ID: &str = "ui-theme";

fn parse_profile_theme(content: &str) -> Option<DshTheme> {
    let mut in_theme_entry = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if line.starts_with('-') {
            in_theme_entry = false;
            if let Some(value) = trimmed.strip_prefix("- id:") {
                in_theme_entry = value.trim().trim_matches(['"', '\'']) == UI_THEME_ENTRY_ID;
            }
            continue;
        }
        if !in_theme_entry {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("preference:") {
            return parse_theme_name(value.trim().trim_matches(['"', '\'']));
        }
    }
    None
}

fn parse_theme_name(value: &str) -> Option<DshTheme> {
    match value {
        "light" => Some(DshTheme::Light),
        "dark" => Some(DshTheme::Dark),
        "system" => Some(DshTheme::System),
        _ => None,
    }
}

fn parse_theme_preference(content: &str) -> Option<DshTheme> {
    let mut in_ui_theme = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("ui-theme:") {
            in_ui_theme = true;
            continue;
        }
        if !in_ui_theme {
            continue;
        }
        if !line.starts_with(' ') && !line.starts_with('\t') {
            return None;
        }
        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            if let Some(value) = trimmed.strip_prefix("preference:") {
                return parse_theme_name(value.trim());
            }
        }
    }
    None
}

/// 是暗色」的割裂（issue #93）。这里把偏好直接落到原生外观：`system` → 跟随系统，
#[cfg(target_os = "macos")]
pub fn apply_window_theme(app_handle: &AppHandle, theme: DshTheme) {
    let appearance = match theme {
        DshTheme::System => None,
        DshTheme::Light => Some(tauri::Theme::Light),
        DshTheme::Dark => Some(tauri::Theme::Dark),
    };
    for window in app_handle.webview_windows().into_values() {
        if let Err(err) = window.set_theme(appearance) {
            log::warn!("[theme] failed to sync native window appearance: {err}");
        }
    }
}

pub fn check_and_emit_theme(app_handle: &AppHandle) {
    let theme = get_dsh_theme(app_handle);
    let mut last = LAST_EMITTED
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap();
    if *last == Some(theme) {
        return;
    }
    *last = Some(theme);
    log::debug!("dsh theme preference changed: {:?}", theme);
    let _ = app_handle.emit("dsh-theme-updated", &theme);
    #[cfg(target_os = "macos")]
    apply_window_theme(app_handle, theme);
}

const CANVAS_DARK: tauri::webview::Color = tauri::webview::Color(0x15, 0x15, 0x17, 0xFF);
const CANVAS_LIGHT: tauri::webview::Color = tauri::webview::Color(0xFF, 0xFF, 0xFF, 0xFF);

pub fn window_background(
    theme: DshTheme,
    system: Option<tauri::Theme>,
) -> Option<tauri::webview::Color> {
    match theme {
        DshTheme::Light => Some(CANVAS_LIGHT),
        DshTheme::Dark => Some(CANVAS_DARK),
        DshTheme::System => match system {
            Some(tauri::Theme::Light) => Some(CANVAS_LIGHT),
            Some(tauri::Theme::Dark) => Some(CANVAS_DARK),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_follows_system() {
        assert_eq!(DEFAULT_THEME, DshTheme::System);
    }

    #[test]
    fn parses_profile_patch_theme_entry() {
        let content = "- id: ui-theme\n  name: \"@deepseek-ai/dsh-client-ui-theme\"\n  config:\n    preference: light\n- id: locale\n  name: \"@deepseek-ai/dsh-client-locale\"\n  config:\n    preference: zh\n";
        assert_eq!(parse_profile_theme(content), Some(DshTheme::Light));
    }

    #[test]
    fn profile_patch_ignores_other_entries() {
        assert_eq!(
            parse_profile_theme("- id: locale\n  config:\n    preference: dark\n"),
            None
        );
    }

    #[test]
    fn profile_patch_rejects_missing_and_unknown() {
        assert_eq!(parse_profile_theme("[]\n"), None);
        assert_eq!(
            parse_profile_theme("- id: ui-theme\n  config:\n    preference: blue\n"),
            None
        );
        assert_eq!(
            parse_profile_theme("- id: ui-theme\n  config:\n    density: compact\n"),
            None
        );
    }

    #[test]
    fn profile_patch_wins_over_legacy_settings() {
        let home = std::env::temp_dir().join("dsh-theme-test-home");
        let profile = std::env::temp_dir().join("dsh-theme-test-profile");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&profile).unwrap();
        fs::write(
            home.join("settings.yaml"),
            "ui-theme:\n  preference: light\n",
        )
        .unwrap();
        fs::write(
            profile.join("cordis.patch.yml"),
            "- id: ui-theme\n  config:\n    preference: dark\n",
        )
        .unwrap();
        assert_eq!(read_dsh_theme(&home, &profile), DshTheme::Dark);
        let _ = fs::remove_file(profile.join("cordis.patch.yml"));
        assert_eq!(read_dsh_theme(&home, &profile), DshTheme::Light);
        let _ = fs::remove_dir_all(&home);
        let _ = fs::remove_dir_all(&profile);
    }

    #[test]
    fn window_background_follows_preference() {
        let rgba = |c: Option<tauri::webview::Color>| c.map(|c| (c.0, c.1, c.2, c.3));
        assert_eq!(
            rgba(window_background(DshTheme::Light, None)),
            Some((0xFF, 0xFF, 0xFF, 0xFF))
        );
        assert_eq!(
            rgba(window_background(DshTheme::Dark, None)),
            Some((0x15, 0x15, 0x17, 0xFF))
        );
        assert_eq!(
            rgba(window_background(
                DshTheme::System,
                Some(tauri::Theme::Dark)
            )),
            Some((0x15, 0x15, 0x17, 0xFF))
        );
        assert_eq!(
            rgba(window_background(
                DshTheme::System,
                Some(tauri::Theme::Light)
            )),
            Some((0xFF, 0xFF, 0xFF, 0xFF))
        );
        assert_eq!(window_background(DshTheme::System, None), None);
    }

    #[test]
    fn parses_known_preference_values() {
        let cases = [
            ("ui-theme:\n  preference: dark\n", DshTheme::Dark),
            ("ui-theme:\n  preference: light\n", DshTheme::Light),
            ("ui-theme:\n  preference: system\n", DshTheme::System),
        ];
        for (content, expected) in cases {
            assert_eq!(
                parse_theme_preference(content),
                Some(expected),
                "content={content:?}"
            );
        }
    }

    #[test]
    fn rejects_unknown_and_missing_preference() {
        assert_eq!(
            parse_theme_preference("ui-theme:\n  preference: blue\n"),
            None
        );
        assert_eq!(
            parse_theme_preference("ui-theme:\n  density: compact\n"),
            None
        );
        assert_eq!(parse_theme_preference("editor:\n  fontSize: 12\n"), None);
    }
}
