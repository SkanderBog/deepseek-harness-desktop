use std::path::PathBuf;
use std::time::Duration;

use futures_util::StreamExt;
use tauri::{Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::config;
use crate::service::workflow;

use super::meta::{LatestRelease, fetch_latest_release};
use super::version::current_version;
use super::{DOWNLOAD_TIMEOUT_SECS, UPDATES_DIR};
use crate::AppHandle;

fn updates_dir(app_handle: &AppHandle) -> Result<PathBuf, String> {
    let dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("UPDATE_DIR: {e}"))?
        .join(UPDATES_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("UPDATE_DIR: {e}"))?;
    Ok(dir)
}

fn prune_installers_in_dir(dir: &std::path::Path, protected: &[PathBuf]) -> std::io::Result<()> {
    let root = dunce::canonicalize(dir)?;
    let protected: Vec<_> = protected
        .iter()
        .filter_map(|path| dunce::canonicalize(path).ok())
        .collect();
    let mut installers = Vec::new();
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(
            extension.as_str(),
            "exe" | "msi" | "dmg" | "appimage" | "deb" | "rpm"
        ) {
            continue;
        }
        let canonical = dunce::canonicalize(&path)?;
        if canonical.parent() != Some(root.as_path()) {
            continue;
        }
        installers.push((
            protected.contains(&canonical),
            entry.metadata()?.modified()?,
            path,
        ));
    }
    installers.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    for (protected, _, path) in installers.into_iter().skip(5) {
        if protected || dunce::canonicalize(&path)?.parent() != Some(root.as_path()) {
            continue;
        }
        if let Err(error) = std::fs::remove_file(&path) {
            log::warn!("UPDATE_PRUNE: failed to remove {}: {error}", path.display());
        }
    }
    Ok(())
}

fn prune_old_installers(app_handle: &AppHandle) {
    let mut protected: Vec<_> = super::pending::installer_path(app_handle)
        .into_iter()
        .collect();
    if let Ok(executable) = std::env::current_exe() {
        protected.push(executable);
    }
    #[cfg(target_os = "linux")]
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        protected.push(PathBuf::from(appimage));
    }
    let result = updates_dir(app_handle).and_then(|dir| {
        prune_installers_in_dir(&dir, &protected).map_err(|error| error.to_string())
    });
    if let Err(error) = result {
        log::warn!("UPDATE_PRUNE: failed to clean old installers: {error}");
    }
}

fn installer_path(app_handle: &AppHandle, asset_name: &str) -> Result<PathBuf, String> {
    Ok(updates_dir(app_handle)?.join(asset_name))
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopUpdateInfo {
    pub version: String,
    pub current_version: String,
    pub tag: String,
    pub published_at: String,
    pub url: String,
    pub asset_name: String,
    pub path: String,
    pub downloaded: bool,
}

pub async fn check(app_handle: &AppHandle) -> Result<Option<DesktopUpdateInfo>, String> {
    prune_old_installers(app_handle);
    match fetch_latest_release(app_handle).await? {
        None => Ok(None),
        Some(r) => {
            let path = installer_path(app_handle, &r.asset_name)?;
            let downloaded = path.exists();
            Ok(Some(DesktopUpdateInfo {
                version: r.version,
                current_version: current_version(),
                tag: r.tag,
                published_at: r.published_at,
                url: r.url,
                asset_name: r.asset_name,
                path: path.to_string_lossy().into_owned(),
                downloaded,
            }))
        }
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopDownloadProgress {
    pub percentage: f64,
    pub downloaded: u64,
    pub total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

async fn download_from_source(
    client: &reqwest::Client,
    url: &str,
    tmp: &std::path::Path,
    app_handle: &AppHandle,
) -> Result<(), String> {
    log::info!("Downloading desktop installer from {}", url);
    let res = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("UPDATE_DOWNLOAD: {e}"))?
        .error_for_status()
        .map_err(|e| format!("UPDATE_DOWNLOAD: {e}"))?;

    let total = res.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(tmp).map_err(|e| format!("UPDATE_FILE: {e}"))?;
    use std::io::Write;
    let mut downloaded: u64 = 0;
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("UPDATE_DOWNLOAD: {e}"))?;
        file.write_all(&chunk)
            .map_err(|e| format!("UPDATE_FILE: {e}"))?;
        downloaded += chunk.len() as u64;
        let pct = if total > 0 {
            (downloaded as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        let _ = app_handle.emit(
            "desktop-update-progress",
            DesktopDownloadProgress {
                percentage: pct,
                downloaded,
                total,
                message: None,
            },
        );
    }
    drop(file);
    Ok(())
}

fn download_client(app_handle: &AppHandle) -> Result<reqwest::Client, String> {
    config::proxy::http_client_builder(app_handle)?
        .user_agent("deepseek-harness-desktop")
        .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("UPDATE_CLIENT: {e}"))
}

fn download_sources(release: &LatestRelease) -> Vec<String> {
    let mut urls = vec![release.url.clone()];
    if release.digest.is_some() {
        urls.extend(config::mirror_download_urls(&release.url));
    }
    urls
}

/// 需要可执行位，否则表现为「下载成功但无法打开安装包」（issue #79）。这里
#[cfg(unix)]
fn ensure_installer_executable(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)
        .map_err(|e| format!("UPDATE_FILE: {e}"))?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).map_err(|e| format!("UPDATE_FILE: {e}"))?;
    Ok(())
}

