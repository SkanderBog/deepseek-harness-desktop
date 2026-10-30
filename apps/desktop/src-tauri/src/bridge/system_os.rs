use crate::config;
use crate::logger;
use crate::service::{core, plugin, profile};
use crate::{AppHandle, WebviewWindow};
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub async fn proxy_health_check(
    window: WebviewWindow,
    app_handle: AppHandle,
) -> Result<crate::service::workflow::HealthCheck, String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {
        let port = crate::service::workflow::instances::port(&app_handle);
        crate::service::workflow::proxy_health_check(port).await
    })
    .await
}

#[tauri::command]
pub fn harness_ownership(app_handle: AppHandle, window: WebviewWindow) -> Result<(), String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope_sync(
        profile,
        crate::service::workflow::recheck_ownership,
    )
}

#[tauri::command]
pub async fn get_runtime_info(
    window: WebviewWindow,
    app_handle: AppHandle,
) -> Result<config::RuntimeInfo, String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {
        let port = crate::service::workflow::instances::port(&app_handle);
        let mut info = config::runtime_info(&app_handle, port);
        info.dsh_version = core::active_version(&app_handle).or(info.dsh_version);
        Ok(info)
    })
    .await
}

#[tauri::command]
pub async fn open_in_browser(window: WebviewWindow, app_handle: AppHandle) -> Result<(), String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {
        let url =
            config::get_dsh_service_url(crate::service::workflow::instances::port(&app_handle));
        app_handle
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn copy_service_url(window: WebviewWindow, app_handle: AppHandle) -> Result<(), String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {
        let url =
            config::get_dsh_service_url(crate::service::workflow::instances::port(&app_handle));
        crate::bridge::write_clipboard_text(url).await
    })
    .await
}

#[tauri::command]
pub fn reveal_in_folder(app_handle: AppHandle, path: String) -> Result<(), String> {
    if !crate::bridge::guard::is_allowed_path(&app_handle, std::path::Path::new(&path)) {
        return Err(format!("REVEAL_PATH_REJECTED: {path}"));
    }
    tauri_plugin_opener::reveal_item_in_dir(&path).map_err(|e| format!("REVEAL_FAILED: {e}"))
}

#[tauri::command]
pub fn open_dir(app_handle: AppHandle, path: String) -> Result<(), String> {
    if !crate::bridge::guard::is_allowed_path(&app_handle, std::path::Path::new(&path)) {
        return Err(format!("OPEN_DIR_REJECTED: {path}"));
    }
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(|e| format!("OPEN_DIR_FAILED: {e}"))
}

#[tauri::command]
pub async fn reveal_data_dir(app_handle: AppHandle) -> Result<(), String> {
    let dsh_home = config::get_dsh_data_path(&app_handle);
    std::fs::create_dir_all(&dsh_home).map_err(|e| e.to_string())?;

    tauri_plugin_opener::open_path(&dsh_home, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn log_frontend(level: String, target: String, message: String) {
    let lvl = logger::FrontendLevel::from_str(&level);
    logger::log_frontend(lvl, &target, &message);
}

fn tail_bytes(s: &str, max_bytes: usize) -> &str {
    let start = s.len().saturating_sub(max_bytes);
    let mut i = start;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    &s[i..]
}

#[tauri::command]
pub async fn read_service_logs(
    window: WebviewWindow,
    app_handle: AppHandle,
    max_bytes: Option<usize>,
) -> Result<String, String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {
        let log_path = config::get_service_log_path(&app_handle);
        if !log_path.exists() {
            return Ok(String::new());
        }

        let content = std::fs::read_to_string(&log_path).map_err(|e| e.to_string())?;
        let max_bytes = max_bytes.unwrap_or(64 * 1024);
        if content.len() <= max_bytes {
            Ok(content)
        } else {
            Ok(tail_bytes(&content, max_bytes).to_string())
        }
    })
    .await
}

#[tauri::command]
pub async fn clear_service_logs(
    window: WebviewWindow,
    app_handle: AppHandle,
) -> Result<(), String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {
        let log_path = config::get_service_log_path(&app_handle);
        std::fs::write(&log_path, "").map_err(|e| e.to_string())
    })
    .await
}

