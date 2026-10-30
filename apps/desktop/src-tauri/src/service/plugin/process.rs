use serde::Serialize;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

use crate::utils::decode_process_line;

use crate::WebviewWindow;
#[cfg(windows)]
use crate::service::workflow;
#[cfg(not(windows))]
use std::process::{Command, Stdio};

pub(crate) const PREINSTALL_LOG_EVENT: &str = "preinstall-log";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreinstallLogPayload {
    pub line: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ProcessOwner(u64);

#[derive(Clone)]
pub(crate) struct ActivePluginProcess {
    pub(crate) pid: u32,
    pub(crate) child: Option<Arc<Mutex<std::process::Child>>>,
}

static NEXT_PROCESS_OWNER: AtomicU64 = AtomicU64::new(1);
static ACTIVE_PLUGIN_PIDS: OnceLock<Mutex<HashMap<ProcessOwner, ActivePluginProcess>>> =
    OnceLock::new();
static PLUGIN_PROCESS_LOCK: OnceLock<Arc<tokio::sync::Mutex<()>>> = OnceLock::new();
static PLUGIN_OPERATION_LOCK: OnceLock<Arc<tokio::sync::Mutex<()>>> = OnceLock::new();
#[cfg(test)]
pub(crate) static TEST_PROCESS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static PLUGIN_CLEANUP_FAILURE: OnceLock<
    tokio::sync::watch::Sender<Option<(ProcessOwner, String)>>,
> = OnceLock::new();

fn active_pid_lock() -> &'static Mutex<HashMap<ProcessOwner, ActivePluginProcess>> {
    ACTIVE_PLUGIN_PIDS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cleanup_failure_sender() -> &'static tokio::sync::watch::Sender<Option<(ProcessOwner, String)>> {
    PLUGIN_CLEANUP_FAILURE.get_or_init(|| tokio::sync::watch::channel(None).0)
}

pub(crate) fn new_process_owner() -> ProcessOwner {
    ProcessOwner(NEXT_PROCESS_OWNER.fetch_add(1, Ordering::Relaxed))
}

pub(crate) fn active_plugin_pid(owner: ProcessOwner) -> Option<u32> {
    active_plugin_process(owner).map(|process| process.pid)
}

pub(crate) fn active_plugin_process(owner: ProcessOwner) -> Option<ActivePluginProcess> {
    active_pid_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&owner)
        .cloned()
}

pub(crate) fn active_plugin_processes() -> Vec<(ProcessOwner, ActivePluginProcess)> {
    active_pid_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|(owner, process)| (*owner, process.clone()))
        .collect()
}

#[cfg(windows)]
pub(crate) fn plugin_process_has_exited(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, WaitForSingleObject,
    };
    const SYNCHRONIZE_ACCESS: u32 = 0x0010_0000;
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE_ACCESS,
            0,
            pid,
        )
    };
    if handle.is_null() {
        return std::io::Error::last_os_error().raw_os_error() == Some(87);
    }
    let wait = unsafe { WaitForSingleObject(handle, 0) };
    unsafe { CloseHandle(handle) };
    wait == 0
}