fn verify_installer_sha256(path: &std::path::Path, expected: &str) -> Result<(), String> {
    use sha2::Digest;
    use std::io::Read;
    let expected = expected
        .strip_prefix("sha256:")
        .unwrap_or(expected)
        .trim()
        .to_ascii_lowercase();
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("INTEGRITY_METADATA_INVALID: expected SHA-256 is invalid".to_string());
    }
    let mut file = std::fs::File::open(path).map_err(|e| format!("UPDATE_FILE: {e}"))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("UPDATE_FILE: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        return Err(format!(
            "INTEGRITY_CHECK_FAILED: SHA-256 mismatch, expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

pub async fn download(app_handle: &AppHandle) -> Result<DesktopUpdateInfo, String> {
    let release = fetch_latest_release(app_handle)
        .await?
        .ok_or_else(|| "UPDATE_NONE".to_string())?;
    let path = installer_path(app_handle, &release.asset_name)?;

    if path.exists() {
        log::info!("Installer already downloaded: {}", path.display());
        super::pending::set(app_handle, Some((path.as_path(), &release.version)));
        return check(app_handle)
            .await?
            .ok_or_else(|| "UPDATE_NONE".to_string());
    }

    let client = download_client(app_handle)?;

    let urls = download_sources(&release);
    if urls.len() == 1 {
        log::warn!(
            "No SHA-256 digest available for {}, mirror fallback disabled",
            release.asset_name
        );
    }
    let tmp = path.with_extension("part");
    let mut last_err = String::new();
    for (index, url) in urls.iter().enumerate() {
        if index > 0 {
            let host = reqwest::Url::parse(url)
                .ok()
                .and_then(|parsed| parsed.host_str().map(|h| h.to_string()))
                .unwrap_or_else(|| url.clone());
            log::warn!(
                "Primary desktop update source failed, switching to fallback: {}",
                url
            );
            let _ = app_handle.emit(
                "desktop-update-progress",
                DesktopDownloadProgress {
                    percentage: 0.0,
                    downloaded: 0,
                    total: 0,
                    message: Some(format!("主下载源不可用，已切换镜像源重试（{host}）")),
                },
            );
        }
        let _ = std::fs::remove_file(&tmp);
        match download_from_source(&client, url, &tmp, app_handle).await {
            Ok(()) => {
                last_err.clear();
                break;
            }
            Err(e) => last_err = e,
        }
    }
    if !last_err.is_empty() {
        return Err(format!(
            "UPDATE_DOWNLOAD: {last_err}（已尝试 {} 个下载源）",
            urls.len()
        ));
    }

    if let Some(digest) = &release.digest {
        if let Err(e) = verify_installer_sha256(&tmp, digest) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("UPDATE_DOWNLOAD: {e}"));
        }
        log::info!("Installer SHA-256 verified for {}", release.asset_name);
    }

    std::fs::rename(&tmp, &path).map_err(|e| format!("UPDATE_FILE: {e}"))?;

    #[cfg(unix)]
    ensure_installer_executable(&path)?;

    super::pending::set(app_handle, Some((path.as_path(), &release.version)));

    check(app_handle)
        .await?
        .ok_or_else(|| "UPDATE_NONE".to_string())
}

