
use crate::config;
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(not(windows))]
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use tauri::Manager;

#[cfg(unix)]
use super::process::warn_if_inotify_watch_limit_low;
use super::process::{
    has_owned_process, launch_guard, on_owned_process_exit_at, register_owned_process, stop,
    take_startup_sweep, terminate_stale_harness_processes, LaunchGuard, OwnedProcess,
    HARNESS_PROCESS_EXITED_EVENT,
};
use super::status;
#[cfg(windows)]
use super::sweep::{dsh_bin_open_error, relaunch_marker_path, relaunch_via_shell_escape};
use super::utils::{append_log, rotate_service_log, spawn_output_readers, wait_for_port_release};
use super::win_inspector;
use crate::{AppHandle};

#[cfg(windows)]
type HarnessChild = std::sync::Arc<std::os::windows::io::OwnedHandle>;
#[cfg(unix)]
type HarnessChild = std::process::Child;

pub(super) struct LaunchPlan {
    pub(super) setting: config::Setting,
    pub(super) profile: String,
    pub(super) node: PathBuf,
    pub(super) dsh_binary: PathBuf,
    pub(super) core_dir: PathBuf,
    pub(super) profile_root: PathBuf,
    pub(super) dsh_home: PathBuf,
    pub(super) base: PathBuf,
    pub(super) bin_dir: PathBuf,
    pub(super) pnpm_root: PathBuf,
    pub(super) pnpm_binary: PathBuf,
    pub(super) pid_path: PathBuf,
    pub(super) resources: Option<PathBuf>,
    pub(super) git_dir: Option<PathBuf>,
    pub(super) user_pnpm: Option<PathBuf>,
}

const PORT_RELEASE_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);

#[cfg(target_os = "linux")]
fn harness_library_path(
    appdir: Option<&std::ffi::OsStr>,
    library_path: Option<&std::ffi::OsStr>,
) -> Option<OsString> {
    use std::os::unix::ffi::OsStrExt;

    let library_path = library_path?;
    let Some(appdir) = appdir.filter(|path| !path.is_empty()) else {
        return Some(library_path.to_os_string());
    };
    Some(
        std::env::join_paths(
            library_path
                .as_bytes()
                .split(|byte| matches!(*byte, b':' | b';'))
                .map(|path| Path::new(std::ffi::OsStr::from_bytes(path)))
                .filter(|path| !path.starts_with(Path::new(appdir))),
        )
        .expect("split Linux library paths contain no path separator"),
    )
}

fn fatal_report_path(log_path: &Path) -> PathBuf {
    static NEXT_REPORT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_REPORT.fetch_add(1, Ordering::Relaxed);
    log_path.with_file_name(format!(
        "harness-fatal-{}-{timestamp}-{sequence}.json",
        std::process::id()
    ))
}

fn log_fatal_report(log_path: &PathBuf, report_path: &Path) {
    if report_path.is_file() {
        let line = format!("ERROR HARNESS_FATAL_REPORT: {}", report_path.display());
        append_log(log_path, &line);
        log::error!("{line}");
    }
}

fn build_harness_args(
    dsh_binary: &Path,
    profile: &str,
    patch: Option<&Path>,
    port: u16,
    heap_mb: Option<u32>,
    report_path: &Path,
) -> Vec<OsString> {
    let mut args = Vec::with_capacity(12);
    args.extend(super::heap::heap_option_arg(heap_mb));
    let mut directory = OsString::from("--report-directory=");
    directory.push(report_path.parent().unwrap_or(Path::new(".")));
    let mut filename = OsString::from("--report-filename=");
    filename.push(report_path.file_name().unwrap_or_default());
    args.extend([
        OsString::from("--report-on-fatalerror"),
        directory,
        filename,
    ]);
    args.extend([
        dsh_binary.as_os_str().to_os_string(),
        OsString::from("--profile"),
        OsString::from(profile),
    ]);
    if let Some(patch) = patch {
        args.push(OsString::from("--patch"));
        args.push(patch.as_os_str().to_os_string());
    }
    args.extend([
        OsString::from("--port"),
        OsString::from(port.to_string()),
        OsString::from("--no-open"),
    ]);
    args
}

fn find_available_port(start: u16, dsh_binary: &Path) -> Result<u16, String> {
    find_available_port_by(start, |port| {
        prepare_port(dsh_binary, port).map(|free| !free)
    })
}

fn prepare_port(dsh_binary: &Path, port: u16) -> Result<bool, String> {
    if !super::utils::is_port_in_use(port) {
        return Ok(true);
    }
    #[cfg(windows)]
    {
        super::process::reclaim_orphan_port(dsh_binary, port)
    }
    #[cfg(not(windows))]
    {
        let _ = dsh_binary;
        log::warn!(
            "PORT_OWNER: port={port} pid={:?} ownership=unknown",
            super::sweep::port_owner_pid(port)
        );
        Ok(false)
    }
}

fn find_available_port_by(
    start: u16,
    mut is_in_use: impl FnMut(u16) -> Result<bool, String>,
) -> Result<u16, String> {
    let mut port = start;
    loop {
        if !is_in_use(port)? {
            return Ok(port);
        }
        log::warn!("Port {port} is occupied, trying the next port");
        port = port.checked_add(1).ok_or_else(|| {
            "PORT_EXHAUSTED: no available TCP port after the configured port".to_string()
        })?;
    }
}

fn resolve_heal_port(configured: u16, heal_target: u16, heal_target_free: bool) -> u16 {
    if configured != heal_target && heal_target_free {
        heal_target
    } else {
        configured
    }
}

pub async fn start(app_handle: AppHandle) -> Result<(), String> {
    super::early::wait_preparation().await?;
    let setting = config::get_store_dat_setting(&app_handle);
    let node_binary_path = config::get_node_binary_path(&app_handle);
    let dsh_binary_path = crate::service::core::active_dsh_binary(&app_handle);

    if !setting.installed {
        log::debug!("Harness not installed, skipping startup");
        return Ok(());
    }
    if !node_binary_path.exists() || !dsh_binary_path.exists() {
        #[cfg(windows)]
        if dsh_bin_open_error(&app_handle) == Some(448) {
            relaunch_via_shell_escape(&app_handle);
        }
        let mut setting = config::get_store_dat_setting(&app_handle);
        setting.installed = false;
        config::set_store_dat_setting(&app_handle, setting);
        log::info!("Runtime files missing (node/dsh), resetting installed flag");
        return Ok(());
    }

    if has_owned_process() {
        log::info!("Owned Harness process is already running");
        status::set_status(status::Status::Running);
        status::emit_status(&app_handle);
        return Ok(());
    }

    #[cfg(windows)]
    let _ = std::fs::remove_file(relaunch_marker_path(&app_handle));

    log::info!("Starting Harness service");
    status::set_status(status::Status::Starting);
    status::emit_status(&app_handle);
    launch(app_handle).await?;

    Ok(())
}

pub async fn restart(app_handle: AppHandle) -> Result<(), String> {
    log::info!("Restarting Harness service");

    stop(app_handle.clone()).await?;

    start(app_handle).await?;

    Ok(())
}