fn format_env_info(
    app_version: &str,
    dsh_version: Option<&str>,
    node_version: &str,
    os: &str,
    arch: &str,
    profile: &str,
    plugins: &[String],
) -> String {
    let mut lines = vec![format!("app: {app_version}")];
    if let Some(version) = dsh_version {
        lines.push(format!("dsh: {version}"));
    }
    lines.push(format!("node: {node_version}"));
    lines.push(format!("os: {os} ({arch})"));
    lines.push(format!("profile: {profile}"));
    lines.push(format!("plugins: {} installed", plugins.len()));
    lines.extend(plugins.iter().map(|p| format!("  - {p}")));
    lines.join("\n")
}

#[tauri::command]
pub async fn read_run_logs(window: WebviewWindow, app_handle: AppHandle) -> Result<String, String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {
    const MAX_LINES: usize = 100;
    const FRONTEND_MAX_LINES: usize = MAX_LINES / 2;

    let base = config::get_base_dir(&app_handle);
    let service = config::get_service_log_path(&app_handle);
    let desktop = base.join("logs").join("desktop.log");
    let frontend = base.join("logs").join("desktop.frontdesk.log");

    let dsh_version =
        core::active_version(&app_handle).or_else(|| config::get_dsh_version(&app_handle));
    let mut plugins = plugin::watch::list(&app_handle);
    plugins.sort_by(|a, b| a.id.cmp(&b.id));
    let plugin_lines: Vec<String> = plugins
        .iter()
        .map(|p| {
            if p.version.is_empty() {
                p.id.clone()
            } else {
                format!("{}@{}", p.id, p.version)
            }
        })
        .collect();
    let app_version = app_handle.package_info().version.to_string();
    let env_text = format_env_info(
        &app_version,
        dsh_version.as_deref(),
        &config::get_active_node_version(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        &profile::active_profile(&app_handle),
        &plugin_lines,
    );

    let service_text = read_tail(&service, MAX_LINES, false);
    let frontend_text = read_tail(&frontend, FRONTEND_MAX_LINES, false);
    let backend_text = read_tail(&desktop, MAX_LINES, true);

    Ok(format!(
        "### 环境信息\n\n{}\n\n### 服务日志\n\n```\n{}\n```\n\n### 前台日志\n\n```\n{}\n```\n\n### 后台日志\n\n```\n{}\n```",
        env_text,
        service_text.trim_end(),
        frontend_text.trim_end(),
        backend_text.trim_end()
    ))
 }).await
}

fn read_tail(path: &std::path::Path, max_lines: usize, filter_frontend: bool) -> String {
    if !path.exists() {
        return String::new();
    }
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = content
        .lines()
        .filter(|line| !filter_frontend || !is_frontend_log_line(line))
        .collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

fn is_frontend_log_line(line: &str) -> bool {
    const LEVELS: [&str; 5] = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];
    let trimmed = line.trim_start();
    LEVELS
        .iter()
        .any(|lvl| trimmed.contains(&format!("{lvl} frontend:")))
}