fn ensure_within_updates_dir(
    canonical: &std::path::Path,
    updates_real: &std::path::Path,
) -> Result<(), String> {
    let has_parent_dir = canonical
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir));
    if has_parent_dir || !canonical.starts_with(updates_real) {
        log::error!(
            "Rejecting installer outside updates dir: {} (root {})",
            canonical.display(),
            updates_real.display()
        );
        return Err(
            "UPDATE_PATH_REJECTED: installer path is outside updates directory".to_string(),
        );
    }
    Ok(())
}

fn resolve_installer_path(app_handle: &AppHandle, path: &str) -> Result<PathBuf, String> {
    let updates_dir = updates_dir(app_handle)?;
    let p = std::path::Path::new(path);
    if !p.exists() || !p.is_file() {
        return Err(format!("UPDATE_NOT_FOUND: {path}"));
    }
    let canonical = dunce::canonicalize(p).map_err(|e| format!("UPDATE_OPEN: {e}"))?;
    let updates_real = dunce::canonicalize(&updates_dir).map_err(|e| format!("UPDATE_DIR: {e}"))?;
    ensure_within_updates_dir(&canonical, &updates_real)?;
    Ok(canonical)
}

#[cfg(target_os = "linux")]
fn is_appimage(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("AppImage"))
}

#[cfg(target_os = "linux")]
fn installer_mime(path: &std::path::Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "deb" => Some("application/vnd.debian.binary-package"),
        "rpm" => Some("application/x-rpm"),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
const MAX_DESKTOP_ENTRY_DEPTH: usize = 4;

#[cfg(target_os = "linux")]
fn applications_dirs_from(
    data_home: Option<&std::ffi::OsStr>,
    data_dirs: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    match data_home.filter(|value| !value.is_empty()) {
        Some(dir) => roots.push(PathBuf::from(dir)),
        None => {
            if let Some(home) = home.filter(|value| !value.is_empty()) {
                roots.push(PathBuf::from(home).join(".local/share"));
            }
        }
    }
    let data_dirs = data_dirs
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
    roots.extend(
        data_dirs
            .split(':')
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from),
    );
    roots
        .into_iter()
        .map(|dir| dir.join("applications"))
        .collect()
}