#[cfg(not(windows))]
pub(crate) fn plugin_process_has_exited(pid: u32) -> bool {
    let mut status = 0;
    let waited = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
    if waited == pid as libc::pid_t {
        return true;
    }
    if waited == 0 {
        return false;
    }
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

pub(crate) fn plugin_writer_lock_path(profile_dir: &Path) -> std::path::PathBuf {
    profile_dir.join("package.json.lock")
}

fn orphan_lock_should_clear(recorded: &str, has_exited: impl Fn(u32) -> bool) -> bool {
    recorded
        .trim()
        .parse::<u32>()
        .is_ok_and(|pid| pid != 0 && has_exited(pid))
}

pub(crate) fn clear_orphan_plugin_writer_lock(profile_dir: &Path) -> bool {
    let path = plugin_writer_lock_path(profile_dir);
    let Ok(recorded) = std::fs::read_to_string(&path) else {
        return false;
    };
    if !orphan_lock_should_clear(&recorded, plugin_process_has_exited) {
        return false;
    }
    if std::fs::remove_file(&path).is_err() {
        return false;
    }
    log::warn!(
        "removed orphan plugin writer lock ({}), holder {} already exited",
        path.display(),
        recorded.trim()
    );
    true
}

pub(crate) struct PluginWriterLock {
    path: std::path::PathBuf,
}

impl Drop for PluginWriterLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) fn acquire_plugin_writer_lock(profile_dir: &Path) -> Result<PluginWriterLock, String> {
    let path = plugin_writer_lock_path(profile_dir);
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut delay = Duration::from_millis(20);
    loop {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write;
                let guard = PluginWriterLock { path };
                let written = file
                    .write_all(format!("{}\n", std::process::id()).as_bytes())
                    .and_then(|()| file.sync_all());
                drop(file);
                written.map_err(|e| format!("PLUGIN_WRITER_LOCK_WRITE_FAILED: {e}"))?;
                return Ok(guard);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                take_over_exited_plugin_writer_lock(&path);
            }
            Err(error) => return Err(format!("PLUGIN_WRITER_LOCK_FAILED: {error}")),
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "PLUGIN_WRITER_LOCK_TIMEOUT: timed out waiting for {}",
                path.display()
            ));
        }
        std::thread::sleep(delay);
        delay = (delay * 2).min(Duration::from_millis(200));
    }
}

fn take_over_exited_plugin_writer_lock(path: &Path) -> bool {
    let Ok(record) = std::fs::read_to_string(path) else {
        return false;
    };
    if !orphan_lock_should_clear(&record, plugin_process_has_exited) {
        return false;
    }
    let Some(claim) = claim_takeover(path, &record) else {
        return false;
    };
    let still_stale = std::fs::read_to_string(path).ok().as_deref() == Some(record.as_str())
        && orphan_lock_should_clear(&record, plugin_process_has_exited);
    let removed = still_stale && std::fs::remove_file(path).is_ok();
    let _ = std::fs::remove_file(&claim);
    removed
}

fn claim_takeover(path: &Path, record: &str) -> Option<std::path::PathBuf> {
    use std::io::Write;

    let claim = path.with_file_name(format!(
        "package.json.lock.takeover-{:016x}",
        lock_record_hash(record)
    ));
    for attempt in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&claim)
        {
            Ok(mut file) => {
                let _ = file
                    .write_all(format!("{}\n", std::process::id()).as_bytes())
                    .and_then(|()| file.sync_all());
                return Some(claim);
            }
            Err(error) if attempt == 0 && error.kind() == std::io::ErrorKind::AlreadyExists => {
                let Ok(owner) = std::fs::read_to_string(&claim) else {
                    return None;
                };
                if !orphan_lock_should_clear(&owner, plugin_process_has_exited) {
                    return None;
                }
                let _ = std::fs::remove_file(&claim);
            }
            Err(_) => return None,
        }
    }
    None
}

