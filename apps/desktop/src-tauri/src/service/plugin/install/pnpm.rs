//! 用户 pnpm 探测（注入桌面端选定 Node 的 PATH，见 issue #182；stdin 关闭、wait/
//! 捆绑版 pnpm；只有进程状态无法确认时才 fail-closed 返回 `Err`（issue #449）。

use crate::config;
use crate::service::cli;
use crate::service::download;
use crate::service::download::Installable;
use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::Emitter;

use super::PREINSTALL_LOG_EVENT;
use super::PidGuard;
use super::PreinstallLogPayload;
use super::ProcessOwner;
use super::acquire_process_lock;
use super::new_process_owner;
use super::profile_dir;
use crate::{AppHandle, WebviewWindow};

const MIN_TRUSTED_PNPM_MAJOR: u32 = 10;
#[cfg(windows)]
const WINDOWS_SPACE_PROBE_ARG: &str = "--config.dsh-probe=with space";
const PNPM_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const PNPM_PROBE_CLEANUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const PNPM_PROBE_REAP_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(25);
const PNPM_PROBE_LIVENESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

pub(super) async fn ensure_pnpm(
    app_handle: &AppHandle,
    window: &WebviewWindow,
    owner: ProcessOwner,
) -> Result<bool, String> {
    let store_major = profile_store_major(app_handle);
    let user_major = user_pnpm_major_version_bounded(app_handle, owner).await?;

    if let Some(store) = store_major {
        if user_major == Some(store) {
            log::info!("Reusing user-installed pnpm (major {store}) matching profile store");
            return Ok(false);
        }
        if bundled_pnpm_major(app_handle) == Some(store) {
            log::info!("Using bundled pnpm (major {store}) matching profile store");
            return Ok(true);
        }
        log::warn!(
            "No pnpm matches profile store major {store} (user {user_major:?}), resetting profile modules metadata"
        );
        if reset_incompatible_modules_metadata(app_handle) {
            let _ = window.emit(
                PREINSTALL_LOG_EVENT,
                PreinstallLogPayload {
                    line: format!(
                        "[pnpm] no pnpm matches this profile's store major {store}; rebuilt the profile modules metadata for the pnpm in use"
                    ),
                },
            );
        }
    }

    match user_major {
        Some(major) if major >= MIN_TRUSTED_PNPM_MAJOR => {
            log::info!("Reusing user-installed pnpm (major {major}) for plugin install");
            return Ok(false);
        }
        Some(major) => {
            log::warn!(
                "User pnpm major {major} < {MIN_TRUSTED_PNPM_MAJOR} (missing autoInstallPeers/workspace-root semantics), using bundled pnpm"
            );
        }
        None => {
            log::warn!(
                "User pnpm version not detectable (broken/blocked shim?), using bundled pnpm"
            );
        }
    }

    if config::get_pnpm_binary_path(app_handle).exists() {
        return Ok(true);
    }

    let _ = window.emit(
        PREINSTALL_LOG_EVENT,
        PreinstallLogPayload {
            line: "[pnpm] bundled pnpm not found, downloading before plugin install".to_string(),
        },
    );

    let tracker = download::ProgressTracker::new(window, 2);
    let url = download::Pnpm.get_download_url()?;
    let name = url.split('/').next_back().unwrap_or(&url).to_string();
    let buffer = download::download_file(&tracker, url)
        .await
        .map_err(|e| format!("PNPM_DOWNLOAD_FAILED: {e}"))?;
    download::verify_sha256(&buffer, config::PNPM_SHA256)
        .map_err(|e| format!("PNPM_INTEGRITY_FAILED: {e}"))?;
    let dest = download::Pnpm.get_install_path(app_handle);

    download::ensure_extract(&tracker, name, buffer, dest.clone())
        .await
        .map_err(|e| format!("PNPM_EXTRACT_FAILED: {e}"))?;
    config::dependencies::record(app_handle, config::dependencies::DEP_PNPM, Some(dest));

    let _ = window.emit(
        PREINSTALL_LOG_EVENT,
        PreinstallLogPayload {
            line: "[pnpm] bundled pnpm ready".to_string(),
        },
    );
    Ok(true)
}

async fn user_pnpm_major_version_bounded(
    app_handle: &AppHandle,
    owner: ProcessOwner,
) -> Result<Option<u32>, String> {
    let Some(pnpm) = cli::find_user_pnpm(app_handle) else {
        return Ok(None);
    };
    pnpm_major_version_at_bounded(
        &pnpm,
        Some(&config::get_node_binary_path(app_handle)),
        Some(&cli::get_bin_dir(app_handle)),
        owner,
    )
    .await
}