#[cfg(target_os = "linux")]
fn applications_dirs() -> Vec<PathBuf> {
    applications_dirs_from(
        std::env::var_os("XDG_DATA_HOME").as_deref(),
        std::env::var_os("XDG_DATA_DIRS").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

#[cfg(target_os = "linux")]
fn find_desktop_entry(id: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    if id.contains('/') || id.contains("..") {
        return None;
    }
    dirs.iter().find_map(|dir| find_entry_in_dir(id, dir))
}

#[cfg(target_os = "linux")]
fn find_entry_in_dir(id: &str, dir: &std::path::Path) -> Option<PathBuf> {
    let mut level = vec![(dir.to_path_buf(), String::new())];
    for _ in 0..MAX_DESKTOP_ENTRY_DEPTH {
        let mut next = Vec::new();
        for (current, prefix) in level {
            let Ok(entries) = std::fs::read_dir(&current) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                else {
                    continue;
                };
                if path.is_dir() {
                    next.push((path, format!("{prefix}{name}-")));
                } else if format!("{prefix}{name}") == id {
                    return Some(path);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        level = next;
    }
    None
}

#[cfg(target_os = "linux")]
fn default_handler_id(mime: &str) -> Option<String> {
    let output = std::process::Command::new("xdg-mime")
        .args(["query", "default", mime])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!id.is_empty()).then_some(id)
}

#[cfg(target_os = "linux")]
fn split_exec(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(target_os = "linux")]
fn parse_desktop_entry(text: &str) -> Option<(Vec<String>, bool)> {
    let mut group = String::new();
    let mut exec = None;
    let mut terminal = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            group = line[1..line.len() - 1].to_string();
            continue;
        }
        if group != "Desktop Entry" {
            continue;
        }
        if let Some(value) = line.strip_prefix("Exec=") {
            exec = Some(split_exec(value));
        } else if let Some(value) = line.strip_prefix("Terminal=") {
            terminal = value.trim().eq_ignore_ascii_case("true");
        }
    }
    Some((exec?, terminal))
}

#[cfg(target_os = "linux")]
fn exec_with_file(tokens: &[String], file: &str) -> Vec<String> {
    let mut used = false;
    let mut command = Vec::with_capacity(tokens.len() + 1);
    for token in tokens {
        let mut expanded = String::new();
        let mut chars = token.chars();
        while let Some(ch) = chars.next() {
            if ch != '%' {
                expanded.push(ch);
                continue;
            }
            match chars.next() {
                Some('%') => expanded.push('%'),
                Some(code) if "fFuU".contains(code) => {
                    if !used {
                        expanded.push_str(file);
                        used = true;
                    }
                }
                Some(_) => {}
                None => expanded.push('%'),
            }
        }
        if !expanded.is_empty() {
            command.push(expanded);
        }
    }
    if !used {
        command.push(file.to_string());
    }
    command
}

#[cfg(target_os = "linux")]
fn resolves_to_executable(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let executable = |path: &std::path::Path| {
        std::fs::metadata(path)
            .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    };
    if program.contains('/') {
        return executable(std::path::Path::new(program));
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| executable(&dir.join(program)))
    })
}

#[cfg(target_os = "linux")]
fn spawn_waited_handler(command: &[String]) -> Result<std::process::Child, String> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| "UPDATE_OPEN: empty handler command".to_string())?;
    if !resolves_to_executable(program) {
        return Err(format!("UPDATE_OPEN: handler program not found: {program}"));
    }
    let mut child = std::process::Command::new("/bin/sh");
    child
        .arg("-c")
        .arg(r#""$@""#)
        .arg("sh")
        .arg(program)
        .args(args)
        .stdin(std::process::Stdio::null());
    child.spawn().map_err(|e| format!("UPDATE_OPEN: {e}"))
}

/// 「点了安装、窗口直接消失、什么都没装上」（issue #865）。这里自行解析默认
#[cfg(target_os = "linux")]
fn open_with_system_handler(file: &std::path::Path) -> Result<(), String> {
    let mime = installer_mime(file)
        .ok_or_else(|| "UPDATE_OPEN: unsupported installer type".to_string())?;
    let id =
        default_handler_id(mime).ok_or_else(|| "UPDATE_OPEN: no default handler".to_string())?;
    let entry = find_desktop_entry(&id, &applications_dirs())
        .ok_or_else(|| format!("UPDATE_OPEN: desktop entry not found: {id}"))?;
    let text = std::fs::read_to_string(&entry).map_err(|e| format!("UPDATE_OPEN: {e}"))?;
    let (exec, terminal) = parse_desktop_entry(&text)
        .ok_or_else(|| format!("UPDATE_OPEN: no Exec in {}", entry.display()))?;
    if terminal {
        return Err(format!("UPDATE_OPEN: handler needs a terminal: {id}"));
    }
    let command = exec_with_file(&exec, &file.to_string_lossy());
    spawn_waited_handler(&command).map(|_| ())
}

pub(super) fn open_installer_now(app_handle: &AppHandle, path: &str) -> Result<(), String> {
    let resolved = resolve_installer_path(app_handle, path)?;
    log::info!("Opening desktop installer: {}", resolved.display());
    #[cfg(unix)]
    ensure_installer_executable(&resolved)?;

    #[cfg(target_os = "linux")]
    if is_appimage(&resolved) {
        std::process::Command::new(&resolved)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("UPDATE_OPEN: {e}"))?;
        return Ok(());
    }

    // （issue #865）。处理器解析失败时退回 opener 的分离式交接：至少能打开。
    #[cfg(target_os = "linux")]
    match open_with_system_handler(&resolved) {
        Ok(()) => return Ok(()),
        Err(error) => log::warn!("UPDATE_OPEN: system handler launch failed: {error}"),
    }

    app_handle
        .opener()
        .open_path(resolved.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("UPDATE_OPEN: {e}"))
}

