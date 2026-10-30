use serde::Serialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{Manager, Runtime};

use super::constants::*;
use super::dependencies;
use super::{Region, detect_region};
use crate::AppHandle;

pub fn get_base_dir<R: Runtime>(app_handle: &AppHandle<R>) -> PathBuf {
    base_dir_from(|| {
        app_handle
            .path()
            .app_data_dir()
            .expect("Failed to resolve app data directory")
    })
}

pub(crate) fn base_dir_from(app_data: impl FnOnce() -> PathBuf) -> PathBuf {
    if let Some(dir) = env::var_os(DOWNLOAD_CACHE_ENV_VAR).filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    let base = app_data();
    if cfg!(debug_assertions) {
        base.join(APP_DATA_DEV_DIR_NAME)
    } else {
        base
    }
}

/// Upstream publishes both a plain `node_modules` zip and an `.asar.zip` holding a
/// single `.asar`. The desktop runs the asar: Electron patches `fs` for it, so the
/// dsh bin inside is loadable while the archive stays one file on disk.
fn dsh_pkg_asset_filename(os: &str, arch: &str) -> Result<String, String> {
    let stem = match (os, arch) {
        ("windows", _) => "deepseek-harness-pkg-windows",
        ("macos", "aarch64") => "deepseek-harness-pkg-macos-arm64",
        ("macos", "x86_64") => "deepseek-harness-pkg-macos-x64",
        ("linux", "x86_64") => "deepseek-harness-pkg-linux",
        ("linux", "aarch64") => "deepseek-harness-pkg-linux-arm64",
        _ => return Err(format!("Unsupported platform: {} {}", os, arch)),
    };
    Ok(format!("{stem}.asar.zip"))
}

pub fn get_dsh_download_url() -> Result<String, String> {
    Ok(format!(
        "{}{}",
        DSH_CORE_URL,
        dsh_pkg_asset_filename(env::consts::OS, env::consts::ARCH)?
    ))
}

pub fn mirror_download_urls(asset_url: &str) -> Vec<String> {
    DSH_MIRROR_PREFIXES
        .iter()
        .map(|prefix| format!("{prefix}{asset_url}"))
        .collect()
}

pub fn get_dsh_download_url_for_tag(tag: &str) -> Result<String, String> {
    let base = DSH_CORE_URL.replace(
        "releases/latest/download/",
        &format!("releases/download/{tag}/"),
    );
    Ok(format!(
        "{}{}",
        base,
        dsh_pkg_asset_filename(env::consts::OS, env::consts::ARCH)?
    ))
}