pub(crate) async fn pnpm_major_version_at_bounded(
    pnpm: &Path,
    node: Option<&Path>,
    bin_dir: Option<&Path>,
    owner: ProcessOwner,
) -> Result<Option<u32>, String> {
    let deadline = tokio::time::Instant::now() + PNPM_PROBE_TIMEOUT;
    let mut command = pnpm_probe_command(pnpm, node, bin_dir);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let process_guard = tokio::time::timeout_at(deadline, acquire_process_lock())
        .await
        .map_err(|_| {
            "PNPM_PROBE_LOCK_TIMEOUT: timed out waiting for the process lock".to_string()
        })??;
    let child = match command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            log_probe_fallback(pnpm, "spawn", error);
            return Ok(None);
        }
    };
    let child = Arc::new(Mutex::new(child));
    let pid_guard = PidGuard::set_child(owner, &child);
    let pending = ProbeCleanupPending {
        child,
        owner,
        _process_guard: process_guard,
        _pid_guard: pid_guard,
    };
    let pnpm = pnpm.to_path_buf();
    tauri::async_runtime::spawn(async move {
        let child = pending.child.clone();
        let mut waiter =
            tauri::async_runtime::spawn_blocking(move || wait_for_probe_output(&child));
        let (reason, waiter) = match tokio::time::timeout_at(deadline, &mut waiter).await {
            Ok(Ok(Ok(output))) => {
                if let Some(reason) = super::super::process::process_cleanup_failure(pending.owner)
                {
                    (reason, None)
                } else {
                    pending.release();
                    return Ok(parse_pnpm_major_output(&pnpm, &output));
                }
            }
            Ok(joined) => {
                let reason = match joined {
                    Ok(Err(error)) => format!("PNPM_PROBE_OUTPUT_FAILED: {error}"),
                    Err(error) => format!("PNPM_PROBE_WAIT_TASK_FAILED: {error}"),
                    Ok(Ok(_)) => unreachable!(),
                };
                (reason, None)
            }
            Err(_) => (
                format!(
                    "PNPM_PROBE_TIMEOUT: pnpm version probe exceeded {} seconds",
                    PNPM_PROBE_TIMEOUT.as_secs()
                ),
                Some(waiter),
            ),
        };
        cleanup_probe(pending, waiter, reason.clone()).await?;
        Ok(probe_timeout_or_fallback(&pnpm, reason))
    })
    .await
    .map_err(|error| format!("PNPM_PROBE_TASK_FAILED: {error}"))?
}

struct ProbeCleanupPending {
    child: Arc<Mutex<std::process::Child>>,
    owner: ProcessOwner,
    _process_guard: tokio::sync::OwnedMutexGuard<()>,
    _pid_guard: PidGuard,
}

impl ProbeCleanupPending {
    fn release(self) {
        let Self {
            owner,
            _process_guard,
            _pid_guard,
            ..
        } = self;
        super::super::process::release_process_cleanup(owner, _pid_guard, _process_guard);
    }
}

fn wait_for_probe_output(
    child: &Arc<Mutex<std::process::Child>>,
) -> std::io::Result<std::process::Output> {
    let (stdout, stderr) = {
        let mut child = child.lock().unwrap_or_else(|error| error.into_inner());
        (child.stdout.take(), child.stderr.take())
    };
    std::thread::scope(|scope| {
        let stdout_reader = std::thread::Builder::new()
            .name("pnpm-probe-stdout".to_string())
            .spawn_scoped(scope, move || read_probe_pipe(stdout))?;
        let stderr_reader = std::thread::Builder::new()
            .name("pnpm-probe-stderr".to_string())
            .spawn_scoped(scope, move || read_probe_pipe(stderr))?;
        let stdout = stdout_reader
            .join()
            .map_err(|_| std::io::Error::other("pnpm version probe stdout reader panicked"))??;
        let stderr = stderr_reader
            .join()
            .map_err(|_| std::io::Error::other("pnpm version probe stderr reader panicked"))??;
        let status = loop {
            if let Some(status) = child
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .try_wait()?
            {
                break status;
            }
            std::thread::sleep(PNPM_PROBE_REAP_RETRY_INTERVAL);
        };
        Ok(std::process::Output {
            status,
            stdout,
            stderr,
        })
    })
}

use super::super::cancel::terminate_probe_tree;

fn terminate_probe(
    child: &Arc<Mutex<std::process::Child>>,
    terminate_tree: fn(u32) -> bool,
) -> std::io::Result<()> {
    let mut child = child.lock().unwrap_or_else(|error| error.into_inner());
    if child.try_wait()?.is_none() {
        let tree_killed = terminate_tree(child.id());
        let root_killed = child.kill();
        if !tree_killed {
            return Err(std::io::Error::other(
                "PNPM_PROBE_TREE_CLEANUP_FAILED: pnpm version probe process tree termination was not confirmed",
            ));
        }
        if let Err(error) = root_killed {
            if child.try_wait()?.is_none() {
                return Err(error);
            }
        }
    }
    Ok(())
}

async fn cleanup_probe(
    pending: ProbeCleanupPending,
    waiter: Option<tauri::async_runtime::JoinHandle<std::io::Result<std::process::Output>>>,
    reason: String,
) -> Result<(), String> {
    let mut cleanup = monitor_probe_cleanup(pending, waiter, reason, terminate_probe_tree);
    match tokio::time::timeout(PNPM_PROBE_CLEANUP_TIMEOUT, &mut cleanup).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(format!("PNPM_PROBE_CLEANUP_FAILED: {error}")),
        Err(_) => Err(
            "PNPM_PROBE_CLEANUP_TIMEOUT: pnpm version probe cleanup did not finish within 5 seconds"
                .to_string(),
        ),
    }
}

fn monitor_probe_cleanup(
    pending: ProbeCleanupPending,
    waiter: Option<tauri::async_runtime::JoinHandle<std::io::Result<std::process::Output>>>,
    reason: String,
    terminate_tree: fn(u32) -> bool,
) -> tauri::async_runtime::JoinHandle<()> {
    super::super::process::mark_process_cleanup_failed(pending.owner, reason);
    tauri::async_runtime::spawn(async move {
        let child = pending.child.clone();
        let killer =
            tauri::async_runtime::spawn_blocking(move || terminate_probe(&child, terminate_tree));
        let wait = async move {
            if let Some(waiter) = waiter {
                if let Err(error) = waiter.await {
                    log::error!("pnpm version probe cleanup wait task failed: {error}");
                }
            }
        };
        let (killed, ()) = tokio::join!(killer, wait);
        if !matches!(killed, Ok(Ok(()))) {
            let reason = format!(
                "PNPM_PROBE_TREE_CLEANUP_FAILED: pnpm version probe process tree termination was not confirmed: {killed:?}"
            );
            log::error!("{reason}");
            super::super::process::mark_process_cleanup_failed(pending.owner, reason);
            // ponytail: 未确认整树清理仅重启恢复；需自动恢复再跟踪可靠进程树身份。
            std::future::pending::<()>().await;
        }
        wait_for_probe_cleanup(&pending).await;
        if super::super::process::process_cleanup_failure(pending.owner)
            .is_some_and(|reason| reason.starts_with("PNPM_PROBE_TREE_CLEANUP_FAILED:"))
        {
            std::future::pending::<()>().await;
        }
        pending.release();
    })
}