fn lock_record_hash(record: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    record.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn process_cleanup_failure(owner: ProcessOwner) -> Option<String> {
    cleanup_failure_sender()
        .borrow()
        .as_ref()
        .filter(|(active, _)| *active == owner)
        .map(|(_, reason)| reason.clone())
}

pub(crate) fn is_process_cleanup_failure(error: &str) -> bool {
    [
        "PNPM_PROBE_TREE_CLEANUP_FAILED:",
        "PNPM_PROBE_CLEANUP_",
        "PLUGIN_PROCESS_",
        "PREINSTALL_WAIT:",
        "INTERNAL_PLUGIN_PROCESS_REAP_TIMEOUT:",
        "INTERNAL_PLUGIN_ENSURE_DROPPED:",
    ]
    .iter()
    .any(|prefix| error.starts_with(prefix))
}

pub(crate) fn mark_process_cleanup_failed(owner: ProcessOwner, reason: String) {
    cleanup_failure_sender().send_if_modified(|failure| {
        if failure.as_ref().is_some_and(|(active, current)| {
            *active == owner && current.starts_with("PNPM_PROBE_TREE_CLEANUP_FAILED:")
        }) {
            return false;
        }
        *failure = Some((owner, reason));
        true
    });
}

pub(crate) fn clear_process_cleanup_failed(owner: ProcessOwner) {
    cleanup_failure_sender().send_if_modified(|failure| {
        if failure.as_ref().is_some_and(|(active, _)| *active == owner) {
            *failure = None;
            true
        } else {
            false
        }
    });
}

pub(crate) fn release_process_cleanup(
    owner: ProcessOwner,
    pid_guard: PidGuard,
    process_guard: tokio::sync::OwnedMutexGuard<()>,
) {
    clear_process_cleanup_failed(owner);
    drop(pid_guard);
    drop(process_guard);
}

pub(crate) async fn acquire_process_lock() -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
    let lock = PLUGIN_PROCESS_LOCK
        .get_or_init(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone();
    let mut cleanup_failure = cleanup_failure_sender().subscribe();
    loop {
        if let Some((_, reason)) = cleanup_failure.borrow().as_ref() {
            return Err(reason.clone());
        }
        tokio::select! {
            guard = lock.clone().lock_owned() => {
                if let Some((_, reason)) = cleanup_failure.borrow().as_ref() {
                    drop(guard);
                    return Err(reason.clone());
                }
                return Ok(guard);
            }
            changed = cleanup_failure.changed() => {
                if changed.is_err() {
                    return Err(
                        "PLUGIN_PROCESS_COORDINATOR_DROPPED: cleanup coordinator ended unexpectedly"
                            .to_string(),
                    );
                }
            }
        }
    }
}

pub(crate) async fn acquire_operation_lock() -> tokio::sync::OwnedMutexGuard<()> {
    PLUGIN_OPERATION_LOCK
        .get_or_init(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
        .lock_owned()
        .await
}

/// Command for a plugin child process.
///
/// The Electron runtime's own binary must run as Node, so it comes from
/// `client.node()` — the runtime owns both the path and `ELECTRON_RUN_AS_NODE`.
/// A user-supplied launcher (a real `pnpm`, a standalone `node`) is spawned as
/// itself and must not receive that switch.
pub(crate) fn plugin_command(
    app_handle: &tauri::AppHandle<tauri_runtime_electron::ElectronRuntime>,
    program: &Path,
) -> tokio::process::Command {
    use tauri::Manager;
    if program == crate::config::get_node_binary_path(app_handle) {
        return app_handle
            .state::<tauri_runtime_electron::ElectronClient>()
            .inner()
            .node();
    }
    tokio::process::Command::new(program)
}

pub(crate) struct PidGuard {
    owner: ProcessOwner,
    pid: u32,
}

impl PidGuard {
    pub(crate) fn set(owner: ProcessOwner, pid: u32) -> Self {
        active_pid_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(owner, ActivePluginProcess { pid, child: None });
        Self { owner, pid }
    }

    pub(crate) fn set_child(owner: ProcessOwner, child: &Arc<Mutex<std::process::Child>>) -> Self {
        let pid = child.lock().unwrap_or_else(|error| error.into_inner()).id();
        active_pid_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(
                owner,
                ActivePluginProcess {
                    pid,
                    child: Some(child.clone()),
                },
            );
        Self { owner, pid }
    }
}

impl Drop for PidGuard {
    fn drop(&mut self) {
        let mut active = active_pid_lock().lock().unwrap_or_else(|e| e.into_inner());
        if active
            .get(&self.owner)
            .is_some_and(|process| process.pid == self.pid)
        {
            active.remove(&self.owner);
        }
    }
}

#[cfg(windows)]
struct WaitableHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
unsafe impl Send for WaitableHandle {}

pub(crate) async fn run_plugin_process(
    node: &Path,
    args: &[OsString],
    cwd: &Path,
    envs: &HashMap<String, String>,
    window: &WebviewWindow,
    owner: ProcessOwner,
) -> Result<(i32, String), String> {
    let profile = crate::service::profile::active_profile(window.app_handle());
    let process_guard = acquire_process_lock().await?;
    let captured = Arc::new(Mutex::new(String::new()));

    let mut child = plugin_command(window.app_handle(), node)
        .args(args)
        .current_dir(cwd)
        .envs(envs)
        .spawn()
        .map_err(|e| format!("PREINSTALL_SPAWN: {e}"))?;
    let pid = child.id().unwrap_or_default();
    let pid_guard = PidGuard::set(owner, pid);
    log::info!("dsh plugin install started, pid {pid}");

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let emitter_window = window.clone();
    let emitter_captured = captured.clone();
    let emit = tokio::spawn(crate::service::workflow::instances::scope(
        profile,
        async move {
            let _process_guard = process_guard;
            let _pid_guard = pid_guard;
            let readers: Vec<_> = [
                stdout.map(|stream| {
                    spawn_line_emitter(stream, emitter_window.clone(), emitter_captured.clone())
                }),
                stderr.map(|stream| spawn_line_emitter(stream, emitter_window, emitter_captured)),
            ]
            .into_iter()
            .flatten()
            .collect();
            let exit_code = child
                .wait()
                .await
                .map(|status| status.code().unwrap_or(1))
                .unwrap_or(1);
            for reader in readers {
                let _ = reader.await;
            }
            exit_code
        },
    ))
    .await
    .map_err(|e| format!("PREINSTALL_WAIT: {e}"))?;

    tokio::time::sleep(Duration::from_millis(500)).await;
    Ok((emit, drain_captured(captured)))
}

pub(super) fn drain_captured(captured: Arc<Mutex<String>>) -> String {
    captured
        .lock()
        .map(|mut buf| std::mem::take(&mut *buf))
        .unwrap_or_default()
}

fn spawn_line_emitter(
    reader: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    window: WebviewWindow,
    captured: Arc<Mutex<String>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = tokio::io::BufReader::new(reader);
        let mut line = Vec::new();
        loop {
            line.clear();
            match tokio::io::AsyncBufReadExt::read_until(&mut buf, b'\n', &mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    let decoded = decode_process_line(&line);
                    let trimmed = decoded
                        .strip_suffix("\r\n")
                        .or_else(|| decoded.strip_suffix('\n'))
                        .unwrap_or(&decoded)
                        .to_string();
                    let _ = window.emit(
                        PREINSTALL_LOG_EVENT,
                        PreinstallLogPayload {
                            line: trimmed.clone(),
                        },
                    );
                    if let Ok(mut acc) = captured.lock() {
                        acc.push_str(&trimmed);
                        acc.push('\n');
                    }
                }
                Err(e) => {
                    log::error!("Failed to read plugin process output: {}", e);
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn blocking_plugin_task_retains_captured_profile() {
        use crate::service::workflow::instances;
        instances::scope("plugin-window-profile".into(), async {
            let profile = instances::current_profile().unwrap();
            let captured = tauri::async_runtime::spawn_blocking(move || {
                instances::scope_sync(profile, instances::current_profile)
            })
            .await
            .unwrap();
            assert_eq!(captured.as_deref(), Some("plugin-window-profile"));
        })
        .await;
        assert_eq!(instances::current_profile(), None);
    }

    #[test]
    fn drain_captured_preserves_exact_output_and_clears_shared_buffer() {
        let captured = Arc::new(Mutex::new(
            "stdout  \r\n\tstderr\0中文\npartial\r".to_string(),
        ));

        assert_eq!(
            drain_captured(captured.clone()),
            "stdout  \r\n\tstderr\0中文\npartial\r"
        );
        assert_eq!(*captured.lock().unwrap(), "");
        assert_eq!(drain_captured(captured.clone()), "");

        captured.lock().unwrap().push_str("late stderr");
        assert_eq!(drain_captured(captured.clone()), "late stderr");
        assert_eq!(*captured.lock().unwrap(), "");
    }

    #[test]
    fn drain_captured_returns_empty_without_recovering_poisoned_buffer() {
        let captured = Arc::new(Mutex::new("retained output".to_string()));
        let writer = captured.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = writer.lock().unwrap();
                panic!("poison isolated capture buffer");
            })
            .join()
            .is_err()
        );

        assert_eq!(drain_captured(captured.clone()), "");
        assert!(captured.is_poisoned());
        assert_eq!(
            *captured.lock().unwrap_err().into_inner(),
            "retained output"
        );
    }

    #[tokio::test]
    async fn stale_guard_cannot_clear_a_new_process_for_the_same_owner() {
        let _test_guard = TEST_PROCESS_LOCK.lock().await;
        let owner = new_process_owner();
        let stale = PidGuard::set(owner, 100);
        let current = PidGuard::set(owner, 200);

        drop(stale);
        assert_eq!(active_plugin_pid(owner), Some(200));

        drop(current);
        assert_eq!(active_plugin_pid(owner), None);
    }

    #[tokio::test]
    async fn cleanup_failure_clears_before_lock_handoff_and_preserves_other_owner() {
        let _test_guard = TEST_PROCESS_LOCK.lock().await;
        let guard = acquire_process_lock().await.unwrap();
        let owner = new_process_owner();
        let pid_guard = PidGuard::set(owner, 100);
        let reason = "PLUGIN_PROCESS_CLEANUP_FAILED: test process is still active".to_string();
        mark_process_cleanup_failed(owner, reason.clone());

        let waiter = tokio::spawn(acquire_process_lock());
        tokio::task::yield_now().await;
        assert!(
            waiter.is_finished(),
            "active cleanup failure should wake a queued waiter"
        );
        assert_eq!(waiter.await.unwrap().unwrap_err(), reason);

        clear_process_cleanup_failed(owner);
        let handoff = tokio::spawn(acquire_process_lock());
        tokio::task::yield_now().await;
        assert!(
            !handoff.is_finished(),
            "waiter should queue after failure clears while guard remains held"
        );
        drop(pid_guard);
        drop(guard);
        assert!(handoff.await.unwrap().is_ok());
        assert_eq!(active_plugin_pid(owner), None);

        let guard = acquire_process_lock().await.unwrap();
        let other_owner = new_process_owner();
        let stale_pid_guard = PidGuard::set(owner, 101);
        let other_reason =
            "PLUGIN_PROCESS_CLEANUP_FAILED: another owner is still active".to_string();
        mark_process_cleanup_failed(other_owner, other_reason.clone());
        release_process_cleanup(owner, stale_pid_guard, guard);

        assert_eq!(
            acquire_process_lock().await.unwrap_err(),
            other_reason,
            "clearing a stale owner must not remove another owner's failure"
        );
        clear_process_cleanup_failed(other_owner);

        let tree_reason = "PNPM_PROBE_TREE_CLEANUP_FAILED: tree exit is unconfirmed".to_string();
        mark_process_cleanup_failed(owner, tree_reason.clone());
        mark_process_cleanup_failed(owner, "PNPM_PROBE_TIMEOUT: cleanup started".to_string());
        let retained = cleanup_failure_sender().borrow().clone();
        assert_eq!(process_cleanup_failure(owner), Some(tree_reason.clone()));
        clear_process_cleanup_failed(owner);
        assert_eq!(retained, Some((owner, tree_reason)));
    }

    #[test]
    fn cleanup_classifier_keeps_ordinary_install_failures_best_effort() {
        for blocked in [
            "PNPM_PROBE_TREE_CLEANUP_FAILED: tree exit is unconfirmed",
            "PNPM_PROBE_CLEANUP_TIMEOUT: pnpm version probe cleanup did not finish within 5 seconds",
            "PLUGIN_PROCESS_CLEANUP_FAILED: test process is still active",
            "PLUGIN_PROCESS_COORDINATOR_DROPPED: cleanup coordinator ended unexpectedly",
            "PREINSTALL_WAIT: wait task failed",
            "INTERNAL_PLUGIN_PROCESS_REAP_TIMEOUT: owner remained active",
        ] {
            assert!(
                is_process_cleanup_failure(blocked),
                "{blocked} must terminate startup instead of warning"
            );
        }
        for best_effort in [
            "PNPM_PROBE_LOCK_TIMEOUT: timed out waiting for the process lock",
            "PNPM_PROBE_TIMEOUT: pnpm version probe timed out but the process was recycled",
            "PNPM_PROBE_TASK_FAILED: task panicked",
            "PNPM_INSTALL_FAILED: exit code 1: ERR_PNPM_UNEXPECTED_STORE",
            "INTERNAL_PLUGIN_INSTALL_FAILED: pnpm add failed",
            "DEPRECATED_PLUGIN_UNINSTALL_FAILED: dshmarket: missing",
            "PRESET_PLUGIN_STILL_MISSING: 修复后仍缺失以下插件产物",
        ] {
            assert!(
                !is_process_cleanup_failure(best_effort),
                "{best_effort} must stay best-effort"
            );
        }
    }

    #[test]
    fn orphan_lock_clears_only_for_exited_positive_pid() {
        let alive = |_: u32| false;
        let exited = |_: u32| true;
        assert!(orphan_lock_should_clear("1234\n", exited));
        assert!(!orphan_lock_should_clear("1234\n", alive));
        assert!(!orphan_lock_should_clear("", exited));
        assert!(!orphan_lock_should_clear("   \n", exited));
        assert!(!orphan_lock_should_clear("not-a-pid", exited));
        assert!(!orphan_lock_should_clear("-1", exited));
        assert!(!orphan_lock_should_clear("0", exited));
    }

    #[test]
    fn orphan_lock_file_is_removed_for_dead_holder_and_kept_for_live_one() {
        let dir = std::env::temp_dir().join(format!(
            "dsh-orphan-lock-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = plugin_writer_lock_path(&dir);

        let live = std::process::id();
        std::fs::write(&path, format!("{live}\n")).unwrap();
        assert!(
            !clear_orphan_plugin_writer_lock(&dir),
            "live holder must never be touched"
        );
        assert!(path.exists());

        std::fs::write(&path, "garbage").unwrap();
        assert!(!clear_orphan_plugin_writer_lock(&dir));
        assert!(path.exists());

        std::fs::write(&path, format!("{}\n", u32::MAX - 1)).unwrap();
        assert!(
            clear_orphan_plugin_writer_lock(&dir),
            "unknown/absent pid counts as exited"
        );
        assert!(!path.exists());

        assert!(!clear_orphan_plugin_writer_lock(&dir), "no lock, no-op");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn writer_lock_test_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsh-writer-lock-{}-{:?}-{tag}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writer_lock_records_holder_and_releases_on_drop() {
        let dir = writer_lock_test_dir("release");
        let path = plugin_writer_lock_path(&dir);

        let guard = acquire_plugin_writer_lock(&dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().trim(),
            std::process::id().to_string()
        );

        drop(guard);
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn writer_lock_takes_over_dead_holder() {
        let dir = writer_lock_test_dir("takeover");
        let path = plugin_writer_lock_path(&dir);
        std::fs::write(&path, "4000000\n").unwrap();

        let guard = acquire_plugin_writer_lock(&dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().trim(),
            std::process::id().to_string()
        );

        drop(guard);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn writer_lock_reclaims_claim_left_by_exited_process() {
        let dir = writer_lock_test_dir("stale-claim");
        let path = plugin_writer_lock_path(&dir);
        let record = "4000000\n".to_string();
        std::fs::write(&path, &record).unwrap();
        let claim = path.with_file_name(format!(
            "package.json.lock.takeover-{:016x}",
            lock_record_hash(&record)
        ));
        std::fs::write(&claim, "3999999\n").unwrap();

        let guard = acquire_plugin_writer_lock(&dir).unwrap();
        assert!(!claim.exists());

        drop(guard);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