fn find_local_node_binary() -> Option<PathBuf> {
    let bin_name = if cfg!(windows) { "node.exe" } else { "node" };

    let path_dirs: Vec<PathBuf> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .filter(|dir| !dir.as_os_str().is_empty())
            .collect();

    #[cfg(target_os = "macos")]
    let dirs: Vec<PathBuf> = {
        let mut dirs = path_dirs;
        dirs.extend([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ]);
        dirs
    };

    #[cfg(not(target_os = "macos"))]
    let dirs = path_dirs;

    for dir in dirs {
        let candidate = dir.join(bin_name);
        if candidate.is_file() && is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|meta| meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    true
}

fn node_version_output(node: &Path, args: &[&str]) -> Option<std::process::Output> {
    #[cfg(windows)]
    if let Some(output) = crate::service::workflow::early::probe_output(node, args, true) {
        return output.ok();
    }
    let mut command = std::process::Command::new(node);
    command.args(args).env_remove("NODE_OPTIONS");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.output().ok()
}

fn probe_node_version(node: &Path) -> Option<(String, Option<PathBuf>)> {
    if let Some(output) = node_version_output(
        node,
        &[
            "-p",
            "JSON.stringify([process.versions.node,process.execPath])",
        ],
    ) {
        if output.status.success() {
            if let Ok((version, actual)) =
                serde_json::from_slice::<(String, PathBuf)>(&output.stdout)
            {
                if semver::Version::parse(&version).is_ok() && actual.is_absolute() {
                    return Some((version, Some(actual)));
                }
            }
        }
    }
    let output = node_version_output(node, &["--version"])?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = stdout.trim().trim_start_matches('v');
    (!version.is_empty()).then(|| (version.to_string(), None))
}

type NodeVersionIdentity = (PathBuf, u64, std::time::SystemTime);

static NODE_VERSION_CACHE: std::sync::Mutex<Option<(NodeVersionIdentity, String)>> =
    std::sync::Mutex::new(None);

fn cached_node_version(
    node: &Path,
    cache: &std::sync::Mutex<Option<(NodeVersionIdentity, String)>>,
    probe: impl FnOnce() -> Option<(String, Option<PathBuf>)>,
) -> Option<String> {
    let identity = || {
        let path = dunce::canonicalize(node).ok()?;
        let meta = fs::metadata(&path).ok()?;
        meta.is_file()
            .then_some((path, meta.len(), meta.modified().ok()?))
    };
    // ponytail: 串行版本探测；多运行时高并发时再按文件身份分锁。
    let mut guard = cache.lock().unwrap_or_else(|error| error.into_inner());
    let Some(key) = identity() else {
        drop(guard);
        return probe().map(|(version, _)| version);
    };
    if let Some((cached_key, version)) = guard.as_ref() {
        if cached_key == &key {
            return Some(version.clone());
        }
    }
    let (version, actual) = probe()?;
    if actual
        .and_then(|path| dunce::canonicalize(path).ok())
        .as_ref()
        == Some(&key.0)
        && identity().as_ref() == Some(&key)
    {
        *guard = Some((key, version.clone()));
    }
    Some(version)
}

pub fn get_node_version_of_path(node: &Path) -> Option<String> {
    cached_node_version(node, &NODE_VERSION_CACHE, || probe_node_version(node))
}

pub fn get_local_node_path() -> Option<PathBuf> {
    let node = find_local_node_binary()?;
    let version = get_node_version_of_path(&node)?;
    is_supported_node_version(&version).then_some(node)
}

/// 只按 semver 挑选，可能与其不匹配（issue #441：Node 25 的 ABI 141 加载 ABI 137 的
pub fn electron_node_binary() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let directory = executable.parent()?;
    for root in [
        directory.join("runtime"),
        directory.join("resources/runtime"),
        directory.join("../Resources/runtime"),
    ] {
        if let Ok(binary) = electron_rs::electron_binary(&root) {
            return Some(binary);
        }
    }
    #[cfg(debug_assertions)]
    if let Ok(binary) = electron_rs::electron_binary(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/electron"),
    ) {
        return Some(binary);
    }
    None
}

pub fn get_node_binary_path<R: Runtime>(_app_handle: &AppHandle<R>) -> PathBuf {
    electron_node_binary()
        .or_else(get_local_node_path)
        .unwrap_or_default()
}

pub fn get_dsh_install_path<R: Runtime>(app_handle: &AppHandle<R>) -> PathBuf {
    dependencies::active_root(app_handle, dependencies::DEP_DSH)
}

pub fn get_dsh_binary_path<R: Runtime>(app_handle: &AppHandle<R>) -> PathBuf {
    dependencies::binary_path(app_handle, dependencies::DEP_DSH)
}

pub fn get_pnpm_install_path<R: Runtime>(app_handle: &AppHandle<R>) -> PathBuf {
    dependencies::active_root(app_handle, dependencies::DEP_PNPM)
}

pub fn get_pnpm_binary_path<R: Runtime>(app_handle: &AppHandle<R>) -> PathBuf {
    dependencies::binary_path(app_handle, dependencies::DEP_PNPM)
}

fn pnpm_base_url(region: Region) -> &'static str {
    match region {
        Region::Domestic => PNPM_MIRROR_BASE_URL,
        Region::Overseas => PNPM_BASE_URL,
    }
}