pub async fn open_installer(app_handle: &AppHandle, path: String) -> Result<(), String> {
    if !super::version::supports_stable_updates(&app_handle.config().identifier) {
        return Err(
            "UPDATE_CHANNEL_UNSUPPORTED: nightly builds do not install stable updates".into(),
        );
    }
    let _transition = workflow::acquire_core_transition().await?;
    for profile in workflow::instances::profiles() {
        workflow::instances::scope(profile, async {
            if workflow::has_owned_process() {
                workflow::stop(app_handle.clone()).await?;
            }
            Ok::<(), String>(())
        })
        .await?;
    }
    open_installer_now(app_handle, &path)?;
    super::pending::set(app_handle, None);

    #[cfg(target_os = "linux")]
    if is_appimage(std::path::Path::new(&path)) {
        tauri_plugin_single_instance::destroy(app_handle);
        app_handle.exit(0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immediate_installer_stops_every_profile_before_handoff_and_propagates_failure() {
        let source = include_str!("install.rs");
        let opening = source
            .split("pub async fn open_installer(")
            .nth(1)
            .unwrap()
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        let transition = opening
            .find("workflow::acquire_core_transition().await?")
            .unwrap();
        let profiles = opening
            .find("for profile in workflow::instances::profiles()")
            .unwrap();
        let scope = opening
            .find("workflow::instances::scope(profile, async {")
            .unwrap();
        let stop = opening
            .find("workflow::stop(app_handle.clone()).await?")
            .unwrap();
        let handoff = opening
            .find("open_installer_now(app_handle, &path)?")
            .unwrap();
        assert!(transition < profiles && profiles < scope && scope < stop && stop < handoff);
        assert!(opening[stop..handoff].contains(".await?;"));
    }

    #[test]
    fn prune_installers_keeps_up_to_five() {
        let dir = tempfile::tempdir().unwrap();
        prune_installers_in_dir(dir.path(), &[]).unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("installer-{i}.exe")), b"payload").unwrap();
            prune_installers_in_dir(dir.path(), &[]).unwrap();
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), i + 1);
        }
    }

    #[test]
    fn prune_installers_keeps_five_by_modified_time() {
        let dir = tempfile::tempdir().unwrap();
        let names = [
            "z.exe",
            "a.MSI",
            "y.dmg",
            "b.AppImage",
            "x.deb",
            "c.rpm",
            "w.exe",
            "d.exe",
        ];
        for (i, name) in names.iter().enumerate() {
            let file = std::fs::File::create(dir.path().join(name)).unwrap();
            file.set_times(std::fs::FileTimes::new().set_modified(
                std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + i as u64),
            ))
            .unwrap();
        }
        for name in ["download.part", "notes.txt"] {
            std::fs::write(dir.path().join(name), b"keep").unwrap();
        }
        std::fs::create_dir(dir.path().join("nested.exe")).unwrap();
        std::fs::write(dir.path().join("nested.exe/installer.exe"), b"keep").unwrap();

        prune_installers_in_dir(dir.path(), &[]).unwrap();
        for (i, name) in names.iter().enumerate() {
            assert_eq!(dir.path().join(name).exists(), i >= 3, "{name}");
        }
        for name in ["download.part", "notes.txt", "nested.exe/installer.exe"] {
            assert_eq!(std::fs::read(dir.path().join(name)).unwrap(), b"keep");
        }
    }

    #[test]
    fn prune_installers_preserves_pending_and_running_packages_within_limit() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..8 {
            let file =
                std::fs::File::create(dir.path().join(format!("installer-{i}.exe"))).unwrap();
            file.set_times(
                std::fs::FileTimes::new()
                    .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + i)),
            )
            .unwrap();
        }
        let protected = [
            dir.path().join("installer-0.exe"),
            dir.path().join("installer-1.exe"),
        ];
        prune_installers_in_dir(dir.path(), &protected).unwrap();
        for i in 0..8 {
            assert_eq!(
                dir.path().join(format!("installer-{i}.exe")).exists(),
                i < 2 || i >= 5,
                "{i}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn prune_installers_ignores_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let sentinel = outside.path().join("outside.exe");
        std::fs::write(&sentinel, b"keep").unwrap();
        let link = dir.path().join("linked.exe");
        std::os::unix::fs::symlink(&sentinel, &link).unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("installer-{i}.exe")), b"payload").unwrap();
        }
        prune_installers_in_dir(dir.path(), &[]).unwrap();
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"keep");
        assert!(
            std::fs::symlink_metadata(link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 6);
    }

    /// 下载后补充可执行位：下载默认 0644 的文件被修复为含可执行位（issue #79）。
    #[cfg(unix)]
    #[test]
    fn ensure_installer_executable_sets_exec_bit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("dsh-update-exec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("installer.AppImage");
        std::fs::write(&file, b"payload").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        ensure_installer_executable(&file).unwrap();
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o111,
            0o111,
            "应包含所有者/组/其他可执行位，mode={mode:o}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn appimage_detection_is_extension_based() {
        use std::path::Path;
        assert!(is_appimage(Path::new(
            "/tmp/Deepseek.Harness.Desktop_0.13.0_amd64.AppImage"
        )));
        assert!(is_appimage(Path::new("/tmp/tool.appimage")));
        assert!(!is_appimage(Path::new(
            "/tmp/Deepseek.Harness.Desktop_0.13.0_amd64.deb"
        )));
        assert!(!is_appimage(Path::new(
            "/tmp/Deepseek.Harness.Desktop_0.13.0_x86_64.rpm"
        )));
        assert!(!is_appimage(Path::new("/tmp/AppImage")));
    }

    #[test]
    fn download_sources_only_mirrors_with_digest() {
        let url = "https://github.com/x/y/releases/download/v0.7.4/x.dmg";
        let base = LatestRelease {
            version: "0.7.4".into(),
            tag: "v0.7.4".into(),
            published_at: String::new(),
            url: url.into(),
            asset_name: "x.dmg".into(),
            digest: None,
        };
        let without = download_sources(&base);
        assert_eq!(without.len(), 1);
        assert_eq!(without[0], url);
        let with_digest = LatestRelease {
            digest: Some(format!("sha256:{}", "b".repeat(64))),
            ..base.clone()
        };
        let sources = download_sources(&with_digest);
        assert_eq!(sources.len(), 1 + crate::config::DSH_MIRROR_PREFIXES.len());
        for (source, prefix) in sources[1..].iter().zip(crate::config::DSH_MIRROR_PREFIXES) {
            assert!(
                source.starts_with(prefix),
                "镜像应为 {prefix} 前缀: {source}"
            );
            assert!(
                source.ends_with("/releases/download/v0.7.4/x.dmg"),
                "镜像保留完整资产路径: {source}"
            );
        }
    }

    #[test]
    fn verify_installer_sha256_streams_and_rejects_mismatch() {
        use sha2::Digest;
        let dir = std::env::temp_dir().join(format!("dsh-update-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("installer.part");
        let content = b"deepseek-harness-desktop installer payload";
        std::fs::write(&file, content).unwrap();
        let real = format!("sha256:{:x}", sha2::Sha256::digest(content));
        assert!(verify_installer_sha256(&file, &real).is_ok());
        assert!(verify_installer_sha256(&file, real.trim_start_matches("sha256:")).is_ok());
        let wrong = format!("sha256:{}", "0".repeat(64));
        assert!(verify_installer_sha256(&file, &wrong).is_err());
        assert!(verify_installer_sha256(&file, "sha256:zz").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_within_updates_dir_rejects_outside_paths() {
        let root = std::path::Path::new("root").join("updates");
        assert!(ensure_within_updates_dir(&root.join("app-setup.exe"), &root).is_ok());
        assert!(ensure_within_updates_dir(&root.join("..").join("evil.exe"), &root).is_err());
        let sibling = std::path::Path::new("root")
            .join("updates-evil")
            .join("x.exe");
        assert!(ensure_within_updates_dir(&sibling, &root).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn installer_mime_maps_deb_and_rpm_only() {
        use std::path::Path;
        assert_eq!(
            installer_mime(Path::new("/tmp/App_0.21.0_amd64.deb")),
            Some("application/vnd.debian.binary-package")
        );
        assert_eq!(
            installer_mime(Path::new("/tmp/App_0.21.0_x86_64.RPM")),
            Some("application/x-rpm")
        );
        assert_eq!(installer_mime(Path::new("/tmp/App.AppImage")), None);
        assert_eq!(installer_mime(Path::new("/tmp/App")), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_desktop_entry_reads_main_group_only() {
        let text = "[Desktop Entry]\nType=Application\nTerminal=false\nExec=gdebi-gtk \"%f\" --flag\n\n[Desktop Action Other]\nExec=ignored %f\n";
        let (exec, terminal) = parse_desktop_entry(text).unwrap();
        assert_eq!(
            exec,
            vec![
                "gdebi-gtk".to_string(),
                "%f".to_string(),
                "--flag".to_string()
            ]
        );
        assert!(!terminal);
        let (_, terminal) =
            parse_desktop_entry("[Desktop Entry]\nExec=x\nTerminal=true\n").unwrap();
        assert!(terminal);
        assert!(parse_desktop_entry("[Desktop Entry]\nType=Application\n").is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn exec_with_file_expands_field_codes() {
        fn tokens(list: &[&str]) -> Vec<String> {
            list.iter().map(|item| item.to_string()).collect()
        }
        assert_eq!(
            exec_with_file(&tokens(&["gdebi-gtk", "%f"]), "/tmp/a.deb"),
            tokens(&["gdebi-gtk", "/tmp/a.deb"])
        );
        assert_eq!(
            exec_with_file(&tokens(&["apt", "--file", "%F", "%i"]), "/tmp/a.deb"),
            tokens(&["apt", "--file", "/tmp/a.deb"])
        );
        assert_eq!(
            exec_with_file(&tokens(&["gdebi-gtk"]), "/tmp/a.deb"),
            tokens(&["gdebi-gtk", "/tmp/a.deb"])
        );
        assert_eq!(
            exec_with_file(&tokens(&["sh", "-c", "echo 100%%"]), "/tmp/a.deb"),
            tokens(&["sh", "-c", "echo 100%", "/tmp/a.deb"])
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn find_desktop_entry_searches_dirs_in_order() {
        let dir = std::env::temp_dir().join(format!("dsh-desktop-entry-{}", std::process::id()));
        let empty = dir.join("empty");
        let hit = dir.join("hit");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::create_dir_all(&hit).unwrap();
        std::fs::write(
            hit.join("gdebi.desktop"),
            "[Desktop Entry]\nExec=gdebi-gtk %f\n",
        )
        .unwrap();
        assert_eq!(
            find_desktop_entry("gdebi.desktop", &[empty.clone(), hit.clone()]),
            Some(hit.join("gdebi.desktop"))
        );
        assert_eq!(
            find_desktop_entry("missing.desktop", &[empty, hit.clone()]),
            None
        );
        assert_eq!(find_desktop_entry("../gdebi.desktop", &[hit]), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn find_desktop_entry_derives_ids_for_nested_dirs() {
        let dir = std::env::temp_dir().join(format!("dsh-desktop-nested-{}", std::process::id()));
        let apps = dir.join("applications");
        std::fs::create_dir_all(apps.join("vendor")).unwrap();
        std::fs::create_dir_all(apps.join("vendor/deep")).unwrap();
        std::fs::write(apps.join("vendor/gdebi.desktop"), "nested").unwrap();
        std::fs::write(apps.join("vendor/deep/tool.desktop"), "deep").unwrap();
        std::fs::write(apps.join("flat-gdebi.desktop"), "flat").unwrap();

        assert_eq!(
            find_desktop_entry("vendor-gdebi.desktop", std::slice::from_ref(&apps)),
            Some(apps.join("vendor/gdebi.desktop"))
        );
        assert_eq!(
            find_desktop_entry("vendor-deep-tool.desktop", std::slice::from_ref(&apps)),
            Some(apps.join("vendor/deep/tool.desktop"))
        );
        assert_eq!(
            find_desktop_entry("flat-gdebi.desktop", std::slice::from_ref(&apps)),
            Some(apps.join("flat-gdebi.desktop")),
            "嵌套文件名本身不构成 id，只有推导出的 id 参与匹配"
        );
        assert_eq!(
            find_desktop_entry("gdebi.desktop", std::slice::from_ref(&apps)),
            None,
            "仅作为嵌套文件名的 gdebi.desktop 不属于顶层 id"
        );

        std::fs::write(apps.join("vendor-flat.desktop"), "shallow").unwrap();
        std::fs::write(apps.join("vendor/flat.desktop"), "deep").unwrap();
        assert_eq!(
            find_desktop_entry("vendor-flat.desktop", std::slice::from_ref(&apps)),
            Some(apps.join("vendor-flat.desktop")),
            "顶层与嵌套推出同一 id 时浅层优先"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn applications_dirs_treats_empty_xdg_values_as_unset() {
        use std::ffi::OsStr;
        let home = OsStr::new("/home/tester");
        let defaults = applications_dirs_from(None, None, Some(home));
        assert_eq!(
            defaults,
            vec![
                PathBuf::from("/home/tester/.local/share/applications"),
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/usr/share/applications"),
            ]
        );
        assert_eq!(
            applications_dirs_from(Some(OsStr::new("")), Some(OsStr::new("")), Some(home)),
            defaults,
            "空串应与未设置等价"
        );
        assert_eq!(
            applications_dirs_from(
                Some(OsStr::new("/opt/data")),
                Some(OsStr::new("/opt/a:/opt/b")),
                Some(home)
            ),
            vec![
                PathBuf::from("/opt/data/applications"),
                PathBuf::from("/opt/a/applications"),
                PathBuf::from("/opt/b/applications"),
            ]
        );
        assert_eq!(
            applications_dirs_from(None, None, None),
            vec![
                PathBuf::from("/usr/local/share/applications"),
                PathBuf::from("/usr/share/applications"),
            ],
            "无 HOME 时不应造出相对路径"
        );
    }

    /// 处理器必须始终有活着的父进程（issue #865）：由等待子进程的 shell 启动后，
    #[cfg(target_os = "linux")]
    #[test]
    fn waited_handler_keeps_parent_alive() {
        let dir = std::env::temp_dir().join(format!("dsh-handler-ppid-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ppid_file = dir.join("ppid.txt");
        let command = vec![
            "sh".to_string(),
            "-c".to_string(),
            format!("echo $PPID > {}; sleep 1", ppid_file.display()),
        ];
        let mut child = spawn_waited_handler(&command).unwrap();
        let mut recorded = None;
        for _ in 0..100 {
            if let Ok(text) = std::fs::read_to_string(&ppid_file) {
                recorded = text.trim().parse::<u32>().ok();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let parent = recorded.expect("处理器应写出自己的 PPID");
        assert_eq!(parent, child.id(), "处理器的父进程应是等待它的 shell");
        assert_ne!(parent, 1, "处理器不应被 init 收养");
        assert!(
            child.try_wait().unwrap().is_none(),
            "等待子进程的 shell 应在处理器运行期间存活"
        );
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn missing_handler_program_is_rejected() {
        assert!(spawn_waited_handler(&[]).is_err());
        assert!(spawn_waited_handler(&["/nonexistent/gdebi-gtk".to_string()]).is_err());
        assert!(spawn_waited_handler(&["no-such-handler-9f3a".to_string()]).is_err());
        let dir = std::env::temp_dir().join(format!("dsh-handler-mode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plain = dir.join("plain-handler");
        std::fs::write(&plain, b"#!/bin/sh\n").unwrap();
        assert!(
            spawn_waited_handler(&[plain.to_string_lossy().into_owned()]).is_err(),
            "存在但无执行位的程序应被拒绝"
        );
        let mut child = spawn_waited_handler(&["/bin/true".to_string()]).unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(0));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