async fn wait_for_probe_cleanup(pending: &ProbeCleanupPending) {
    wait_for_probe_cleanup_with(
        || {
            !matches!(
                pending
                    .child
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .try_wait(),
                Ok(Some(_))
            )
        },
        PNPM_PROBE_LIVENESS_INTERVAL,
    )
    .await;
}

async fn wait_for_probe_cleanup_with<F>(mut remains_active: F, interval: std::time::Duration)
where
    F: FnMut() -> bool,
{
    while remains_active() {
        tokio::time::sleep(interval).await;
    }
}

fn read_probe_pipe<R: Read>(pipe: Option<R>) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    if let Some(mut pipe) = pipe {
        pipe.read_to_end(&mut output)?;
    }
    Ok(output)
}

fn log_probe_fallback(pnpm: &Path, phase: &str, error: impl std::fmt::Display) {
    log::warn!(
        "pnpm version probe {phase} failed for {}: {error}; falling back to bundled pnpm",
        pnpm.display()
    );
}

/// 杀软扫描等）不该中断整次插件安装（issue #449）。只有清理未确认（进程可能
fn probe_timeout_or_fallback(pnpm: &Path, reason: String) -> Option<u32> {
    log_probe_fallback(pnpm, "timeout", reason);
    None
}

/// 关闭 stdin 避免 Corepack 交互挂起（issue #449），注入选定 Node 避免陈旧 PATH（issue #182）。
fn pnpm_probe_command(
    pnpm: &Path,
    node: Option<&Path>,
    bin_dir: Option<&Path>,
) -> std::process::Command {
    let mut command = std::process::Command::new(pnpm);
    crate::config::apply_node_env(&mut command);
    #[cfg(windows)]
    command.arg(WINDOWS_SPACE_PROBE_ARG);
    command.arg("--version");
    if let Some(path) = pnpm_probe_path(pnpm, node, bin_dir) {
        command.env("PATH", path);
    }
    if bin_dir.is_some() {
        command.env("DSH_PNPM", pnpm);
    }
    command.stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

fn parse_pnpm_major_output(pnpm: &Path, output: &std::process::Output) -> Option<u32> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::warn!(
            "pnpm version probe failed for {} with status {}: {}",
            pnpm.display(),
            output.status,
            stderr.trim()
        );
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.split('.').next()?.trim().parse::<u32>().ok()
}

fn pnpm_probe_path(pnpm: &Path, node: Option<&Path>, bin_dir: Option<&Path>) -> Option<OsString> {
    let mut paths = Vec::new();
    if let Some(bin_dir) = bin_dir {
        paths.push(bin_dir.to_path_buf());
    }
    if let Some(parent) = node.and_then(Path::parent) {
        paths.push(parent.to_path_buf());
    }
    if let Some(parent) = pnpm.parent() {
        paths.push(parent.to_path_buf());
    }
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(paths).ok()
}

pub(crate) fn profile_store_major(app_handle: &AppHandle) -> Option<u32> {
    profile_store_major_at(&profile_dir(app_handle))
}

pub(crate) fn profile_store_major_at(profile_dir: &Path) -> Option<u32> {
    parse_store_major_from_modules_yaml(&read_modules_yaml_at(profile_dir)?)
}

pub(crate) fn profile_store_base_dir(app_handle: &AppHandle) -> Option<String> {
    profile_store_base_dir_at(&profile_dir(app_handle))
}

pub(crate) fn profile_store_base_dir_at(profile_dir: &Path) -> Option<String> {
    let store_dir = parse_store_dir_from_modules_yaml(&read_modules_yaml_at(profile_dir)?)?;
    Some(strip_store_version(&store_dir))
}

fn strip_store_version(store_dir: &str) -> String {
    let trimmed = store_dir.trim_end_matches(['\\', '/']);
    let last = trimmed.rsplit(['\\', '/']).next().unwrap_or_default();
    let keep = trimmed.len() - last.len();
    let digits = last.strip_prefix('v').unwrap_or_default();
    if keep == 0 || digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return store_dir.to_string();
    }
    trimmed[..keep].trim_end_matches(['\\', '/']).to_string()
}

pub(crate) fn reset_incompatible_modules_metadata(app_handle: &AppHandle) -> bool {
    let modules_yaml = profile_dir(app_handle)
        .join("node_modules")
        .join(".modules.yaml");
    if !modules_yaml.is_file() {
        return false;
    }
    match std::fs::remove_file(&modules_yaml) {
        Ok(()) => {
            log::info!(
                "reset incompatible pnpm modules metadata: {}",
                modules_yaml.display()
            );
            true
        }
        Err(e) => {
            log::warn!("reset {} failed: {e}", modules_yaml.display());
            false
        }
    }
}

fn read_modules_yaml_at(profile_dir: &Path) -> Option<String> {
    std::fs::read_to_string(profile_dir.join("node_modules").join(".modules.yaml")).ok()
}

fn parse_store_dir_from_modules_yaml(content: &str) -> Option<String> {
    let value: serde_yaml::Value = serde_yaml::from_str(content).ok()?;
    let raw = value.get("storeDir")?.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    Some(raw.to_string())
}