pub fn get_pnpm_download_url() -> String {
    format!(
        "{}pnpm-{}.tgz",
        pnpm_base_url(detect_region()),
        PNPM_VERSION
    )
}

#[cfg_attr(not(windows), allow(dead_code))] // 仅 Windows 的 Git 运行时探测使用
#[cfg_attr(not(windows), allow(dead_code))] // 仅 Windows 的 Git 运行时探测使用
#[cfg_attr(all(not(windows), not(test)), allow(dead_code))] // 仅 Windows 的 MinGit 任务与单测使用
#[cfg_attr(not(windows), allow(dead_code))] // 仅 Windows 的 MinGit 任务使用
#[cfg_attr(not(windows), allow(dead_code))] // 仅 Windows 的 MinGit 任务使用
#[cfg(windows)]
pub fn find_system_git_binary() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join("git.exe"))
        .find(|candidate| candidate.is_file() && git_binary_works(candidate))
}

#[cfg(windows)]
fn git_output(binary: &Path, arg: &str) -> Option<std::process::Output> {
    if let Some(output) = crate::service::workflow::early::probe_output(binary, &[arg], false) {
        return output.ok();
    }
    use std::os::windows::process::CommandExt;

    std::process::Command::new(binary)
        .arg(arg)
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .ok()
}

/// 启动时把已安装且可用的 MinGit 误判为缺失，反复进入依赖安装流程（issue #166）。
#[cfg(any(windows, test))]
fn git_https_helper_exists(exec_path: &Path) -> bool {
    const HELPER: &str = "git-remote-https.exe";

    if exec_path.join(HELPER).is_file() {
        return true;
    }

    exec_path
        .parent()
        .and_then(Path::parent)
        .is_some_and(|platform_root| platform_root.join("bin").join(HELPER).is_file())
}

#[cfg(windows)]
fn git_binary_works(binary: &Path) -> bool {
    if !git_output(binary, "--version").is_some_and(|output| output.status.success()) {
        return false;
    }
    let Some(output) = git_output(binary, "--exec-path").filter(|output| output.status.success())
    else {
        return false;
    };
    let exec_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    !exec_path.is_empty() && git_https_helper_exists(Path::new(&exec_path))
}

pub fn get_git_cmd_dir<R: Runtime>(_app_handle: &AppHandle<R>) -> Option<PathBuf> {
    system_git_cmd_dir()
}

pub fn get_git_cmd_dir_at() -> Option<PathBuf> {
    system_git_cmd_dir()
}

pub fn apply_node_env(command: &mut std::process::Command) {
    command.env("ELECTRON_RUN_AS_NODE", "1");
}

pub fn node_env_value() -> (&'static str, &'static str) {
    ("ELECTRON_RUN_AS_NODE", "1")
}

pub fn get_node_binary_path_at() -> Option<PathBuf> {
    electron_node_binary().or_else(get_local_node_path)
}

#[cfg(windows)]
fn system_git_cmd_dir() -> Option<PathBuf> {
    find_system_git_binary().and_then(|git| git.parent().map(Path::to_path_buf))
}

#[cfg(not(windows))]
fn system_git_cmd_dir() -> Option<PathBuf> {
    None
}

pub fn get_dsh_package_json_path<R: Runtime>(app_handle: &AppHandle<R>) -> PathBuf {
    get_dsh_install_path(app_handle).join(DSH_MANIFEST_RELATIVE)
}

pub(crate) fn user_home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let key = "USERPROFILE";
    #[cfg(not(windows))]
    let key = "HOME";
    std::env::var_os(key).map(PathBuf::from)
}

pub fn get_dsh_data_path<R: Runtime>(_app_handle: &AppHandle<R>) -> PathBuf {
    dsh_data_path_from_env()
}