#[tauri::command]
pub async fn open_external_url(app_handle: AppHandle, url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(format!("EXTERNAL_URL_INVALID: {url}"));
    }
    app_handle
        .opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::format_env_info;
    use super::is_frontend_log_line;
    use super::tail_bytes;

    #[test]
    fn log_tail_filters_frontend_before_selecting_last_lines() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("dsh-shell-log-tail-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("desktop.log");
        std::fs::write(
            &path,
            "INFO dsh: first\r\nINFO frontend: noisy\r\nWARN dsh: 中文\nINFO frontend: last\n",
        )
        .unwrap();
        assert_eq!(
            super::read_tail(&path, 2, true),
            "INFO dsh: first\nWARN dsh: 中文"
        );
        assert_eq!(
            super::read_tail(&path, 2, false),
            "WARN dsh: 中文\nINFO frontend: last"
        );
        assert_eq!(super::read_tail(&path, 0, true), "");
        assert_eq!(super::read_tail(&root.join("missing"), 100, false), "");
        assert_eq!(super::read_tail(&root, 100, true), "");
        std::fs::write(&path, [0xff]).unwrap();
        assert_eq!(super::read_tail(&path, 100, false), "");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn env_info_carries_profile_and_plugins() {
        let text = format_env_info(
            "0.19.0",
            Some("0.1.7-rc.2"),
            "24.19.0",
            "windows",
            "x86_64",
            "web",
            &[
                "dshmarket@0.22.1".to_string(),
                "@scope/tool@1.2.3".to_string(),
            ],
        );
        assert_eq!(
            text,
            "app: 0.19.0\ndsh: 0.1.7-rc.2\nnode: 24.19.0\nos: windows (x86_64)\nprofile: web\nplugins: 2 installed\n  - dshmarket@0.22.1\n  - @scope/tool@1.2.3"
        );
    }

    #[test]
    fn env_info_omits_dsh_line_when_version_unknown() {
        let text = format_env_info("0.19.0", None, "24.19.0", "linux", "aarch64", "tauri", &[]);
        assert_eq!(
            text,
            "app: 0.19.0\nnode: 24.19.0\nos: linux (aarch64)\nprofile: tauri\nplugins: 0 installed"
        );
        assert!(!text.contains("dsh:"));
    }

    #[test]
    fn env_info_empty_plugin_list_is_just_a_count() {
        let text = format_env_info(
            "1.0.0",
            Some("0.1.7"),
            "22.19.0",
            "macos",
            "aarch64",
            "web",
            &[],
        );
        assert!(text.ends_with("plugins: 0 installed"));
        assert!(!text.contains("(none)"));
    }

    #[test]
    fn env_info_keeps_plugin_without_version_readable() {
        let text = format_env_info(
            "1.0.0",
            Some("0.1.7"),
            "22.19.0",
            "windows",
            "x86_64",
            "web",
            &["dsh-broken".to_string()],
        );
        assert!(text.contains("plugins: 1"));
        assert!(text.contains("dsh-broken"));
        assert!(!text.contains("dsh-broken@"));
    }

    #[test]
    fn frontend_line_detected() {
        assert!(is_frontend_log_line(
            "2024-06-01 12:00:00.123Z INFO frontend: [tag] message"
        ));
        assert!(is_frontend_log_line(
            "[2024-06-01 12:00:00.123Z] INFO frontend: message"
        ));
        assert!(is_frontend_log_line(
            "2024-06-01 12:00:00.123Z WARN frontend: something"
        ));
        assert!(is_frontend_log_line(
            "2024-06-01 12:00:00.123Z ERROR frontend: boom"
        ));
    }

    #[test]
    fn backend_line_not_detected() {
        assert!(!is_frontend_log_line(
            "2024-06-01 12:00:00.123Z INFO dsh: starting server"
        ));
        assert!(!is_frontend_log_line(
            "[2024-06-01 12:00:00.123Z] INFO dsh: emit to frontend: 3"
        ));
        assert!(!is_frontend_log_line(
            "2024-06-01 12:00:00.123Z DEBUG reqwest: GET /ping"
        ));
    }

    #[test]
    fn frontend_level_padding_and_extra_spaces() {
        assert!(is_frontend_log_line(
            "2024-06-01 12:00:00.123Z  INFO frontend: padded"
        ));
    }

    #[test]
    fn tail_bytes_keeps_ascii_within_limit() {
        assert_eq!(tail_bytes("hello world", 5), "world");
        assert_eq!(tail_bytes("abc", 2), "bc");
    }

    #[test]
    fn tail_bytes_advances_to_char_boundary() {
        assert_eq!(tail_bytes("中a", 2), "a");
        assert_eq!(tail_bytes("😀x", 3), "x");
        assert_eq!(tail_bytes("中文abc", 3), "abc");
    }

    #[test]
    fn tail_bytes_shorter_than_limit_returns_whole() {
        assert_eq!(tail_bytes("中文", 10), "中文");
        assert_eq!(tail_bytes("", 10), "");
    }
}