fn reset_active_profile_root(profile_root: &Path) {
    if !profile_root.is_dir() {
        return;
    }
    let root_config = profile_root.join("cordis.yml");
    const PROFILE_ROOT_EMPTY: &str = "# dsh profile root — an empty entry list. The tree is composed as patches:\n# each bundle in package.json's dsh.profile.bundles, then cordis.patch.yml, then any\n# --patch overlays. Edit cordis.patch.yml, not this file.\n[]\n";
    if std::fs::read_to_string(&root_config).ok().as_deref() != Some(PROFILE_ROOT_EMPTY) {
        if let Err(e) = std::fs::write(&root_config, PROFILE_ROOT_EMPTY) {
            log::warn!("PROFILE_ROOT_RESET_FAILED: {}: {e}", root_config.display());
        } else {
            log::info!(
                "Reset stale profile root before spawn: {}",
                root_config.display()
            );
        }
    }
}

fn wait_harness(
    child: &mut HarnessChild,
    timeout: Option<std::time::Duration>,
) -> Result<Option<Option<i64>>, String> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, WaitForSingleObject, INFINITE,
        };

        let handle = child.as_raw_handle();
        let millis = timeout.map_or(INFINITE, |value| value.as_millis() as u32);
        match unsafe { WaitForSingleObject(handle, millis) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                Ok(Some(
                    (unsafe { GetExitCodeProcess(handle, &mut code) } != 0)
                        .then_some(i64::from(code)),
                ))
            }
            _ => Err(format!(
                "PROCESS_WAIT_FAILED: {}",
                std::io::Error::last_os_error()
            )),
        }
    }
    #[cfg(unix)]
    {
        let Some(timeout) = timeout else {
            return child
                .wait()
                .map(|exit| Some(exit.code().map(i64::from)))
                .map_err(|error| format!("PROCESS_WAIT_FAILED: {error}"));
        };
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(exit)) => return Ok(Some(exit.code().map(i64::from))),
                Ok(None) if std::time::Instant::now() >= deadline => return Ok(None),
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
                Err(error) => return Err(format!("PROCESS_WAIT_FAILED: {error}")),
            }
        }
    }
}

fn supervise_harness(
    mut on_exit: impl FnMut(u32, Option<i64>, Option<&Path>),
    log_path: &PathBuf,
    started: &mut Option<tokio::sync::oneshot::Sender<Result<(), String>>>,
    guards: (LaunchGuard, tokio::sync::OwnedMutexGuard<()>),
    mut spawn: impl FnMut(
        &Path,
    ) -> Result<
        (
            OwnedProcess,
            HarnessChild,
            std::io::Result<std::sync::mpsc::Receiver<bool>>,
        ),
        String,
    >,
    mut reset: impl FnMut(),
    mut persist: impl FnMut(u32),
) -> Result<(), String> {
    let mut guards = Some(guards);
    let mut previous_exit = None;
    for attempt in 1..=3 {
        let report_path = fatal_report_path(log_path);
        let registered = register_owned_process(previous_exit.map(|(pid, _)| pid), || {
            if previous_exit.is_some() {
                reset();
            }
            let (owned, child, signature) = spawn(&report_path)?;
            #[cfg(unix)]
            let mut child = child;
            let pid = owned.pid;
            let signature = match signature {
                Ok(signature) => {
                    persist(pid);
                    Ok(signature)
                }
                Err(error) => {
                    let error = format!("PROCESS_START_FAILED: spawn output reader: {error}");
                    #[cfg(windows)]
                    let cleanup = {
                        use std::os::windows::io::AsRawHandle;
                        super::process::terminate_windows_process(
                            pid,
                            child.as_raw_handle(),
                            super::process::kill_pid_tree,
                        )
                    };
                    #[cfg(unix)]
                    let cleanup = (|| {
                        if child.try_wait()?.is_none() {
                            unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
                            if let Err(kill) = child.kill() {
                                if child.try_wait()?.is_none() {
                                    return Err(kill);
                                }
                            }
                        }
                        child.wait().map(|_| ())
                    })()
                    .map_err(|cleanup: std::io::Error| {
                        format!("HARNESS_CLEANUP_FAILED: process {pid}: {cleanup}")
                    });
                    match cleanup {
                        Ok(()) => return Err(error),
                        Err(cleanup) => Err(format!("{error}; {cleanup}")),
                    }
                }
            };
            Ok((owned, (pid, child, signature)))
        });
        let (pid, mut child, signature) = match registered {
            Ok(Some(process)) => process,
            Ok(None) => return Err("STARTUP_CANCELLED: Harness launch was stopped".into()),
            Err(error) => {
                if let Some((pid, code)) = previous_exit {
                    on_exit(pid, code, None);
                }
                return Err(error);
            }
        };
        let signature = signature?;
        log::info!("Harness process spawned and registered: pid={pid}, attempt={attempt}");
        if let Some(started) = started.take() {
            let _ = started.send(Ok(()));
        }
        let mut wait = |timeout| {
            let result = wait_harness(&mut child, timeout);
            if let Err(error) = &result {
                log::error!(
                    "{error}; terminating this Harness process before releasing its monitor"
                );
                #[cfg(windows)]
                {
                    use std::os::windows::io::AsRawHandle;
                    super::process::terminate_windows_process(
                        pid,
                        child.as_raw_handle(),
                        super::process::kill_pid_tree,
                    )?;
                }
                #[cfg(unix)]
                {
                    super::process::kill_pid_tree(pid);
                    child
                        .wait()
                        .map_err(|cleanup| format!("{error}; cleanup failed: {cleanup}"))?;
                }
                on_exit(pid, None, Some(&report_path));
            }
            result
        };
        let code = match wait(Some(std::time::Duration::from_millis(2500)))? {
            Some(code) => code,
            None => {
                let (launch, transition) = guards.take().unwrap();
                drop(launch);
                drop(transition);
                let code = wait(None)?.flatten();
                on_exit(pid, code, Some(&report_path));
                return Ok(());
            }
        };
        let duplicate = matches!(
            signature.recv_timeout(std::time::Duration::from_millis(250)),
            Ok(true)
        );
        if code == Some(1) && duplicate && attempt < 3 {
            log_fatal_report(log_path, &report_path);
            log::warn!(
                "DUPLICATE_LOADER_ENTRY: dsh exited early with duplicate loader entry \
                 (pid={pid}, code=1); resetting profile root and relaunching (attempt {attempt}/3)"
            );
            previous_exit = Some((pid, code));
            continue;
        }
        on_exit(pid, code, Some(&report_path));
        return Ok(());
    }
    unreachable!()
}