pub(crate) fn dsh_data_path_from_env() -> PathBuf {
    let dir_name = if cfg!(debug_assertions) {
        DSH_HOME_DEV_DIR_NAME
    } else {
        if let Some(home) = std::env::var_os("DSH_HOME") {
            if !home.is_empty() {
                return PathBuf::from(home);
            }
        }
        DSH_HOME_DIR_NAME
    };
    user_home_dir()
        .map(|home| home.join(dir_name))
        .unwrap_or_else(|| PathBuf::from(dir_name))
}

pub fn get_service_log_path<R: Runtime>(app_handle: &AppHandle<R>) -> PathBuf {
    service_log_path_from(
        &get_base_dir(app_handle),
        crate::service::workflow::instances::current_profile().as_deref(),
    )
}

pub(crate) fn service_log_path_from(base: &Path, profile: Option<&str>) -> PathBuf {
    let name = if cfg!(debug_assertions) {
        "dsh-web.dev.log"
    } else {
        "dsh-web.log"
    };
    let logs = base.join("logs");
    match profile {
        Some(profile) => logs.join(profile).join(name),
        None => logs.join(name),
    }
}

pub fn get_active_node_version() -> String {
    get_local_node_path()
        .and_then(|node| get_node_version_of_path(&node))
        .unwrap_or_default()
}