fn parse_store_major_from_modules_yaml(content: &str) -> Option<u32> {
    let store_dir = parse_store_dir_from_modules_yaml(content)?;
    let major = store_dir.rsplit(['\\', '/']).next()?.strip_prefix('v')?;
    major.parse().ok()
}

pub(crate) fn bundled_pnpm_major(app_handle: &AppHandle) -> Option<u32> {
    bundled_pnpm_major_at(&config::get_pnpm_install_path(app_handle))
}

pub(crate) fn bundled_pnpm_major_at(pnpm_root: &Path) -> Option<u32> {
    let content = std::fs::read_to_string(pnpm_root.join("package.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    value
        .get("version")?
        .as_str()?
        .split('.')
        .next()?
        .parse()
        .ok()
}

pub(crate) async fn harness_prefer_bundled_pnpm(app_handle: &AppHandle) -> Result<bool, String> {
    harness_prefer_bundled_pnpm_from(
        &profile_dir(app_handle),
        &config::get_pnpm_install_path(app_handle),
        &config::get_pnpm_binary_path(app_handle),
        cli::find_user_pnpm(app_handle).as_deref(),
        &config::get_node_binary_path(app_handle),
        &cli::get_bin_dir(app_handle),
    )
    .await
}

pub(crate) async fn harness_prefer_bundled_pnpm_from(
    profile_dir: &Path,
    pnpm_root: &Path,
    pnpm_binary: &Path,
    user_pnpm: Option<&Path>,
    node: &Path,
    bin_dir: &Path,
) -> Result<bool, String> {
    if !pnpm_binary.exists() {
        return Ok(false);
    }
    let store_major = profile_store_major_at(profile_dir);
    if store_major.is_some_and(|store| bundled_pnpm_major_at(pnpm_root) != Some(store)) {
        return Ok(false);
    }
    let user_major = match user_pnpm {
        Some(pnpm) => {
            pnpm_major_version_at_bounded(pnpm, Some(node), Some(bin_dir), new_process_owner())
                .await?
        }
        None => None,
    };
    Ok(match store_major {
        Some(store) => user_major != Some(store),
        None => !matches!(user_major, Some(major) if major >= MIN_TRUSTED_PNPM_MAJOR),
    })
}

#[cfg(test)]
mod tests {
    use super::super::super::process::TEST_PROCESS_LOCK;
    use super::*;

    #[tokio::test]
    async fn startup_pnpm_paths_reread_metadata_and_skip_unneeded_probes() {
        let _test_guard = TEST_PROCESS_LOCK.lock().await;
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().join("profile");
        let modules = profile.join("node_modules").join(".modules.yaml");
        let pnpm_root = root.path().join("pnpm");
        let manifest = pnpm_root.join("package.json");
        let binary = pnpm_root.join("bin").join("pnpm.cjs");
        let user = root.path().join("must-not-be-probed");
        let node = root.path().join("node");
        let bin_dir = root.path().join("desktop-bin");
        std::fs::create_dir_all(modules.parent().unwrap()).unwrap();
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        let yaml = "storeDir: /store/v10\n";
        std::fs::write(&modules, yaml).unwrap();
        std::fs::write(&manifest, r#"{"version":"11.2.0"}"#).unwrap();
        assert_eq!(read_modules_yaml_at(&profile).as_deref(), Some(yaml));
        assert_eq!(profile_store_major_at(&profile), Some(10));
        assert_eq!(
            profile_store_base_dir_at(&profile).as_deref(),
            Some("/store")
        );
        assert_eq!(bundled_pnpm_major_at(&pnpm_root), Some(11));

        let process_guard = acquire_process_lock().await.unwrap();
        for present in [false, true] {
            if present {
                std::fs::write(&binary, "").unwrap();
            }
            let prefer = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                harness_prefer_bundled_pnpm_from(
                    &profile,
                    &pnpm_root,
                    &binary,
                    Some(&user),
                    &node,
                    &bin_dir,
                ),
            )
            .await
            .expect("missing or incompatible bundled pnpm must not acquire the probe lock")
            .unwrap();
            assert!(!prefer);
        }
        drop(process_guard);

        for store in [11, 10] {
            let json = format!(r#"{{"storeDir":"/store/v{store}"}}"#);
            std::fs::write(&modules, &json).unwrap();
            assert_eq!(profile_store_major_at(&profile), Some(store));
            assert_eq!(
                profile_store_base_dir_at(&profile).as_deref(),
                Some("/store")
            );
            assert_eq!(
                harness_prefer_bundled_pnpm_from(
                    &profile, &pnpm_root, &binary, None, &node, &bin_dir,
                )
                .await
                .unwrap(),
                store == 11
            );
            assert_eq!(std::fs::read_to_string(&modules).unwrap(), json);
        }
        std::fs::write(&manifest, r#"{"version":"10.9.0"}"#).unwrap();
        assert_eq!(bundled_pnpm_major_at(&pnpm_root), Some(10));
        assert!(
            harness_prefer_bundled_pnpm_from(&profile, &pnpm_root, &binary, None, &node, &bin_dir,)
                .await
                .unwrap()
        );
    }

    /// issue #449 回归：探测超时、但进程已被强杀并回收干净时，必须降级为
    #[test]
    fn pnpm_probe_timeout_after_recycled_cleanup_falls_back_to_bundled() {
        let pnpm = PathBuf::from("slow-pnpm");
        let fallback = probe_timeout_or_fallback(
            &pnpm,
            "PNPM_PROBE_TIMEOUT: pnpm version probe exceeded 10 seconds".to_string(),
        );

        assert!(
            fallback.is_none(),
            "recovered probe timeout must degrade to bundled pnpm, not fail the install"
        );
    }

    /// issue #449 根因：探测命令必须显式关闭 stdin。corepack 版本未缓存时会先
    #[cfg(unix)]
    #[test]
    fn pnpm_probe_command_closes_stdin_for_interactive_shims() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "dsh-pnpm-probe-stdin-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let pnpm = root.join("interactive-pnpm");
        std::fs::write(&pnpm, "#!/bin/sh\nread _\nprintf '11.0.0\\n'\n").unwrap();
        let mut permissions = std::fs::metadata(&pnpm).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&pnpm, permissions).unwrap();

        let mut child = pnpm_probe_command(&pnpm, None, None)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut buffer = String::new();
            let _ = std::io::BufReader::new(stdout).read_to_string(&mut buffer);
            let _ = sender.send(buffer);
        });

        let output = match receiver.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(output) => output,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("pnpm version probe must not inherit stdin (it blocks on input)");
            }
        };
        let status = child.wait().unwrap();
        let _ = reader.join();

        assert!(status.success());
        assert_eq!(output, "11.0.0\n");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn pnpm_probe_cleanup_monitor_releases_after_process_disappears() {
        let polls = std::sync::atomic::AtomicUsize::new(0);
        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            wait_for_probe_cleanup_with(
                || polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2,
                std::time::Duration::from_millis(1),
            ),
        )
        .await
        .expect("cleanup monitor should release after liveness turns false");
        assert_eq!(polls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn pnpm_probe_cleanup_monitor_stays_fail_closed_while_process_lives() {
        let pending = tokio::time::timeout(
            std::time::Duration::from_millis(5),
            wait_for_probe_cleanup_with(|| true, std::time::Duration::from_millis(1)),
        )
        .await;
        assert!(pending.is_err());
    }

    #[tokio::test]
    async fn exited_probe_releases_its_pid_and_process_guard() {
        let _test_guard = TEST_PROCESS_LOCK.lock().await;
        #[cfg(windows)]
        let mut child = {
            use std::os::windows::process::CommandExt;
            std::process::Command::new("cmd.exe")
                .creation_flags(0x08000000)
                .args(["/D", "/C", "exit 0"])
                .spawn()
                .unwrap()
        };
        #[cfg(not(windows))]
        let mut child = std::process::Command::new("sh")
            .args(["-c", "exit 0"])
            .spawn()
            .unwrap();
        let pid = child.id();
        assert!(child.wait().unwrap().success());
        let owner = super::super::super::process::new_process_owner();
        let pid_guard = PidGuard::set(owner, pid);
        let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
        let process_guard = lock.clone().lock_owned().await;
        assert!(lock.try_lock().is_err());
        cleanup_probe(
            ProbeCleanupPending {
                child: Arc::new(Mutex::new(child)),
                owner,
                _process_guard: process_guard,
                _pid_guard: pid_guard,
            },
            None,
            "PNPM_PROBE_WAIT_TASK_FAILED: test cleanup".to_string(),
        )
        .await
        .unwrap();
        assert_eq!(super::super::super::process::active_plugin_pid(owner), None);
        assert!(lock.try_lock().is_ok());
    }

    #[tokio::test]
    async fn live_probe_retains_its_pid_and_process_guard_until_exit() {
        let _test_guard = TEST_PROCESS_LOCK.lock().await;
        #[cfg(windows)]
        let mut command = {
            use std::os::windows::process::CommandExt;
            let mut command = std::process::Command::new("cmd.exe");
            command
                .creation_flags(0x08000000)
                .args(["/D", "/C", "set /p token="]);
            command
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut command = std::process::Command::new("sh");
            command.args(["-c", "read -r token"]);
            command
        };
        let mut child = command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        let owner = super::super::super::process::new_process_owner();
        let pid_guard = PidGuard::set(owner, pid);
        let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
        let process_guard = lock.clone().lock_owned().await;
        let mut input = child.stdin.take().unwrap();
        let pending = ProbeCleanupPending {
            child: Arc::new(Mutex::new(child)),
            owner,
            _process_guard: process_guard,
            _pid_guard: pid_guard,
        };
        let release = async {
            wait_for_probe_cleanup(&pending).await;
            pending.release();
        };
        tokio::pin!(release);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut release)
                .await
                .is_err()
        );
        assert_eq!(
            super::super::super::process::active_plugin_pid(owner),
            Some(pid)
        );
        assert!(lock.try_lock().is_err());
        use std::io::Write;
        input.write_all(b"done\n").unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), &mut release)
            .await
            .unwrap();
        assert_eq!(super::super::super::process::active_plugin_pid(owner), None);
        assert!(lock.try_lock().is_ok());
    }

    struct BrokenPipeReader;

    impl std::io::Read for BrokenPipeReader {
        fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "probe output pipe closed",
            ))
        }
    }

    #[test]
    fn pnpm_probe_broken_output_pipe_is_reported_as_fallback_failure() {
        let error = read_probe_pipe(Some(BrokenPipeReader)).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pnpm_major_version_at_probes_the_exact_selected_path() {
        let _test_guard = TEST_PROCESS_LOCK.lock().await;
        use std::os::unix::fs::PermissionsExt;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("dsh-pnpm-major-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let selected = root.join("selected-pnpm");
        let other = root.join("other-pnpm");
        std::fs::write(&selected, "#!/bin/sh\nprintf '11.2.0\\n'\n").unwrap();
        std::fs::write(&other, "#!/bin/sh\nprintf '10.9.0\\n'\n").unwrap();
        for path in [&selected, &other] {
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o700);
            std::fs::set_permissions(path, permissions).unwrap();
        }

        for (path, major) in [(&selected, 11), (&other, 10)] {
            assert_eq!(
                pnpm_major_version_at_bounded(path, None, None, new_process_owner())
                    .await
                    .unwrap(),
                Some(major)
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn pnpm_major_version_probe_injects_selected_node_for_cmd_shim() {
        let _test_guard = TEST_PROCESS_LOCK.lock().await;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("dsh pnpm cmd major {} {nonce}", std::process::id()));
        let pnpm_dir = root.join("pnpm & corepack");
        let node_dir = root.join("selected node");
        std::fs::create_dir_all(&pnpm_dir).unwrap();
        std::fs::create_dir_all(&node_dir).unwrap();
        let selected = pnpm_dir.join("pnpm.cmd");
        let node = node_dir.join("node.cmd");
        std::fs::write(&selected, "@echo off\r\nnode --version\r\n").unwrap();
        std::fs::write(pnpm_dir.join("node.cmd"), "@echo off\r\necho 9.0.0\r\n").unwrap();
        std::fs::write(&node, "@echo off\r\necho 11.24.0\r\n").unwrap();

        assert_eq!(
            pnpm_major_version_at_bounded(&selected, Some(&node), None, new_process_owner())
                .await
                .unwrap(),
            Some(11)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn pnpm_probe_rejects_shim_that_breaks_on_space_arguments() {
        let _test_guard = TEST_PROCESS_LOCK.lock().await;
        let root = tempfile::tempdir().unwrap();
        let broken = root.path().join("Author Software").join("pnpm.cmd");
        std::fs::create_dir_all(broken.parent().unwrap()).unwrap();
        std::fs::write(
            &broken,
            "@echo off\r\nset \"all=%*\"\r\nif not \"%all%\"==\"%all: =%\" (\r\n  echo 'C:\\Users\\JT\\AppData\\Local\\Author' is not recognized 1>&2\r\n  exit /b 1\r\n)\r\necho 12.4.2\r\n",
        )
        .unwrap();
        assert_eq!(
            pnpm_major_version_at_bounded(&broken, None, None, new_process_owner())
                .await
                .unwrap(),
            None
        );

        let healthy = root.path().join("Healthy Dir").join("pnpm.cmd");
        std::fs::create_dir_all(healthy.parent().unwrap()).unwrap();
        std::fs::write(&healthy, "@echo off\r\necho 12.4.2\r\n").unwrap();
        assert_eq!(
            pnpm_major_version_at_bounded(&healthy, None, None, new_process_owner())
                .await
                .unwrap(),
            Some(12)
        );
    }

    #[tokio::test]
    async fn pnpm_version_probe_bounds_slow_shims_and_releases_the_process_lock() {
        let _test_guard = TEST_PROCESS_LOCK.lock().await;
        let root = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let pnpm = {
            let pnpm = root.path().join("slow pnpm.cmd");
            std::fs::write(
                &pnpm,
                "@echo off\r\npowershell.exe -NoProfile -NonInteractive -Command \"Start-Sleep -Seconds 20\"\r\necho 11.2.0\r\n",
            )
            .unwrap();
            pnpm
        };
        #[cfg(unix)]
        let pnpm = {
            use std::os::unix::fs::PermissionsExt;
            let pnpm = root.path().join("slow-pnpm");
            std::fs::write(&pnpm, "#!/bin/sh\nsleep 20\nprintf '11.2.0\\n'\n").unwrap();
            std::fs::set_permissions(&pnpm, std::fs::Permissions::from_mode(0o700)).unwrap();
            pnpm
        };
        let started = std::time::Instant::now();
        assert_eq!(
            pnpm_major_version_at_bounded(&pnpm, None, None, new_process_owner())
                .await
                .unwrap(),
            None
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(18));
        let process_guard =
            tokio::time::timeout(std::time::Duration::from_secs(1), acquire_process_lock())
                .await
                .expect("finished probe must release the process lock")
                .unwrap();
        drop(process_guard);
    }

    #[tokio::test]
    async fn cancelled_probe_keeps_its_pid_and_lock_until_the_child_exits() {
        let _test_guard = TEST_PROCESS_LOCK.lock().await;
        let root = tempfile::tempdir().unwrap();
        let release = root.path().join("release");
        #[cfg(windows)]
        let pnpm = {
            let pnpm = root.path().join("held pnpm.cmd");
            let path = release.to_string_lossy().replace('\'', "''");
            std::fs::write(
                &pnpm,
                format!("@echo off\r\npowershell.exe -NoProfile -NonInteractive -Command \"while (-not (Test-Path -LiteralPath '{path}')) {{ Start-Sleep -Milliseconds 25 }}\"\r\necho 11.2.0\r\n"),
            )
            .unwrap();
            pnpm
        };
        #[cfg(unix)]
        let pnpm = {
            use std::os::unix::fs::PermissionsExt;
            let pnpm = root.path().join("held-pnpm");
            let path = release.to_string_lossy().replace('\'', "'\\''");
            std::fs::write(
                &pnpm,
                format!(
                    "#!/bin/sh\nwhile [ ! -f '{path}' ]; do sleep 0.025; done\nprintf '11.2.0\\n'\n"
                ),
            )
            .unwrap();
            std::fs::set_permissions(&pnpm, std::fs::Permissions::from_mode(0o700)).unwrap();
            pnpm
        };
        let owner = new_process_owner();
        let caller =
            tokio::spawn(
                async move { pnpm_major_version_at_bounded(&pnpm, None, None, owner).await },
            );
        let pid = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(pid) = super::super::super::process::active_plugin_pid(owner) {
                    break pid;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        let retained_pid = super::super::super::process::active_plugin_pid(owner);
        let held_lock =
            tokio::time::timeout(std::time::Duration::from_millis(50), acquire_process_lock())
                .await;
        std::fs::write(release, "done").unwrap();
        let released =
            tokio::time::timeout(std::time::Duration::from_secs(5), acquire_process_lock())
                .await
                .unwrap()
                .unwrap();
        assert_eq!(retained_pid, Some(pid));
        assert!(
            held_lock.is_err(),
            "cancelled caller must not release a live probe lock"
        );
        assert_eq!(super::super::super::process::active_plugin_pid(owner), None);
        drop(released);
    }

    #[tokio::test]
    async fn cleanup_timeout_fails_closed_until_the_waiter_finishes() {
        let _test_guard = TEST_PROCESS_LOCK.lock().await;
        #[cfg(windows)]
        let mut child = {
            use std::os::windows::process::CommandExt;
            std::process::Command::new("cmd.exe")
                .creation_flags(0x08000000)
                .args(["/D", "/C", "exit 0"])
                .spawn()
                .unwrap()
        };
        #[cfg(unix)]
        let mut child = std::process::Command::new("sh")
            .args(["-c", "exit 0"])
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        let owner = new_process_owner();
        let process_guard = acquire_process_lock().await.unwrap();
        let pending = ProbeCleanupPending {
            child: Arc::new(Mutex::new(child)),
            owner,
            _process_guard: process_guard,
            _pid_guard: PidGuard::set(owner, pid),
        };
        let (release, held) = tokio::sync::oneshot::channel::<()>();
        let waiter = tauri::async_runtime::spawn(async move {
            held.await.unwrap();
            Err(std::io::Error::other("probe output pipe closed"))
        });
        let started = std::time::Instant::now();
        let result = cleanup_probe(
            pending,
            Some(waiter),
            "PNPM_PROBE_WAIT_TASK_FAILED: held waiter".to_string(),
        )
        .await;
        let retained_pid = super::super::super::process::active_plugin_pid(owner);
        let blocked =
            tokio::time::timeout(std::time::Duration::from_millis(50), acquire_process_lock())
                .await;
        release.send(()).unwrap();
        let released = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while super::super::super::process::active_plugin_pid(owner).is_some() {
                tokio::task::yield_now().await;
            }
            acquire_process_lock().await
        })
        .await
        .unwrap()
        .unwrap();
        assert!(
            result
                .unwrap_err()
                .starts_with("PNPM_PROBE_CLEANUP_TIMEOUT:")
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(7));
        assert_eq!(retained_pid, Some(pid));
        assert!(
            blocked
                .unwrap()
                .unwrap_err()
                .starts_with("PNPM_PROBE_WAIT_TASK_FAILED:")
        );
        assert_eq!(super::super::super::process::active_plugin_pid(owner), None);
        drop(released);
    }

    #[tokio::test]
    async fn failed_tree_termination_retains_guards_even_after_root_exit() {
        let _test_guard = TEST_PROCESS_LOCK.lock().await;
        #[cfg(windows)]
        let mut command = {
            use std::os::windows::process::CommandExt;
            let mut command = std::process::Command::new("cmd.exe");
            command
                .creation_flags(0x08000000)
                .args(["/D", "/C", "set /p token="]);
            command
        };
        #[cfg(unix)]
        let mut command = {
            use std::os::unix::process::CommandExt;
            let mut command = std::process::Command::new("sh");
            command.args(["-c", "read -r token"]).process_group(0);
            command
        };
        let child = Arc::new(Mutex::new(
            command
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        ));
        let pid = child.lock().unwrap().id();
        let owner = new_process_owner();
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let pending = ProbeCleanupPending {
            child: child.clone(),
            owner,
            _process_guard: lock.clone().lock_owned().await,
            _pid_guard: PidGuard::set(owner, pid),
        };
        let waiting_child = child.clone();
        let waiter =
            tauri::async_runtime::spawn_blocking(move || wait_for_probe_output(&waiting_child));
        let mut monitor = monitor_probe_cleanup(
            pending,
            Some(waiter),
            "PNPM_PROBE_TIMEOUT: injected tree termination failure".to_string(),
            |_| false,
        );
        let exited = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while child.lock().unwrap().try_wait().unwrap().is_none() {
                tokio::time::sleep(PNPM_PROBE_REAP_RETRY_INTERVAL).await;
            }
        })
        .await;
        let failure = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Err(reason) = acquire_process_lock().await {
                    if reason.starts_with("PNPM_PROBE_TREE_CLEANUP_FAILED:") {
                        break reason;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        let retained_pid = super::super::super::process::active_plugin_pid(owner);
        let lock_held = lock.try_lock().is_err();
        let retained = tokio::time::timeout(std::time::Duration::from_millis(50), &mut monitor)
            .await
            .is_err();
        if retained {
            monitor.abort();
            let _ = monitor.await;
        }
        super::super::super::process::clear_process_cleanup_failed(owner);
        if exited.is_err() {
            let mut child = child.lock().unwrap();
            let _ = child.kill();
            let _ = child.wait();
        }
        assert!(
            exited.is_ok(),
            "root fallback must kill the real probe child"
        );
        assert!(
            failure.is_ok(),
            "root exit must not clear unconfirmed tree cleanup"
        );
        assert!(retained, "tree failure must retain the cleanup monitor");
        assert!(lock_held, "tree failure must retain the process guard");
        assert_eq!(retained_pid, Some(pid));
        assert_eq!(super::super::super::process::active_plugin_pid(owner), None);
        assert!(lock.try_lock().is_ok());
    }

    #[test]
    fn store_major_parsed_from_modules_yaml() {
        let content = "\
lockfileVersion: '9.0'
settings:
  autoInstallPeers: true
  excludeLinksFromLockfile: false
dependencies:
  '@deepseek-ai/dsh-base': 0.0.4
  '@deepseek-ai/dsh-web-app': 0.0.4
storeDir: C:\\Users\\test\\AppData\\Local\\pnpm\\store\\v10
virtualStoreDir: node_modules/.pnpm
";
        assert_eq!(parse_store_major_from_modules_yaml(content), Some(10));
    }

    #[test]
    fn store_major_supports_unix_and_quoted_paths() {
        assert_eq!(
            parse_store_major_from_modules_yaml(
                "storeDir: /home/test/.local/share/pnpm/store/v11\n"
            ),
            Some(11)
        );
        assert_eq!(
            parse_store_major_from_modules_yaml("storeDir: \"C:\\\\pnpm store\\\\v3\"\n"),
            Some(3)
        );
    }

    #[test]
    fn store_major_missing_when_no_store_dir() {
        assert_eq!(
            parse_store_major_from_modules_yaml("lockfileVersion: '9.0'\n"),
            None
        );
        assert_eq!(parse_store_major_from_modules_yaml(""), None);
        assert_eq!(
            parse_store_major_from_modules_yaml("storeDir: C:\\Users\\x\\pnpm\\store\n"),
            None
        );
    }

    #[test]
    fn store_dir_parsed_for_handoff_to_pnpm() {
        let content = "\
hoistPattern:
  - '*'
storeDir: C:\\Users\\test\\AppData\\Local\\pnpm\\store\\v10
virtualStoreDir: node_modules/.pnpm
";
        assert_eq!(
            parse_store_dir_from_modules_yaml(content).as_deref(),
            Some("C:\\Users\\test\\AppData\\Local\\pnpm\\store\\v10")
        );
        assert_eq!(
            parse_store_dir_from_modules_yaml("storeDir: /home/test/.local/share/pnpm/store/v11\n")
                .as_deref(),
            Some("/home/test/.local/share/pnpm/store/v11")
        );
        assert_eq!(
            parse_store_dir_from_modules_yaml("storeDir: \"C:\\\\pnpm store\\\\v3\"\n").as_deref(),
            Some("C:\\pnpm store\\v3")
        );
        assert_eq!(
            parse_store_dir_from_modules_yaml("storeDir: 'D:\\.pnpm-store\\v10'\n").as_deref(),
            Some("D:\\.pnpm-store\\v10")
        );
        assert_eq!(parse_store_dir_from_modules_yaml("storeDir:\n"), None);
        assert_eq!(
            parse_store_dir_from_modules_yaml("lockfileVersion: '9.0'\n"),
            None
        );
    }

    #[test]
    fn store_dir_parsed_from_json_modules_yaml() {
        let content = r#"{
  "hoistPattern": [
    "*"
  ],
  "layoutVersion": 5,
  "storeDir": "/Users/ruan/Library/pnpm/store/v11",
  "virtualStoreDir": "node_modules/.pnpm"
}"#;
        assert_eq!(
            parse_store_dir_from_modules_yaml(content).as_deref(),
            Some("/Users/ruan/Library/pnpm/store/v11")
        );
        assert_eq!(parse_store_major_from_modules_yaml(content), Some(11));

        let windows = r#"{"storeDir": "C:\\Users\\test\\AppData\\Local\\pnpm\\store\\v10"}"#;
        assert_eq!(
            parse_store_dir_from_modules_yaml(windows).as_deref(),
            Some(r"C:\Users\test\AppData\Local\pnpm\store\v10")
        );
        assert_eq!(parse_store_major_from_modules_yaml(windows), Some(10));

        assert_eq!(parse_store_dir_from_modules_yaml("{}"), None);
        assert_eq!(
            parse_store_dir_from_modules_yaml(r#"{"storeDir": 5}"#),
            None
        );
    }

    #[test]
    fn store_base_dir_strips_trailing_pnpm_version_segment() {
        assert_eq!(
            strip_store_version("C:\\Users\\test\\AppData\\Local\\pnpm\\store\\v10"),
            "C:\\Users\\test\\AppData\\Local\\pnpm\\store"
        );
        assert_eq!(
            strip_store_version("/home/test/.local/share/pnpm/store/v11"),
            "/home/test/.local/share/pnpm/store"
        );
        assert_eq!(
            strip_store_version("/Users/gao/Library/pnpm/store/v10/"),
            "/Users/gao/Library/pnpm/store"
        );
        assert_eq!(
            strip_store_version("D:\\.pnpm-store\\v3"),
            "D:\\.pnpm-store"
        );
    }

    #[test]
    fn store_base_dir_keeps_paths_without_version_segment() {
        assert_eq!(
            strip_store_version("C:\\Users\\x\\pnpm\\store"),
            "C:\\Users\\x\\pnpm\\store"
        );
        assert_eq!(
            strip_store_version("/mnt/pnpm-store-v10"),
            "/mnt/pnpm-store-v10"
        );
        assert_eq!(strip_store_version("/mnt/v-store"), "/mnt/v-store");
        assert_eq!(strip_store_version("v10"), "v10");
        assert_eq!(strip_store_version(""), "");
    }

    #[test]
    fn probe_path_puts_desktop_shim_dir_first() {
        let path = pnpm_probe_path(
            Path::new("/user/bin/pnpm"),
            Some(Path::new("/opt/node/bin/node")),
            Some(Path::new("/desktop/bin")),
        )
        .unwrap();
        let entries: Vec<String> = std::env::split_paths(&path)
            .map(|entry| entry.to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries[0], "/desktop/bin");
        assert_eq!(entries[1], "/opt/node/bin");
        assert_eq!(entries[2], "/user/bin");
    }

    #[test]
    fn probe_path_without_bin_dir_keeps_node_first() {
        let path = pnpm_probe_path(
            Path::new("/user/bin/pnpm"),
            Some(Path::new("/opt/node/bin/node")),
            None,
        )
        .unwrap();
        let entries: Vec<String> = std::env::split_paths(&path)
            .map(|entry| entry.to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries[0], "/opt/node/bin");
        assert_eq!(entries[1], "/user/bin");
    }
}