pub async fn launch(app_handle: AppHandle) -> Result<(), String> {
    super::early::wait_preparation().await?;
    // 启动分段计时（issue #766）：只写日志，不改控制流。每段一行 `STARTUP_PHASE`，
    let startup_started = std::time::Instant::now();
    let mut phase_started = startup_started;
    let mark_phase = |name: &str, phase_started: &mut std::time::Instant| {
        let now = std::time::Instant::now();
        log::info!(
            "STARTUP_PHASE: name={name} ms={} total={}",
            now.duration_since(*phase_started).as_millis(),
            now.duration_since(startup_started).as_millis()
        );
        *phase_started = now;
    };
    let mut setting = config::get_store_dat_setting(&app_handle);
    setting.port = super::instances::port(&app_handle);
    let node_binary_path = config::get_node_binary_path(&app_handle);
    let transition = super::process::acquire_core_transition();
    tokio::pin!(transition);
    let _transition_guard = loop {
        if launch_guard().load(Ordering::SeqCst) && has_owned_process() {
            return Ok(());
        }
        tokio::select! {
            guard = &mut transition => break guard?,
            _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {}
        }
    };
    let dsh_binary_path = crate::service::core::active_dsh_binary(&app_handle);

    log::debug!("Checking Node.js path: {:?}", node_binary_path);
    if !node_binary_path.exists() {
        log::error!("Node.js not installed");
        return Err("NODE_NOT_FOUND: Node.js not installed".to_string());
    }
    log::debug!("Checking Harness path: {:?}", dsh_binary_path);
    if !dsh_binary_path.exists() {
        log::error!("Harness not installed");
        return Err("HARNESS_NOT_FOUND: Harness not installed".to_string());
    }
    mark_phase("resolve", &mut phase_started);

    if has_owned_process() {
        log::info!("Owned Harness process is already running, skipping launch");
        return Ok(());
    }
    if launch_guard()
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        log::info!("Harness launch already in progress, skipping");
        return Ok(());
    }
    let _launch_guard = LaunchGuard(launch_guard());
    // 不清扫会导致端口一路漂移（3080→…→3085，issue #91）并让后续目录互换
    {
        let handle = app_handle.clone();
        let profile = crate::service::profile::active_profile(&app_handle);
        tauri::async_runtime::spawn_blocking(move || {
            super::instances::scope_sync(profile, || {
                if take_startup_sweep() {
                    let result = terminate_stale_harness_processes(&handle);
                    if result.is_err() {
                        super::process::retry_startup_sweep();
                    }
                    result
                } else {
                    log::debug!(
                        "Skipping pre-launch stale Harness sweep: already swept during this launch"
                    );
                    Ok(())
                }
            })
        })
        .await
        .map_err(|error| format!("HARNESS_SWEEP_FAILED: {error}"))??;
    }

    // 端口只增不减、一路从 3080 漂到 3084+（issue #91）。先于
    let heal_target = setting.manual_port.unwrap_or(config::default_port());
    let entry = dsh_binary_path.clone();
    let profile = crate::service::profile::active_profile(&app_handle);
    let heal_target_free = tauri::async_runtime::spawn_blocking(move || {
        super::instances::scope_sync(profile, || {
            wait_for_port_release(heal_target, PORT_RELEASE_WAIT);
            prepare_port(&entry, heal_target)
        })
    })
    .await
    .map_err(|error| format!("HARNESS_CLEANUP_FAILED: {error}"))??;
    let healed_port = resolve_heal_port(setting.port, heal_target, heal_target_free);
    if healed_port != setting.port {
        log::info!(
            "Harness port healed from {} back to {} (no longer occupied)",
            setting.port,
            healed_port
        );
        setting.port = healed_port;
        super::instances::set_port(setting.port);
    }

    let port = setting.port;
    let entry = dsh_binary_path.clone();
    let profile = crate::service::profile::active_profile(&app_handle);
    let available_port = tauri::async_runtime::spawn_blocking(move || {
        super::instances::scope_sync(profile, || {
            if port != heal_target {
                wait_for_port_release(port, PORT_RELEASE_WAIT);
            }
            find_available_port(port, &entry)
        })
    })
    .await
    .map_err(|error| format!("HARNESS_CLEANUP_FAILED: {error}"))??;
    if available_port != setting.port {
        log::info!(
            "Harness port changed from {} to {} because the configured port is occupied",
            setting.port,
            available_port
        );
        setting.port = available_port;
        super::instances::set_port(setting.port);
    }
    super::instances::set_port(setting.port);
    mark_phase("stale_sweep_and_port", &mut phase_started);

    // 建目录 + 可写性预检（issue #466）：`~/.dsh` 属主不是当前用户时（典型：此前
    let dsh_home = config::get_dsh_data_path(&app_handle);
    crate::service::perm::ensure_dir_writable(&dsh_home, "DSH_HOME_MKDIR_FAILED")?;
    crate::service::profile::migrate_desktop_profile_name(&app_handle);

    // 预检不创建目录，避免抢先建出半初始化档案（issue #452）。
    crate::service::perm::ensure_writable_path(
        &crate::service::plugin::profile_dir(&app_handle),
        &dsh_home,
        "PROFILE_NOT_WRITABLE",
    )?;

    // 核心 bundle 层自愈（issue #452）：当前档案的 `dsh.profile.bundles` 必须带
    if let Err(e) = crate::service::profile::ensure_active_profile_core_bundles(&app_handle) {
        log::warn!("ensure active profile core bundles failed: {e}");
    }

    if let Err(e) = crate::service::plugin::purge_user_plugins_in_safe_profile(&app_handle) {
        log::warn!("safe mode user plugin purge failed: {e}");
    }
    mark_phase("profile_prepare", &mut phase_started);

    // 监视 profile 目录，上限过低会在启动一瞬间抛 ENOSPC 直接退出（issue #116）。
    #[cfg(unix)]
    warn_if_inotify_watch_limit_low();

    if let Err(e) = win_inspector::apply(&app_handle) {
        log::warn!("win32 legacy patch cleanup failed: {e}");
    }
    if let Err(e) = crate::service::patch::renderer::apply(&app_handle) {
        log::warn!("renderer SlotOutlet patch failed: {e}");
    }
    if let Err(e) = crate::service::patch::composer::apply(&app_handle) {
        log::warn!("composer workspace-less patch failed: {e}");
    }
    if let Err(e) = crate::service::patch::conversation_draft::apply(&app_handle) {
        log::warn!("conversation draft compatibility patch failed: {e}");
    }
    if let Err(e) = crate::service::patch::permission_settings::apply(&app_handle) {
        log::warn!("permission settings error patch failed: {e}");
    }
    if let Err(e) = crate::service::patch::scroll_follow::apply(&app_handle) {
        log::warn!("Scroll follow reduced-motion patch skipped: {e}");
    }
    if let Err(e) = crate::service::patch::chat_follow_anchor::apply(&app_handle) {
        log::warn!("Chat follow anchor patch skipped: {e}");
    }
    if let Err(e) = crate::service::patch::mobile_composer::apply(&app_handle) {
        log::warn!("mobile composer patch failed: {e}");
    }
    if let Err(e) = crate::service::patch::mobile_sidebar::apply(&app_handle) {
        log::warn!("mobile sidebar patch failed: {e}");
    }
    if let Err(e) = crate::service::patch::session::apply(&app_handle) {
        log::warn!("SessionStore.remove patch failed: {e}");
    }
    if let Err(e) = crate::service::patch::llm_session::apply(&app_handle) {
        log::warn!("pi-ai session header patch failed: {e}");
    }
    if let Err(e) = crate::service::patch::pi_ai_thinking::apply(&app_handle) {
        log::warn!("pi-ai thinking-as-text patch failed: {e}");
    }
    if let Err(e) = crate::service::patch::model_selection::apply(&app_handle) {
        log::warn!("model selection mouse click patch failed: {e}");
    }
    if let Err(e) = crate::service::patch::workspace::apply(&app_handle) {
        log::warn!("workspace worktree membership patch failed: {e}");
    }
    if let Err(e) = crate::service::patch::workspace_view::apply(&app_handle) {
        log::warn!("workspace view state patch failed: {e}");
    }
    if let Err(e) = crate::service::patch::plugin_visibility::apply(&app_handle) {
        log::warn!("plugin visibility patch failed: {e}");
    }
    crate::service::patch::mcp_startup::apply(&app_handle)?;
    mark_phase("core_patches", &mut phase_started);
    if let Err(e) = crate::service::plugin::ensure_profile_npmrc(&app_handle) {
        log::warn!("ensure profile .npmrc failed: {e}");
    }
    if let Err(e) = crate::service::plugin::ensure_internal_plugins(&app_handle).await {
        if crate::service::plugin::is_process_cleanup_failure(&e) {
            log::error!("ensure internal plugins blocked by unconfirmed process cleanup: {e}");
            return Err(e);
        }
        log::warn!("ensure internal plugins failed: {e}");
    }
    mark_phase("ensure_internal_plugins", &mut phase_started);
    if let Err(e) = crate::service::plugin::ensure_preset_plugins(&app_handle).await {
        log::error!("ensure preset plugins blocked by unconfirmed process cleanup: {e}");
        return Err(e);
    }
    mark_phase("ensure_preset_plugins", &mut phase_started);
    if let Err(e) = crate::service::patch::plugin_visibility::apply(&app_handle) {
        log::warn!("plugin visibility patch after plugin self-healing failed: {e}");
    }
    if let Err(e) = crate::service::core::prepare_active_runtime(&app_handle).await {
        log::error!("prepare active core runtime failed: {e}");
        return Err(e);
    }
    mark_phase("prepare_active_runtime", &mut phase_started);
    // （issue #441），此时必须重新解析：下面的 DSH_NODE 注入与 PATH 前置都以
    let node_binary_path = config::get_node_binary_path(&app_handle);
    if !node_binary_path.exists() {
        log::error!("Node.js runtime resolved for launch is missing");
        return Err(format!(
            "NODE_NOT_FOUND: Node.js runtime is missing: {}",
            node_binary_path.display()
        ));
    }
    let profile = crate::service::profile::active_profile(&app_handle);
    let plan = LaunchPlan {
        setting,
        profile_root: crate::service::profile::profile_dir_of(&app_handle, &profile),
        profile,
        node: node_binary_path,
        dsh_binary: dsh_binary_path,
        core_dir: crate::service::core::active_core_dir(&app_handle),
        dsh_home,
        base: config::get_base_dir(&app_handle),
        bin_dir: crate::service::cli::get_bin_dir(&app_handle),
        pnpm_root: config::get_pnpm_install_path(&app_handle),
        pnpm_binary: config::get_pnpm_binary_path(&app_handle),
        pid_path: super::sweep::harness_pid_path(&app_handle),
        resources: app_handle.path().resource_dir().ok(),
        git_dir: config::get_git_cmd_dir(&app_handle),
        user_pnpm: crate::service::cli::find_user_pnpm(&app_handle),
    };
    let notifications = Arc::new(OnceLock::new());
    let _ = notifications.set(app_handle);
    launch_prepared(
        plan,
        (_launch_guard, _transition_guard),
        notifications,
        #[cfg(windows)]
        None,
    )
    .await
}