fn parse_node_version(output: &str) -> Option<(u64, u64, u64)> {
    let version = output.trim().trim_start_matches('v');
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

fn is_supported_node_version(version: &str) -> bool {
    let Some((major, minor, _patch)) = parse_node_version(version) else {
        return false;
    };
    match major {
        22 => minor >= 19,
        major if major >= 24 => true,
        _ => false,
    }
}

pub fn is_runtime_compatible(app_handle: &AppHandle) -> bool {
    get_node_binary_path(app_handle).is_file()
}

pub fn get_dsh_version<R: Runtime>(app_handle: &AppHandle<R>) -> Option<String> {
    let manifest = dsh_package_manifest(app_handle)?;
    manifest
        .get("dependencies")
        .and_then(|deps| deps.get("@deepseek-ai/dsh"))
        .and_then(|value| value.as_str())
        .map(|value| {
            value
                .trim_start_matches(['^', '~', '=', '>', '<'])
                .to_string()
        })
        .or_else(|| {
            manifest
                .get("name")
                .and_then(|value| value.as_str())
                .filter(|name| *name == "@deepseek-ai/dsh")
                .and_then(|_| manifest.get("version"))
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
}

/// The dsh `package.json`, which lives inside the shipped asar.
///
/// The install root only holds `dsh.asar`; the manifest entry pins the reader to
/// `dsh.asar/node_modules/@deepseek-ai/dsh/package.json`, and that path is
/// resolved through the asar reader instead of `fs`.
fn dsh_package_manifest<R: Runtime>(app_handle: &AppHandle<R>) -> Option<serde_json::Value> {
    let install_root = get_dsh_install_path(app_handle);
    let inside_asar = crate::service::download::read_asar_entry(
        &install_root.join(DSH_ASAR_FILE),
        DSH_MANIFEST_IN_ASAR,
    );
    match inside_asar {
        Ok(bytes) => serde_json::from_slice(&bytes).ok(),
        Err(error) => {
            // Local cores and the plain-node_modules layout keep it on disk.
            let manifest_path = get_dsh_package_json_path(app_handle);
            match fs::read_to_string(&manifest_path) {
                Ok(content) => serde_json::from_str(&content).ok(),
                Err(_) => {
                    log::debug!(
                        "dsh manifest unavailable ({error}); tried {}",
                        manifest_path.display()
                    );
                    None
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeInfo {
    pub app_version: String,
    pub dsh_version: Option<String>,
    pub node_version: String,
    pub service_url: String,
    pub data_dir: String,
    pub log_path: String,
    pub platform: String,
    pub arch: String,
}

pub fn runtime_info<R: Runtime>(app: &AppHandle<R>, port: u16) -> RuntimeInfo {
    RuntimeInfo {
        app_version: app.package_info().version.to_string(),
        dsh_version: get_dsh_version(app),
        node_version: get_active_node_version(),
        service_url: get_dsh_service_url(port),
        data_dir: get_dsh_data_path(app).to_string_lossy().into_owned(),
        log_path: get_service_log_path(app).to_string_lossy().into_owned(),
        platform: env::consts::OS.to_string(),
        arch: env::consts::ARCH.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn base_directory_preserves_build_split_and_cache_short_circuit() {
        let override_dir = env::var_os(DOWNLOAD_CACHE_ENV_VAR).filter(|dir| !dir.is_empty());
        let base = base_dir_from(|| {
            assert!(
                override_dir.is_none(),
                "cache override must not resolve AppData"
            );
            PathBuf::from("app-data")
        });
        let expected = override_dir.map(PathBuf::from).unwrap_or_else(|| {
            if cfg!(debug_assertions) {
                PathBuf::from("app-data").join(APP_DATA_DEV_DIR_NAME)
            } else {
                PathBuf::from("app-data")
            }
        });
        assert_eq!(base, expected);
    }

    #[test]
    fn service_log_path_keeps_profile_and_build_filename() {
        let base = Path::new("app-data");
        let name = if cfg!(debug_assertions) {
            "dsh-web.dev.log"
        } else {
            "dsh-web.log"
        };
        assert_eq!(
            service_log_path_from(base, None),
            base.join("logs").join(name)
        );
        assert_eq!(
            service_log_path_from(base, Some("profile-a")),
            base.join("logs").join("profile-a").join(name)
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn node_version_cache_runs_probe_once_for_the_same_resolved_file() {
        let root = tempfile::tempdir().unwrap();
        let node = root.path().join("node");
        fs::write(&node, b"fixture").unwrap();
        let cache = std::sync::Mutex::new(None);
        let calls = std::cell::Cell::new(0);
        for path in [&node, &root.path().join(".").join("node")] {
            assert_eq!(
                cached_node_version(path, &cache, || {
                    calls.set(calls.get() + 1);
                    Some(("24.0.0".into(), Some(node.clone())))
                }),
                Some("24.0.0".into())
            );
        }
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn node_version_cache_merges_concurrent_probes_of_the_same_file() {
        let root = tempfile::tempdir().unwrap();
        let node = root.path().join("node");
        fs::write(&node, b"fixture").unwrap();
        let cache = std::sync::Mutex::new(None);
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let barrier = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    barrier.wait();
                    assert_eq!(
                        cached_node_version(&node, &cache, || {
                            calls.fetch_add(1, Ordering::SeqCst);
                            Some(("24.0.0".into(), Some(node.clone())))
                        }),
                        Some("24.0.0".into())
                    );
                });
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn node_version_cache_invalidates_when_size_mtime_or_path_changes() {
        let root = tempfile::tempdir().unwrap();
        let node = root.path().join("node");
        fs::write(&node, b"first").unwrap();
        let cache = std::sync::Mutex::new(None);
        let calls = std::cell::Cell::new(0);
        let probe = |path: &Path| {
            calls.set(calls.get() + 1);
            Some((format!("24.0.{}", calls.get()), Some(path.to_path_buf())))
        };
        assert_eq!(
            cached_node_version(&node, &cache, || probe(&node)),
            Some("24.0.1".into())
        );
        let original_modified = fs::metadata(&node).unwrap().modified().unwrap();
        fs::write(&node, b"replacement").unwrap();
        fs::File::options()
            .write(true)
            .open(&node)
            .unwrap()
            .set_modified(original_modified)
            .unwrap();
        assert_eq!(
            cached_node_version(&node, &cache, || probe(&node)),
            Some("24.0.2".into())
        );
        let modified = fs::metadata(&node).unwrap().modified().unwrap();
        fs::File::options()
            .write(true)
            .open(&node)
            .unwrap()
            .set_modified(modified + std::time::Duration::from_secs(2))
            .unwrap();
        assert_eq!(
            cached_node_version(&node, &cache, || probe(&node)),
            Some("24.0.3".into())
        );
        let other = root.path().join("other-node");
        fs::copy(&node, &other).unwrap();
        fs::File::options()
            .write(true)
            .open(&other)
            .unwrap()
            .set_modified(fs::metadata(&node).unwrap().modified().unwrap())
            .unwrap();
        assert_eq!(
            cached_node_version(&other, &cache, || probe(&other)),
            Some("24.0.4".into())
        );
        assert_eq!(calls.get(), 4);
    }

    #[test]
    fn node_version_cache_does_not_store_a_probe_if_the_file_changes_during_it() {
        let root = tempfile::tempdir().unwrap();
        let node = root.path().join("node");
        fs::write(&node, b"first").unwrap();
        let cache = std::sync::Mutex::new(None);
        assert_eq!(
            cached_node_version(&node, &cache, || {
                fs::write(&node, b"replacement").unwrap();
                Some(("24.0.0".into(), Some(node.clone())))
            }),
            Some("24.0.0".into())
        );
        assert_eq!(
            cached_node_version(&node, &cache, || Some((
                "24.0.1".into(),
                Some(node.clone())
            ))),
            Some("24.0.1".into())
        );
    }

    #[test]
    fn node_version_cache_does_not_reuse_a_fixed_shim_after_its_target_version_changes() {
        let root = tempfile::tempdir().unwrap();
        let node = root
            .path()
            .join(if cfg!(windows) { "node.cmd" } else { "node" });
        let script = if cfg!(windows) {
            "@echo off\r\nif \"%~1\"==\"-p\" (type \"%~dp0identity\") else (type \"%~dp0version\")\r\n"
        } else {
            "#!/bin/sh\nif [ \"$1\" = '-p' ]; then cat \"${0%/*}/identity\"; else cat \"${0%/*}/version\"; fi\n"
        };
        fs::write(&node, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&node, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let actual = root.path().join("actual-node");
        fs::write(&actual, b"target").unwrap();
        let before = fs::metadata(&node).unwrap();
        fs::write(root.path().join("version"), "v22.19.0\n").unwrap();
        fs::write(
            root.path().join("identity"),
            serde_json::to_vec(&("22.19.0", &actual)).unwrap(),
        )
        .unwrap();
        let first = get_node_version_of_path(&node).unwrap();
        assert_eq!(first, "22.19.0");
        assert!(is_supported_node_version(&first));
        fs::write(root.path().join("version"), "v23.0.0\n").unwrap();
        fs::write(
            root.path().join("identity"),
            serde_json::to_vec(&("23.0.0", &actual)).unwrap(),
        )
        .unwrap();
        let second = get_node_version_of_path(&node).unwrap();
        assert_eq!(second, "23.0.0");
        assert!(!is_supported_node_version(&second));
        let after = fs::metadata(&node).unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        fs::write(root.path().join("identity"), b"invalid JSON").unwrap();
        for version in ["22.19.0", "23.0.0"] {
            fs::write(root.path().join("version"), format!("v{version}\n")).unwrap();
            assert_eq!(get_node_version_of_path(&node).as_deref(), Some(version));
        }
    }

    #[test]
    fn node_version_cache_requires_an_existing_matching_exec_path() {
        let root = tempfile::tempdir().unwrap();
        let node = root.path().join("node");
        fs::write(&node, b"fixture").unwrap();
        let cache = std::sync::Mutex::new(None);
        for actual in [None, Some(root.path().join("missing-node"))] {
            for version in ["22.19.0", "23.0.0"] {
                assert_eq!(
                    cached_node_version(&node, &cache, || Some((version.into(), actual.clone()))),
                    Some(version.into())
                );
            }
        }
    }

    #[test]
    fn node_version_cache_executes_the_actual_node_probe_once() {
        let candidate = find_local_node_binary().expect("Node.js required for version probe tests");
        let (_, actual) = probe_node_version(&candidate).expect("working Node.js required");
        let node = actual.expect("Node.js must expose its real executable path");
        let output = node_version_output(&node, &["--version"]).unwrap();
        assert!(output.status.success());
        let expected = String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .trim_start_matches('v')
            .to_string();
        let cache = std::sync::Mutex::new(None);
        let calls = std::cell::Cell::new(0);
        for _ in 0..3 {
            assert_eq!(
                cached_node_version(&node, &cache, || {
                    calls.set(calls.get() + 1);
                    let result = probe_node_version(&node).unwrap();
                    assert_eq!(
                        dunce::canonicalize(result.1.as_ref().unwrap()).unwrap(),
                        dunce::canonicalize(&node).unwrap()
                    );
                    Some(result)
                }),
                Some(expected.clone())
            );
        }
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn node_version_cache_does_not_remember_failed_probes() {
        let root = tempfile::tempdir().unwrap();
        let node = root.path().join("node");
        fs::write(&node, b"fixture").unwrap();
        let cache = std::sync::Mutex::new(None);
        assert_eq!(cached_node_version(&node, &cache, || None), None);
        assert_eq!(
            cached_node_version(&node, &cache, || Some((
                "24.0.0".into(),
                Some(node.clone())
            ))),
            Some("24.0.0".into())
        );
        assert_eq!(
            cached_node_version(&node, &cache, || panic!("success must be cached")),
            Some("24.0.0".into())
        );
    }

    #[test]
    fn node_version_cache_never_hits_when_file_identity_is_unavailable() {
        let root = tempfile::tempdir().unwrap();
        let node = root.path().join("node");
        fs::write(&node, b"fixture").unwrap();
        let cache = std::sync::Mutex::new(None);
        assert_eq!(
            cached_node_version(&node, &cache, || Some((
                "24.0.0".into(),
                Some(node.clone())
            ))),
            Some("24.0.0".into())
        );
        fs::remove_file(&node).unwrap();
        for version in ["24.0.1", "24.0.2"] {
            assert_eq!(
                cached_node_version(&node, &cache, || Some((version.into(), Some(node.clone())))),
                Some(version.into())
            );
        }
        fs::create_dir(&node).unwrap();
        for version in ["24.0.3", "24.0.4"] {
            assert_eq!(
                cached_node_version(&node, &cache, || Some((version.into(), Some(node.clone())))),
                Some(version.into())
            );
        }
    }

    fn unique_runtime_test_dir(name: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("dsh-runtime-{name}-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn mirror_urls_expand_every_prefix() {
        let asset = "https://github.com/dsh-tauri-desk/deepseek-harness-pkg/releases/download/v1.0.0/deepseek-harness-pkg-windows.zip";
        let urls = mirror_download_urls(asset);
        assert_eq!(urls.len(), DSH_MIRROR_PREFIXES.len());
        for (url, prefix) in urls.iter().zip(DSH_MIRROR_PREFIXES) {
            assert_eq!(url, &format!("{prefix}{asset}"));
        }
    }

    #[test]
    fn pnpm_base_url_switches_on_region() {
        assert_eq!(pnpm_base_url(Region::Overseas), PNPM_BASE_URL);
        assert_eq!(pnpm_base_url(Region::Domestic), PNPM_MIRROR_BASE_URL);
    }

    #[test]
    fn download_urls_keep_platform_filename_shape() {
        let dsh = get_dsh_download_url().expect("dsh url");
        assert!(dsh.starts_with("https://"));
        assert!(
            dsh.ends_with(".asar.zip"),
            "the desktop runs the asar build: {dsh}"
        );
    }

    #[test]
    fn dsh_pkg_asset_filename_matches_linux_architecture() {
        assert_eq!(
            dsh_pkg_asset_filename("linux", "x86_64").unwrap(),
            "deepseek-harness-pkg-linux.asar.zip"
        );
        assert_eq!(
            dsh_pkg_asset_filename("linux", "aarch64").unwrap(),
            "deepseek-harness-pkg-linux-arm64.asar.zip"
        );
        assert!(dsh_pkg_asset_filename("linux", "riscv64").is_err());
    }
}