pub(super) async fn launch_prepared(
    plan: LaunchPlan,
    guards: (LaunchGuard, tokio::sync::OwnedMutexGuard<()>),
    notifications: Arc<OnceLock<AppHandle>>,
    #[cfg(windows)] job: Option<Arc<super::win_spawn::ProcessJob>>,
) -> Result<(), String> {
    let prepared_started = std::time::Instant::now();
    let mut phase_started = prepared_started;
    let mark_phase = |name: &str, phase_started: &mut std::time::Instant| {
        let now = std::time::Instant::now();
        log::info!(
            "STARTUP_PHASE: name={name} ms={} prepared_total={}",
            now.duration_since(*phase_started).as_millis(),
            now.duration_since(prepared_started).as_millis()
        );
        *phase_started = now;
    };
    let LaunchPlan {
        setting,
        profile: active_profile,
        node: node_binary_path,
        dsh_binary: dsh_binary_path,
        core_dir,
        profile_root,
        dsh_home,
        base,
        bin_dir,
        pnpm_root,
        pnpm_binary,
        pid_path,
        resources,
        git_dir,
        user_pnpm,
    } = plan;
    let mut envs: HashMap<String, String> = HashMap::new();
    envs.insert(
        "DSH_HOME".to_string(),
        dsh_home.to_string_lossy().into_owned(),
    );
    envs.insert("DSH_TELEMETRY_DISABLED".to_string(), "1".to_string());
    envs.insert("NO_COLOR".to_string(), "1".to_string());
    envs.insert("DSH_WEB_PORT".to_string(), setting.port.to_string());
    // shim 报 "Node.js runtime not found"（issue #121，与 build_plugin_envs
    let node_abs =
        dunce::canonicalize(&node_binary_path).unwrap_or_else(|_| node_binary_path.clone());
    envs.insert(
        "DSH_NODE".to_string(),
        node_abs.to_string_lossy().into_owned(),
    );

    // （轻量缓解，issue #69 系列）。
    if let Some(node_dir) = node_binary_path.parent() {
        if let Some(existing_path) = std::env::var_os("PATH") {
            let git_dirs = win_inspector::git_bash_bin_dirs();
            for dir in &git_dirs {
                log::debug!("harness service PATH prepend: {}", dir.to_string_lossy());
            }
            let mut paths = vec![bin_dir.clone()];
            paths.push(node_dir.to_path_buf());
            if let Some(git_dir) = git_dir {
                log::debug!(
                    "harness service Git PATH prepend: {}",
                    git_dir.to_string_lossy()
                );
                paths.push(git_dir);
            }
            paths.extend(git_dirs);
            paths.extend(std::env::split_paths(&existing_path));
            if let Ok(new_path) = std::env::join_paths(paths) {
                envs.insert("PATH".to_string(), new_path.to_string_lossy().into_owned());
            }
        }
    }

    // 后又因 PATH 看不到真正的 pnpm（issue #139）。
    if let Some(user_pnpm) = user_pnpm.as_deref() {
        if let Some(pnpm_value) = crate::service::cli::pnpm_env_value(user_pnpm, &bin_dir) {
            envs.insert("DSH_PNPM".to_string(), pnpm_value);
        }
    }

    match crate::service::plugin::harness_prefer_bundled_pnpm_from(
        &profile_root,
        &pnpm_root,
        &pnpm_binary,
        user_pnpm.as_deref(),
        &node_binary_path,
        &bin_dir,
    )
    .await
    {
        Ok(true) => {
            envs.insert("DSH_PREFER_BUNDLED_PNPM".to_string(), "1".to_string());
        }
        Ok(false) => {}
        Err(e) if crate::service::plugin::is_process_cleanup_failure(&e) => {
            log::error!("prefer bundled pnpm blocked by unconfirmed process cleanup: {e}");
            return Err(e);
        }
        Err(e) => log::warn!("prefer bundled pnpm probe failed, keeping user pnpm: {e}"),
    }

    // 桌面端代理设置（issue #110）：核心（dsh-http-proxy）在启动时读
    envs.extend(config::proxy::proxy_child_env(&setting.proxy_url));

    envs.insert("DSH_TAURI_EMBEDDED".to_string(), "1".to_string());

    // issue #1064：AppImage 的旧库不能泄漏给 Harness 启动的宿主目录选择器。
    #[cfg(target_os = "linux")]
    let library_path = harness_library_path(
        std::env::var_os("APPDIR").as_deref(),
        std::env::var_os("LD_LIBRARY_PATH").as_deref(),
    );

    let log_path = config::service_log_path_from(&base, Some(&active_profile));
    fs::create_dir_all(log_path.parent().unwrap_or(std::path::Path::new(".")))
        .map_err(|e| format!("LOG_DIR_MKDIR_FAILED: create log dir failed: {e}"))?;
    rotate_service_log(&log_path, 3);

    if let Err(e) = crate::service::plugin::preflight_patch_entries_at(
        &profile_root,
        &dsh_home,
        Some(&dsh_binary_path),
    ) {
        log::error!("patch layer entry preflight failed: {e}");
        return Err(e);
    }
    mark_phase("patch_entry_preflight", &mut phase_started);

    let inherited_heap_mb = super::heap::node_options_heap_limit_mb();
    let inherited_heap_flags = super::heap::node_options_has_heap_flags_env();
    let heap_mb = super::heap::resolve_heap_limit_mb(
        setting.harness_max_heap_mb,
        inherited_heap_flags,
        super::heap::physical_memory_mb(),
    );
    if super::heap::heap_option_arg(heap_mb).is_some() {
        if let Ok(node_options) = std::env::var("NODE_OPTIONS") {
            if super::heap::node_options_has_heap_flags(&node_options) {
                let stripped = super::heap::node_options_without_heap_flags(&node_options);
                log::info!(
                    "Dropped inherited heap flags from NODE_OPTIONS for the Harness process (kept={:?})",
                    stripped
                );
                envs.insert("NODE_OPTIONS".to_string(), stripped.unwrap_or_default());
            }
        }
    }
    match super::heap::effective_limit_mb(heap_mb, inherited_heap_mb) {
        Some(mb) => log::info!(
            "Starting Harness process with effective heap limit {mb} MB (configured={:?}, inherited_from_node_options={:?})",
            setting.harness_max_heap_mb,
            inherited_heap_mb
        ),
        None if inherited_heap_flags => log::info!(
            "Starting Harness process with the heap limit inherited from NODE_OPTIONS (configured={:?})",
            setting.harness_max_heap_mb
        ),
        None => log::info!(
            "Starting Harness process with the Node default heap limit (configured={:?}, inherited_from_node_options={:?})",
            setting.harness_max_heap_mb,
            inherited_heap_mb
        ),
    }

    reset_active_profile_root(&profile_root);

    let hmr_patch = crate::service::plugin::local_plugin_hmr_sync_at(
        &base,
        &profile_root,
        resources.as_deref(),
        setting.local_plugin_hmr,
    );
    if let Some(path) = hmr_patch.as_deref() {
        log::info!("Injecting local plugin HMR layer: {}", path.display());
    }

    if let Some(parent) = pid_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let port = setting.port;
    mark_phase("pre_spawn_setup", &mut phase_started);
    super::startup::note_spawn();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("harness-startup".into())
        .spawn(move || {
            super::instances::scope_sync(active_profile.clone(), || {
                let mut started = Some(started_tx);
                let result = supervise_harness(
                    |pid, code, report_path| {
                        on_owned_process_exit_at(
                            pid,
                            |_| {
                                if let Some(report_path) = report_path {
                                    log_fatal_report(&log_path, report_path);
                                }
                                code
                            },
                            |payload| {
                                if let Some(app) = notifications.get() {
                                    status::emit_status(app);
                                    super::instances::emit(
                                        app,
                                        HARNESS_PROCESS_EXITED_EVENT,
                                        payload,
                                    );
                                }
                            },
                        );
                    },
                    &log_path,
                    &mut started,
                    guards,
                    |report_path| {
                        let args = build_harness_args(
                            &dsh_binary_path,
                            &active_profile,
                            hmr_patch.as_deref(),
                            port,
                            heap_mb,
                            report_path,
                        );
                        #[cfg(windows)]
                        {
                            use std::os::windows::io::{FromRawHandle, OwnedHandle};
                            let spawned = if let Some(job) = job.as_deref() {
                                super::win_spawn::spawn_with_hidden_console_in_job(
                                    &node_binary_path,
                                    &args,
                                    Some(&core_dir),
                                    &envs,
                                    job,
                                )
                            } else {
                                super::win_spawn::spawn_with_hidden_console_owned(
                                    &node_binary_path,
                                    &args,
                                    Some(&core_dir),
                                    &envs,
                                )
                            };
                            let (stdout, stderr, pid, handle) = spawned
                                .map_err(|error| format!("PROCESS_START_FAILED: {error}"))?;
                            let handle = Arc::new(unsafe { OwnedHandle::from_raw_handle(handle) });
                            let signature =
                                spawn_output_readers(Some(stdout), Some(stderr), log_path.clone());
                            Ok((
                                OwnedProcess {
                                    pid,
                                    handle: handle.clone(),
                                },
                                handle,
                                signature,
                            ))
                        }
                        #[cfg(unix)]
                        {
                            use std::os::unix::process::CommandExt;
                            let mut command = Command::new(&node_binary_path);
                            crate::config::apply_node_env(&mut command);
                            command
                                .args(args)
                                .envs(&envs)
                                .current_dir(&core_dir)
                                .stdin(Stdio::null())
                                .stdout(Stdio::piped())
                                .stderr(Stdio::piped())
                                .process_group(0);
                            #[cfg(target_os = "linux")]
                            if let Some(path) = &library_path {
                                command.env("LD_LIBRARY_PATH", path);
                            }
                            let mut child = command
                                .spawn()
                                .map_err(|error| format!("PROCESS_START_FAILED: {error}"))?;
                            let pid = child.id();
                            let signature = spawn_output_readers(
                                child.stdout.take(),
                                child.stderr.take(),
                                log_path.clone(),
                            );
                            Ok((OwnedProcess { pid }, child, signature))
                        }
                    },
                    || reset_active_profile_root(&profile_root),
                    |pid| {
                        let _ = fs::write(&pid_path, format!("{pid}\n{port}\n"));
                    },
                );
                if let Err(error) = result {
                    log::error!("Harness startup supervisor failed: {error}");
                    if let Some(started) = started.take() {
                        let _ = started.send(Err(error));
                    }
                }
            });
        })
        .map_err(|error| format!("PROCESS_START_FAILED: spawn startup supervisor: {error}"))?;
    let result = started_rx
        .await
        .map_err(|error| format!("PROCESS_START_FAILED: startup supervisor ended: {error}"))?;
    mark_phase("spawn_and_register", &mut phase_started);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    mod appimage_library_path {
        use super::*;
        use std::ffi::OsStr;
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        #[test]
        fn removes_only_appimage_library_directories() {
            assert_eq!(
                harness_library_path(
                    Some(OsStr::new("/tmp/.mount_DSH")),
                    Some(OsStr::new("/opt/custom/lib:/tmp/.mount_DSH/usr/lib:/tmp/.mount_DSH:/tmp/.mount_DSH-other/lib:")),
                ),
                Some(OsString::from("/opt/custom/lib:/tmp/.mount_DSH-other/lib:")),
            );
        }

        #[test]
        fn filters_semicolon_and_mixed_library_path_separators() {
            for path in [
                "/opt/custom/lib;/tmp/.mount_DSH/usr/lib",
                "/tmp/.mount_DSH/usr/lib;/opt/custom/lib",
                "/tmp/.mount_DSH/usr/lib:/opt/custom/lib;/tmp/.mount_DSH/usr/lib64",
            ] {
                assert_eq!(
                    harness_library_path(
                        Some(OsStr::new("/tmp/.mount_DSH")),
                        Some(OsStr::new(path))
                    ),
                    Some(OsString::from("/opt/custom/lib")),
                    "{path}",
                );
            }
        }

        #[test]
        fn retains_empty_components_and_loader_tokens() {
            assert_eq!(
                harness_library_path(
                    Some(OsStr::new("/tmp/.mount_DSH")),
                    Some(OsStr::new(";/tmp/.mount_DSH/usr/lib;;$ORIGIN/$LIB:;")),
                ),
                Some(OsString::from("::$ORIGIN/$LIB::")),
            );
        }

        #[test]
        fn filters_non_utf8_appimage_paths_without_removing_siblings() {
            assert_eq!(
                harness_library_path(
                    Some(OsStr::from_bytes(b"/tmp/.mount_\xff")),
                    Some(OsStr::from_bytes(
                        b"/tmp/.mount_\xff/usr/lib;/opt/\xfe/lib:/tmp/.mount_\xff-other/lib"
                    )),
                ),
                Some(OsString::from_vec(
                    b"/opt/\xfe/lib:/tmp/.mount_\xff-other/lib".to_vec()
                )),
            );
        }

        #[test]
        fn preserves_library_path_without_an_appimage_directory() {
            let path = Some(OsStr::new("/opt/custom/lib"));
            assert_eq!(
                harness_library_path(None, path),
                Some(OsString::from("/opt/custom/lib"))
            );
            assert_eq!(
                harness_library_path(Some(OsStr::new("")), path),
                Some(OsString::from("/opt/custom/lib"))
            );
        }

        #[test]
        fn preserves_an_unset_library_path() {
            assert_eq!(
                harness_library_path(Some(OsStr::new("/tmp/.mount_DSH")), None),
                None
            );
        }

        #[test]
        fn clears_a_library_path_containing_only_appimage_directories() {
            for path in [
                "/tmp/.mount_DSH/usr/lib",
                "/tmp/.mount_DSH/usr/lib;/tmp/.mount_DSH/usr/lib64",
                "",
            ] {
                assert_eq!(
                    harness_library_path(
                        Some(OsStr::new("/tmp/.mount_DSH")),
                        Some(OsStr::new(path))
                    ),
                    Some(OsString::new()),
                    "{path}",
                );
            }
        }

        #[test]
        fn preserves_non_utf8_user_library_paths() {
            assert_eq!(
                harness_library_path(
                    Some(OsStr::new("/tmp/.mount_DSH")),
                    Some(OsStr::from_bytes(b"/opt/\xff/lib:/tmp/.mount_DSH/usr/lib"))
                ),
                Some(OsString::from_vec(b"/opt/\xff/lib".to_vec())),
            );
        }
    }

    #[test]
    fn occupied_port_advances_to_a_free_port() {
        let mut checked = Vec::new();
        let selected = find_available_port_by(3080, |port| {
            checked.push(port);
            Ok(port < 3082)
        })
        .expect("find next free port");

        assert_eq!(selected, 3082);
        assert_eq!(checked, vec![3080, 3081, 3082]);
    }

    #[test]
    fn occupied_port_reports_exhaustion_at_max_port() {
        let error = find_available_port_by(u16::MAX, |_| Ok(true)).expect_err("port exhaustion");
        assert!(error.starts_with("PORT_EXHAUSTED:"));
    }

    #[test]
    fn confirmed_orphan_cleanup_failure_does_not_advance_port() {
        let mut checked = Vec::new();
        let error = find_available_port_by(3080, |port| {
            checked.push(port);
            Err("HARNESS_CLEANUP_FAILED: retained listener".to_string())
        })
        .unwrap_err();
        assert_eq!(error, "HARNESS_CLEANUP_FAILED: retained listener");
        assert_eq!(checked, vec![3080]);
    }

    fn text_args(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn patch_layer_argument_precedes_the_port_flags() {
        let patch = Path::new("D:/app-data/cordis.hmr.patch.yml");
        let args = build_harness_args(
            Path::new("D:/core/lib/bin.js"),
            "tauri",
            Some(patch),
            3080,
            None,
            Path::new("D:/app-data/logs/fatal.json"),
        );
        assert_eq!(
            text_args(&args),
            vec![
                "--report-on-fatalerror".to_string(),
                "--report-directory=D:/app-data/logs".to_string(),
                "--report-filename=fatal.json".to_string(),
                "D:/core/lib/bin.js".to_string(),
                "--profile".to_string(),
                "tauri".to_string(),
                "--patch".to_string(),
                "D:/app-data/cordis.hmr.patch.yml".to_string(),
                "--port".to_string(),
                "3080".to_string(),
                "--no-open".to_string(),
            ]
        );
    }

    #[test]
    fn fatal_report_flags_precede_the_script_path() {
        let args = build_harness_args(
            Path::new("D:/core/lib/bin.js"),
            "tauri",
            None,
            3081,
            None,
            Path::new("D:/用户 data/logs/fatal.json"),
        );
        let text = text_args(&args);
        assert_eq!(text[0], "--report-on-fatalerror");
        assert_eq!(text[1], "--report-directory=D:/用户 data/logs");
        assert_eq!(text[2], "--report-filename=fatal.json");
        assert_eq!(text[3], "D:/core/lib/bin.js");
    }

    #[test]
    fn fatal_report_paths_are_distinct_for_each_spawn_attempt() {
        let log = Path::new("D:/用户 data/logs/dsh-web.log");
        let first = fatal_report_path(log);
        let second = fatal_report_path(log);
        assert_eq!(first.parent(), Some(Path::new("D:/用户 data/logs")));
        assert_eq!(first.extension(), Some(std::ffi::OsStr::new("json")));
        assert_ne!(first, second);
    }

    #[test]
    fn fatal_report_log_only_names_this_attempts_existing_file_without_reading_it() {
        let root = tempfile::tempdir().unwrap();
        let log = root.path().join("dsh-web.log");
        fs::write(&log, "existing error\n").unwrap();
        let stale = fatal_report_path(&log);
        fs::write(&stale, b"secret environment").unwrap();
        let report = fatal_report_path(&log);
        log_fatal_report(&log, &report);
        assert_eq!(fs::read_to_string(&log).unwrap(), "existing error\n");
        fs::create_dir(&report).unwrap();
        log_fatal_report(&log, &report);
        assert_eq!(fs::read_to_string(&log).unwrap(), "existing error\n");
        fs::remove_dir(&report).unwrap();
        fs::write(&report, b"\xffsecret environment").unwrap();
        log_fatal_report(&log, &report);
        assert_eq!(
            fs::read_to_string(&log).unwrap(),
            format!(
                "existing error\nERROR HARNESS_FATAL_REPORT: {}\n",
                report.display()
            )
        );
    }

    #[test]
    fn heap_option_stays_ahead_of_the_script_path() {
        let args = build_harness_args(
            Path::new("D:/core/lib/bin.js"),
            "tauri",
            None,
            3081,
            Some(4096),
            Path::new("D:/logs/fatal.json"),
        );
        let text = text_args(&args);
        assert_eq!(text[0], "--max-old-space-size=4096");
        assert_eq!(text[1], "--report-on-fatalerror");
        assert_eq!(text[4], "D:/core/lib/bin.js");
        assert!(!text.contains(&"--patch".to_string()));
    }

    struct SupervisedNode {
        root: tempfile::TempDir,
        instance: std::sync::Arc<super::super::instances::Instance>,
        started: Option<tokio::sync::oneshot::Receiver<Result<(), String>>>,
        finished: std::sync::mpsc::Receiver<Result<(), String>>,
        worker: Option<std::thread::JoinHandle<()>>,
        attempts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        spawned: std::sync::Arc<std::sync::Mutex<Option<OwnedProcess>>>,
        resets: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        transition: std::sync::Arc<tokio::sync::Mutex<()>>,
    }

    impl SupervisedNode {
        fn new(script: &str) -> Self {
            Self::with_reader_failure(script, 0)
        }

        fn with_reader_failure(script: &str, reader: usize) -> Self {
            use std::sync::{atomic::AtomicUsize, Arc, Mutex};

            let node = config::get_local_node_path()
                .expect("supported Node.js required for supervisor tests");
            let root = tempfile::tempdir().unwrap();
            let path = root.path().to_path_buf();
            let script_path = path.join("fixture.cjs");
            fs::write(&script_path, script).unwrap();
            fs::create_dir_all(path.join("AppData/Local")).unwrap();
            fs::create_dir_all(path.join("AppData/Roaming")).unwrap();
            let envs: HashMap<String, String> = HashMap::from([
                ("HOME".into(), path.to_string_lossy().into_owned()),
                ("USERPROFILE".into(), path.to_string_lossy().into_owned()),
                (
                    "APPDATA".into(),
                    path.join("AppData/Roaming").to_string_lossy().into_owned(),
                ),
                (
                    "LOCALAPPDATA".into(),
                    path.join("AppData/Local").to_string_lossy().into_owned(),
                ),
                ("DSH_E2E_HOME".into(), path.to_string_lossy().into_owned()),
                ("NODE_OPTIONS".into(), String::new()),
                ("NODE_PATH".into(), String::new()),
            ]);
            let profile = format!("supervisor-{}", path.file_name().unwrap().to_string_lossy());
            let instance = super::super::instances::instance_of(&profile);
            instance.launching.store(true, Ordering::SeqCst);
            *instance.status.lock().unwrap() = status::Status::Starting;
            let attempts = Arc::new(AtomicUsize::new(0));
            let resets = Arc::new(AtomicUsize::new(0));
            let transition = Arc::new(tokio::sync::Mutex::new(()));
            let guards = (
                LaunchGuard(instance.launching.clone()),
                transition.clone().try_lock_owned().unwrap(),
            );
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let (finished_tx, finished) = std::sync::mpsc::channel();
            let spawned = Arc::new(Mutex::new(None));
            let spawned_process = spawned.clone();
            let spawn_count = attempts.clone();
            let reset_count = resets.clone();
            let worker = std::thread::spawn(move || {
                let log = path.join("dsh-web.log");
                let mut started = Some(started_tx);
                let result = super::super::utils::with_output_reader_failure(reader, || {
                    super::super::instances::scope_sync(profile, || {
                        supervise_harness(
                            |pid, code, report_path| {
                                on_owned_process_exit_at(
                                    pid,
                                    |_| {
                                        if let Some(report_path) = report_path {
                                            log_fatal_report(&log, report_path);
                                        }
                                        code
                                    },
                                    |_| {},
                                );
                            },
                            &log,
                            &mut started,
                            guards,
                            |_| {
                                if spawn_count.fetch_add(1, Ordering::SeqCst) > 0
                                    && !fs::read_to_string(path.join("cordis.yml"))
                                        .unwrap()
                                        .ends_with("\n[]\n")
                                {
                                    return Err("FIXTURE_PROFILE_NOT_RESET: stale root".into());
                                }
                                if path.join("fail-next-spawn").exists() {
                                    return Err(
                                        "PROCESS_START_FAILED: fixture retry spawn failure".into(),
                                    );
                                }
                                fs::write(path.join("cordis.yml"), "- stale-loader-entry\n")
                                    .unwrap();
                                #[cfg(windows)]
                                {
                                    use std::os::windows::io::{FromRawHandle, OwnedHandle};
                                    let (stdout, stderr, pid, handle) =
                                        super::super::win_spawn::spawn_with_hidden_console_owned(
                                            &node,
                                            &[script_path.clone().into_os_string()],
                                            Some(&path),
                                            &envs,
                                        )
                                        .map_err(
                                            |error| format!("PROCESS_START_FAILED: {error}"),
                                        )?;
                                    let handle =
                                        Arc::new(unsafe { OwnedHandle::from_raw_handle(handle) });
                                    let owned = OwnedProcess {
                                        pid,
                                        handle: handle.clone(),
                                    };
                                    *spawned_process.lock().unwrap() = Some(owned.clone());
                                    let signature = spawn_output_readers(
                                        Some(stdout),
                                        Some(stderr),
                                        log.clone(),
                                    );
                                    Ok((owned, handle, signature))
                                }
                                #[cfg(unix)]
                                {
                                    use std::os::unix::process::CommandExt;
                                    let mut process = Command::new(&node);
                                    crate::config::apply_node_env(&mut process);
                                    let mut child = process
                                        .arg(&script_path)
                                        .envs(&envs)
                                        .current_dir(&path)
                                        .stdin(Stdio::null())
                                        .stdout(Stdio::piped())
                                        .stderr(Stdio::piped())
                                        .process_group(0)
                                        .spawn()
                                        .map_err(|error| {
                                            format!("PROCESS_START_FAILED: {error}")
                                        })?;
                                    let pid = child.id();
                                    *spawned_process.lock().unwrap() = Some(OwnedProcess { pid });
                                    let signature = spawn_output_readers(
                                        child.stdout.take(),
                                        child.stderr.take(),
                                        log.clone(),
                                    );
                                    Ok((OwnedProcess { pid }, child, signature))
                                }
                            },
                            || {
                                reset_count.fetch_add(1, Ordering::SeqCst);
                                reset_active_profile_root(&path);
                            },
                            |pid| fs::write(path.join("harness.pid"), pid.to_string()).unwrap(),
                        )
                    })
                });
                if let Some(started) = started.take() {
                    let _ = started.send(result.clone());
                }
                finished_tx.send(result).unwrap();
            });
            Self {
                root,
                instance,
                started: Some(started_rx),
                finished,
                worker: Some(worker),
                attempts,
                spawned,
                resets,
                transition,
            }
        }

        fn acknowledge(&mut self) {
            tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .unwrap()
                .block_on(async {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        self.started.take().unwrap(),
                    )
                    .await
                })
                .expect("spawn acknowledgement must not wait for the startup probe")
                .expect("supervisor must send acknowledgement")
                .expect("fixture must spawn and register");
        }

        fn wait_until(&self, check: impl Fn() -> bool) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
            while !check() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "supervisor fixture condition timed out"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        fn cancel(&self) -> Result<(), String> {
            let owned = {
                let mut slot = self
                    .instance
                    .process
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                self.instance.launching.store(false, Ordering::SeqCst);
                slot.take()
            };
            if let Some(owned) = owned {
                #[cfg(windows)]
                {
                    use std::os::windows::io::AsRawHandle;
                    super::super::process::terminate_windows_process(
                        owned.pid,
                        owned.handle.as_raw_handle(),
                        super::super::process::kill_pid_tree,
                    )?;
                }
                #[cfg(unix)]
                super::super::process::kill_pid_tree(owned.pid);
            }
            Ok(())
        }

        fn finish(&mut self) -> Result<(), String> {
            let result = self
                .finished
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("supervisor must finish and reap its child");
            self.worker.take().unwrap().join().unwrap();
            result
        }
    }

    impl Drop for SupervisedNode {
        fn drop(&mut self) {
            let _ = self.cancel();
            if let Some(worker) = self.worker.take().filter(|worker| worker.is_finished()) {
                let _ = worker.join();
            }
        }
    }

    fn assert_reader_failure_is_reaped(reader: usize) {
        let mut fixture = SupervisedNode::with_reader_failure("setTimeout(()=>{},60000)", reader);
        let error = fixture.finish().unwrap_err();
        assert_eq!(
            error,
            "PROCESS_START_FAILED: spawn output reader: injected output reader spawn failure"
        );
        assert_eq!(
            fixture.started.take().unwrap().blocking_recv().unwrap(),
            Err(error)
        );
        assert_eq!(fixture.attempts.load(Ordering::SeqCst), 1);
        assert!(fixture.instance.process.lock().unwrap().is_none());
        assert!(!fixture.root.path().join("harness.pid").exists());
        assert!(!fixture.instance.launching.load(Ordering::SeqCst));
        assert!(fixture.transition.try_lock().is_ok());
        let spawned = fixture.spawned.lock().unwrap();
        let owned = spawned.as_ref().expect("fixture must create a real child");
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            assert_eq!(
                unsafe {
                    windows_sys::Win32::System::Threading::WaitForSingleObject(
                        owned.handle.as_raw_handle(),
                        0,
                    )
                },
                windows_sys::Win32::Foundation::WAIT_OBJECT_0,
                "failed reader setup must confirm the exact process exited"
            );
        }
        #[cfg(unix)]
        {
            let mut status = 0;
            assert_eq!(unsafe { libc::kill(owned.pid as libc::pid_t, 0) }, -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
            assert_eq!(
                unsafe { libc::waitpid(owned.pid as libc::pid_t, &mut status, libc::WNOHANG) },
                -1
            );
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ECHILD)
            );
        }
    }

    #[test]
    fn supervisor_reaps_child_when_first_output_reader_spawn_fails() {
        assert_reader_failure_is_reaped(1);
    }

    #[test]
    fn supervisor_reaps_child_when_second_output_reader_spawn_fails() {
        assert_reader_failure_is_reaped(2);
    }

    #[test]
    fn supervisor_acknowledges_before_probe_and_drains_both_full_pipes() {
        let mut fixture = SupervisedNode::new(
            "const fs=require('fs'); const chunk='x'.repeat(1024)+'\\n'; for(let i=0;i<512;i++){fs.writeSync(1,'OUT:'+chunk);fs.writeSync(2,'ERR:'+chunk)} fs.writeSync(1,'STDOUT_DONE\\n');fs.writeSync(2,'STDERR_DONE\\n');setTimeout(()=>{},10000)",
        );
        fixture.acknowledge();
        let pid = fixture
            .instance
            .process
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .pid;
        assert_eq!(
            fs::read_to_string(fixture.root.path().join("harness.pid")).unwrap(),
            pid.to_string()
        );
        assert!(fixture.instance.launching.load(Ordering::SeqCst));
        assert!(fixture.transition.try_lock().is_err());
        let log = fixture.root.path().join("dsh-web.log");
        fixture.wait_until(|| {
            fs::read_to_string(&log)
                .is_ok_and(|text| text.contains("STDOUT_DONE") && text.contains("STDERR_DONE"))
        });
        let output = fs::read_to_string(log).unwrap();
        assert_eq!(output.matches("OUT:").count(), 512);
        assert_eq!(output.matches("ERR:").count(), 512);
        fixture.wait_until(|| {
            !fixture.instance.launching.load(Ordering::SeqCst)
                && fixture.transition.try_lock().is_ok()
        });
        assert!(fixture.instance.process.lock().unwrap().is_some());
        fixture.cancel().unwrap();
        fixture.finish().unwrap();
        assert!(fixture.instance.process.lock().unwrap().is_none());
    }

    #[test]
    fn supervisor_retries_only_duplicate_code_one_at_most_twice() {
        for (script, attempts, resets) in [
            ("require('fs').writeSync(2,'duplicate loader entry id: fixture\\n');process.exit(1)", 3, 2),
            ("require('fs').writeSync(2,'Error: EADDRINUSE\\n');process.exit(1)", 1, 0),
            ("require('fs').writeSync(2,'duplicate loader entry id: fixture\\n');process.exit(0)", 1, 0),
        ] {
            let mut fixture = SupervisedNode::new(script);
            fixture.acknowledge();
            fixture.finish().unwrap();
            assert_eq!(fixture.attempts.load(Ordering::SeqCst), attempts, "{script}");
            assert_eq!(fixture.resets.load(Ordering::SeqCst), resets, "{script}");
            assert!(fixture.instance.process.lock().unwrap().is_none());
            assert!(!fixture.instance.launching.load(Ordering::SeqCst));
            assert_eq!(*fixture.instance.status.lock().unwrap(), status::Status::Stopped);
            assert!(fixture.transition.try_lock().is_ok());
        }
    }

    #[test]
    fn supervisor_releases_ownership_and_guards_when_retry_spawn_fails() {
        let mut fixture = SupervisedNode::new(
            "const fs=require('fs');fs.writeFileSync('fail-next-spawn','1');fs.writeSync(2,'duplicate loader entry id: fixture\\n');process.exit(1)",
        );
        fixture.acknowledge();
        assert_eq!(
            fixture.finish().unwrap_err(),
            "PROCESS_START_FAILED: fixture retry spawn failure"
        );
        assert_eq!(fixture.attempts.load(Ordering::SeqCst), 2);
        assert_eq!(fixture.resets.load(Ordering::SeqCst), 1);
        assert!(fixture.instance.process.lock().unwrap().is_none());
        assert_eq!(
            *fixture.instance.status.lock().unwrap(),
            status::Status::Stopped
        );
        fixture.wait_until(|| {
            !fixture.instance.launching.load(Ordering::SeqCst)
                && fixture.transition.try_lock().is_ok()
        });
    }

    #[test]
    fn stopping_during_probe_cancels_retry_without_invalidating_monitor_handle() {
        let mut fixture = SupervisedNode::new(
            "require('fs').writeSync(2,'duplicate loader entry id: fixture\\n');setTimeout(()=>{},10000)",
        );
        fixture.acknowledge();
        let log = fixture.root.path().join("dsh-web.log");
        fixture.wait_until(|| {
            fs::read_to_string(&log).is_ok_and(|text| text.contains("duplicate loader entry"))
        });
        assert!(fixture.instance.launching.load(Ordering::SeqCst));
        fixture.cancel().unwrap();
        if let Err(error) = fixture.finish() {
            assert_eq!(error, "STARTUP_CANCELLED: Harness launch was stopped");
        }
        assert_eq!(fixture.attempts.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.resets.load(Ordering::SeqCst), 0);
        assert!(fixture.instance.process.lock().unwrap().is_none());
        assert!(!fixture.instance.launching.load(Ordering::SeqCst));
        assert!(fixture.transition.try_lock().is_ok());
    }

    /// 端口自愈（issue #91）：自动避让递增遗留的非默认端口，在回落目标空闲时
    #[test]
    fn heal_port_returns_target_when_free() {
        assert_eq!(resolve_heal_port(3084, 3080, true), 3080);
        assert_eq!(resolve_heal_port(9091, 9090, true), 9090);
    }

    #[test]
    fn heal_port_keeps_current_when_target_busy_or_aligned() {
        assert_eq!(resolve_heal_port(3084, 3080, false), 3084);
        assert_eq!(resolve_heal_port(3080, 3080, true), 3080);
        assert_eq!(resolve_heal_port(3080, 3080, false), 3080);
    }
}
