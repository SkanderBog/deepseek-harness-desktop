
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};

use super::cancel::terminate_owned_install;
use super::install::install_internal;
use super::installed::{installed_name, profile_dir, ProfilePackageJson};
use super::preset::{
    bundled_dep_spec, bundled_plugin_dir, bundled_plugin_dir_from, load_presets,
    presets_from_manifest, PreinstallPluginInfo,
};
use super::process::{new_process_owner, PreinstallLogPayload, ProcessOwner, PREINSTALL_LOG_EVENT};
use crate::config;

use manifest::{
    collect_dangling_local_link_deps, dedupe_profile_bundles, dep_matches_spec,
    internal_plugin_entry_is_ready, is_local_link_dep, remove_duplicate_bundle_entries_from_patch,
    remove_internal_plugins_from_manifest, remove_stale_plugin_entry, write_profile_manifest,
};
use materialize::{materialize_links, OfflineLink};

mod manifest;
mod materialize;
use crate::{AppHandle};

#[derive(serde::Serialize, Clone)]
struct InternalPluginsPhase {
    phase: &'static str,
    detail: InternalPluginPhaseDetail,
    completed: usize,
    total: usize,
}

#[derive(serde::Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum InternalPluginPhaseDetail {
    Waiting,
    Checking,
    Installing,
    Heartbeat,
    Done,
    Timeout,
    Cancelled,
}

const ENSURE_ABSOLUTE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);
const ENSURE_CLEANUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
const ENSURE_OWNER_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
const ENSURE_OWNER_DRAIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
/// 复用「刚刚成功过」的核对结论的时间窗（issue #766）。
const ENSURE_REUSE_WINDOW: std::time::Duration = std::time::Duration::from_secs(30);

/// 最近一次成功核对的输入指纹与完成时刻（issue #766）。
#[derive(Clone)]
struct EnsureReceipt {
    fingerprint: u64,
    completed_at: std::time::Instant,
}

#[derive(Clone)]
struct EnsureFlight {
    id: u64,
    owner: ProcessOwner,
    state: EnsureFlightState,
    result: tokio::sync::watch::Receiver<Option<Result<(), String>>>,
    cancel: tokio::sync::watch::Sender<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EnsureFlightState {
    Running,
    Cancelling,
    CleanupFailed(String),
}

enum EnsureSubscription {
    Running(tokio::sync::watch::Receiver<Option<Result<(), String>>>),
    Cancelling(tokio::sync::watch::Receiver<Option<Result<(), String>>>),
    CleanupFailed(String),
}

#[derive(Default)]
struct EnsureCoordinator {
    next_id: u64,
    active: Option<EnsureFlight>,
    receipt: Option<EnsureReceipt>,
}

impl EnsureCoordinator {
    fn reuse(&self, fingerprint: u64) -> bool {
        self.receipt.as_ref().is_some_and(|receipt| {
            receipt.fingerprint == fingerprint
                && receipt.completed_at.elapsed() <= ENSURE_REUSE_WINDOW
        })
    }

    fn record(&mut self, fingerprint: u64) {
        self.receipt = Some(EnsureReceipt {
            fingerprint,
            completed_at: std::time::Instant::now(),
        });
    }

    fn subscribe(&self) -> Option<EnsureSubscription> {
        self.active.as_ref().map(|flight| match &flight.state {
            EnsureFlightState::Running => EnsureSubscription::Running(flight.result.clone()),
            EnsureFlightState::Cancelling => EnsureSubscription::Cancelling(flight.result.clone()),
            EnsureFlightState::CleanupFailed(reason) => {
                EnsureSubscription::CleanupFailed(reason.clone())
            }
        })
    }

    #[allow(clippy::type_complexity)]
    fn start(
        &mut self,
    ) -> (
        u64,
        ProcessOwner,
        tokio::sync::watch::Sender<Option<Result<(), String>>>,
        tokio::sync::watch::Receiver<Option<Result<(), String>>>,
        tokio::sync::watch::Sender<bool>,
        tokio::sync::watch::Receiver<bool>,
    ) {
        self.next_id = self.next_id.wrapping_add(1);
        let id = self.next_id;
        let owner = new_process_owner();
        let (result_tx, result_rx) = tokio::sync::watch::channel(None);
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        self.active = Some(EnsureFlight {
            id,
            owner,
            state: EnsureFlightState::Running,
            result: result_rx.clone(),
            cancel: cancel_tx.clone(),
        });
        (id, owner, result_tx, result_rx, cancel_tx, cancel_rx)
    }

    fn finish(&mut self, id: u64) {
        if self.active.as_ref().is_some_and(|flight| flight.id == id) {
            self.active = None;
        }
    }

    fn begin_cancel(&mut self) -> Option<tokio::sync::watch::Receiver<Option<Result<(), String>>>> {
        let active = self.active.as_mut()?;
        if !matches!(&active.state, EnsureFlightState::CleanupFailed(_)) {
            active.state = EnsureFlightState::Cancelling;
            let _ = active.cancel.send(true);
        }
        Some(active.result.clone())
    }

    fn mark_cleanup_failed(&mut self, id: u64, reason: String) {
        if let Some(active) = self.active.as_mut().filter(|flight| flight.id == id) {
            active.state = EnsureFlightState::CleanupFailed(reason);
        }
    }
}

fn ensure_lock(profile: &str) -> std::sync::Arc<tokio::sync::Mutex<EnsureCoordinator>> {
    static COORDINATORS: std::sync::OnceLock<
        std::sync::Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<EnsureCoordinator>>>>,
    > = std::sync::OnceLock::new();
    COORDINATORS
        .get_or_init(std::sync::Mutex::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .entry(profile.to_string())
        .or_default()
        .clone()
}

fn repair_loader_state(
    profile: &Path,
    dsh_home: &Path,
    internal: &[&PreinstallPluginInfo],
    manifest: &mut Option<serde_json::Value>,
    prepare: bool,
) -> Result<bool, String> {
    let bundle_ids: HashSet<&str> = internal.iter().map(|preset| preset.id.as_str()).collect();
    let mut pending = false;
    if let Some(manifest) = manifest.as_mut() {
        if dedupe_profile_bundles(manifest) {
            if prepare {
                write_profile_manifest(&profile.join("package.json"), manifest)?;
            } else {
                pending = true;
            }
        }
    }

    for patch_path in super::patch_layer_paths(profile, dsh_home) {
        let raw = match std::fs::read_to_string(&patch_path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                let code = if error.kind() == std::io::ErrorKind::InvalidData {
                    super::patch_guard::PATCH_ENCODING_FAILED
                } else {
                    "INTERNAL_PLUGIN_PATCH_READ_FAILED"
                };
                return Err(format!("{code}: {}: {error}", patch_path.display()));
            }
        };
        let mut patch = super::parse_patch_layer(&raw).map_err(|e| {
            format!(
                "INTERNAL_PLUGIN_PATCH_PARSE_FAILED: {}: {e}",
                patch_path.display()
            )
        })?;
        if remove_duplicate_bundle_entries_from_patch(&mut patch, &bundle_ids) {
            if !prepare {
                pending = true;
                continue;
            }
            let rendered = super::render_patch_layer(&patch)
                .map_err(|e| format!("INTERNAL_PLUGIN_PATCH_RENDER_FAILED: {e}"))?;
            super::atomic_write::write_patch_layer_atomically(&patch_path, &rendered).map_err(
                |e| {
                    format!(
                        "INTERNAL_PLUGIN_PATCH_WRITE_FAILED: {}: {e}",
                        patch_path.display()
                    )
                },
            )?;
            log::warn!(
                "INTERNAL_PLUGIN_PROFILE_MIGRATED: removed stale internal loader entries from {}",
                patch_path.display()
            );
        }
    }

    if !prepare {
        return Ok(pending);
    }

    let root = profile.join("cordis.yml");
    const EMPTY_ROOT: &str = "# dsh profile root — an empty entry list. The tree is composed as patches:\n# each bundle in package.json's dsh.profile.bundles, then cordis.patch.yml, then any\n# --patch overlays. Edit cordis.patch.yml, not this file.\n[]\n";
    let existing = match std::fs::read_to_string(&root) {
        Ok(raw) => Some(raw),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "INTERNAL_PLUGIN_ROOT_READ_FAILED: {}: {error}",
                root.display()
            ))
        }
    };
    if existing.as_deref() != Some(EMPTY_ROOT) && profile.is_dir() {
        std::fs::write(&root, EMPTY_ROOT)
            .map_err(|e| format!("INTERNAL_PLUGIN_ROOT_WRITE_FAILED: {}: {e}", root.display()))?;
        log::warn!(
            "INTERNAL_PLUGIN_PROFILE_MIGRATED: reset stale profile root {}",
            root.display()
        );
    }
    Ok(false)
}

fn ensure_fingerprint(app_handle: &AppHandle, internal: &[PreinstallPluginInfo]) -> u64 {
    use std::hash::{Hash, Hasher};

    let profile = profile_dir(app_handle);
    let dsh_home = config::get_dsh_data_path(app_handle);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    profile.to_string_lossy().as_ref().hash(&mut hasher);
    std::fs::read(profile.join("package.json"))
        .unwrap_or_default()
        .hash(&mut hasher);
    std::fs::read(profile.join("cordis.yml"))
        .unwrap_or_default()
        .hash(&mut hasher);
    for patch_path in super::patch_layer_paths(&profile, &dsh_home) {
        std::fs::read(patch_path)
            .unwrap_or_default()
            .hash(&mut hasher);
    }
    crate::service::core::active_version(app_handle).hash(&mut hasher);
    let mut specs: Vec<(&str, &str, &str)> = internal
        .iter()
        .map(|preset| {
            (
                preset.id.as_str(),
                preset.spec.as_str(),
                installed_name(preset),
            )
        })
        .collect();
    specs.sort_unstable();
    specs.hash(&mut hasher);
    hasher.finish()
}

pub(crate) async fn ensure(app_handle: &AppHandle) -> Result<(), String> {
    let profile = crate::service::profile::active_profile(app_handle);
    crate::service::workflow::instances::scope(profile, ensure_scoped(app_handle)).await
}

async fn ensure_scoped(app_handle: &AppHandle) -> Result<(), String> {
    crate::service::workflow::early::wait_preparation().await?;
    let lock = ensure_lock(&crate::service::profile::active_profile(app_handle));
    let presets = load_presets(app_handle);
    let core_version = crate::service::core::active_version(app_handle);
    let internal: Vec<_> = presets
        .into_iter()
        .filter(|p| p.internal && !p.unsupported_on(core_version.as_deref()))
        .collect();
    if crate::service::workflow::has_owned_process() {
        if config::manifest::read(app_handle).is_none() {
            return Err("INTERNAL_PLUGIN_RESTART_REQUIRED: 内置插件清单不可读；请恢复应用资源后通过正常启动重试。".into());
        }
        let refs: Vec<_> = internal.iter().collect();
        let profile = profile_dir(app_handle);
        let check = prepare_internal_plugins(
            &profile,
            &config::get_dsh_data_path(app_handle),
            &refs,
            false,
            |id| bundled_plugin_dir(app_handle, id),
        )?;
        return require_running_plugins_ready(&profile, &check);
    }
    // 连下面的失效链接清理、可写性预检与 loader 状态修复一起跳过（issue #766）。
    let fingerprint = ensure_fingerprint(app_handle, &internal);
    let reused = if internal.is_empty() {
        false
    } else {
        lock.lock().await.reuse(fingerprint)
    };
    if reused {
        log::debug!(
            "Internal plugin check already succeeded for the current profile state, skipping"
        );
        return Ok(());
    }
    let outcome =
        receive_current_or_next_flight(|| subscribe_or_start(app_handle, &internal)).await;
    if outcome.is_ok() {
        lock.lock()
            .await
            .record(ensure_fingerprint(app_handle, &internal));
    }
    outcome
}

async fn subscribe_or_start(
    app_handle: &AppHandle,
    internal: &[PreinstallPluginInfo],
) -> EnsureSubscription {
    let profile = crate::service::profile::active_profile(app_handle);
    let lock = ensure_lock(&profile);
    let mut coordinator = lock.lock().await;
    match coordinator.subscribe() {
        Some(subscription) => subscription,
        None => {
            let (id, owner, result_tx, result_rx, cancel_tx, cancel_rx) = coordinator.start();
            let app_handle = app_handle.clone();
            let internal = internal.to_vec();
            let lock = lock.clone();
            tauri::async_runtime::spawn(crate::service::workflow::instances::scope(
                profile,
                async move {
                    let mut outcome =
                        run_ensure_operation(&app_handle, &internal, owner, cancel_tx, cancel_rx)
                            .await;

                    if !wait_for_owner_release(owner, ENSURE_OWNER_DRAIN_TIMEOUT).await {
                        let reason = format!(
                        "INTERNAL_PLUGIN_PROCESS_REAP_TIMEOUT: plugin process owner {owner:?} remained active for {} seconds",
                        ENSURE_OWNER_DRAIN_TIMEOUT.as_secs()
                    );
                        log::error!("{reason}");
                        outcome = Err(reason.clone());
                        let _ = result_tx.send(Some(outcome));
                        lock.lock().await.mark_cleanup_failed(id, reason);
                        while super::process::active_plugin_pid(owner).is_some() {
                            tokio::time::sleep(ENSURE_OWNER_DRAIN_INTERVAL).await;
                        }
                        lock.lock().await.finish(id);
                        return;
                    }
                    lock.lock().await.finish(id);
                    let _ = result_tx.send(Some(outcome));
                },
            ));
            EnsureSubscription::Running(result_rx)
        }
    }
}

async fn receive_current_or_next_flight<F, Fut>(mut acquire: F) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = EnsureSubscription>,
{
    loop {
        let subscription = acquire().await;
        match subscription {
            EnsureSubscription::Running(mut result) => {
                return receive_flight_result(&mut result).await?;
            }
            EnsureSubscription::Cancelling(mut result) => {
                let _ = receive_flight_result(&mut result).await?;
            }
            EnsureSubscription::CleanupFailed(reason) => return Err(reason),
        }
    }
}

async fn wait_for_owner_release(
    owner: super::process::ProcessOwner,
    timeout: std::time::Duration,
) -> bool {
    wait_for_release_with(
        || super::process::active_plugin_pid(owner).is_some(),
        timeout,
        ENSURE_OWNER_DRAIN_INTERVAL,
    )
    .await
}

async fn wait_for_release_with<F>(
    mut is_active: F,
    timeout: std::time::Duration,
    interval: std::time::Duration,
) -> bool
where
    F: FnMut() -> bool,
{
    let started = tokio::time::Instant::now();
    loop {
        if !is_active() {
            return true;
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            return false;
        }
        tokio::time::sleep(interval.min(timeout - elapsed)).await;
    }
}

async fn receive_flight_result(
    result: &mut tokio::sync::watch::Receiver<Option<Result<(), String>>>,
) -> Result<Result<(), String>, String> {
    loop {
        if let Some(outcome) = result.borrow().clone() {
            return Ok(outcome);
        }
        result.changed().await.map_err(|_| {
            "INTERNAL_PLUGIN_ENSURE_DROPPED: install task ended without result".to_string()
        })?;
    }
}

pub(crate) async fn cancel() -> Result<(), String> {
    let mut result = {
        let profile =
            crate::service::workflow::instances::current_profile().unwrap_or_else(|| "web".into());
        let lock = ensure_lock(&profile);
        let mut coordinator = lock.lock().await;
        let Some(active) = &coordinator.active else {
            return Ok(());
        };
        log::info!(
            "cancelling internal plugin ensure flight owned by {:?}",
            active.owner
        );
        coordinator
            .begin_cancel()
            .expect("active flight must remain present while coordinator lock is held")
    };
    match receive_flight_result(&mut result).await? {
        Err(reason) if reason.starts_with("INTERNAL_PLUGIN_PROCESS_REAP_TIMEOUT:") => Err(reason),
        _ => Ok(()),
    }
}

async fn run_ensure_operation(
    app_handle: &AppHandle,
    internal: &[PreinstallPluginInfo],
    owner: ProcessOwner,
    cancel_tx: tokio::sync::watch::Sender<bool>,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<(), String> {
    let total = internal.len();
    emit_phase(
        app_handle,
        "loading",
        InternalPluginPhaseDetail::Waiting,
        0,
        total,
    );
    emit_phase(
        app_handle,
        "progress",
        InternalPluginPhaseDetail::Checking,
        0,
        total,
    );
    let refs: Vec<_> = internal.iter().collect();
    let operation = ensure_inner(app_handle, &refs, cancel.clone(), owner);
    tokio::pin!(operation);
    let deadline = tokio::time::sleep(ENSURE_ABSOLUTE_TIMEOUT);
    tokio::pin!(deadline);
    let period = std::time::Duration::from_secs(5);
    let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            outcome = &mut operation => {
                emit_phase(app_handle, "done", InternalPluginPhaseDetail::Done, total, total);
                return outcome;
            }
            _ = &mut deadline => {
                let reason = "INTERNAL_PLUGIN_INSTALL_TIMEOUT: plugin install exceeded 600 seconds";
                log::error!("{reason}");
                let _ = cancel_tx.send(true);
                terminate_owned_install(owner).await;
                if tokio::time::timeout(ENSURE_CLEANUP_TIMEOUT, &mut operation).await.is_err() {
                    log::error!("INTERNAL_PLUGIN_CLEANUP_TIMEOUT: plugin process did not exit after forced termination");
                }
                emit_phase(app_handle, "done", InternalPluginPhaseDetail::Timeout, 0, total);
                return Err(reason.to_string());
            }
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    let reason = "INTERNAL_PLUGIN_INSTALL_CANCELLED: plugin install was cancelled";
                    log::warn!("{reason}");
                    terminate_owned_install(owner).await;
                    if tokio::time::timeout(ENSURE_CLEANUP_TIMEOUT, &mut operation).await.is_err() {
                        log::error!("INTERNAL_PLUGIN_CLEANUP_TIMEOUT: plugin process did not exit after forced termination");
                    }
                    emit_phase(app_handle, "done", InternalPluginPhaseDetail::Cancelled, 0, total);
                    return Err(reason.to_string());
                }
            }
            _ = heartbeat.tick() => {
                emit_phase(app_handle, "progress", InternalPluginPhaseDetail::Heartbeat, 0, total);
            }
        }
    }
}

fn emit_phase(
    app_handle: &AppHandle,
    phase: &'static str,
    detail: InternalPluginPhaseDetail,
    completed: usize,
    total: usize,
) {
    crate::service::workflow::instances::emit(
        app_handle,
        "internal-plugins-phase",
        InternalPluginsPhase {
            phase,
            detail,
            completed,
            total,
        },
    );
}

struct InternalPluginCheck {
    manifest: Option<serde_json::Value>,
    parsed: Option<ProfilePackageJson>,
    need: Vec<(String, String, PathBuf)>,
    orphans: Vec<String>,
    sources_ready: bool,
    migration_pending: bool,
}

fn prepare_internal_plugins(
    profile: &Path,
    dsh_home: &Path,
    internal: &[&PreinstallPluginInfo],
    prepare: bool,
    resolve_bundle: impl Fn(&str) -> Option<PathBuf>,
) -> Result<InternalPluginCheck, String> {
    let manifest_path = profile.join("package.json");
    let mut manifest = match std::fs::read_to_string(&manifest_path) {
        Ok(raw) => Some(
            serde_json::from_str::<serde_json::Value>(&raw)
                .map_err(|e| format!("INTERNAL_PLUGIN_MANIFEST_PARSE_FAILED: {e}"))?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("INTERNAL_PLUGIN_MANIFEST_READ_FAILED: {e}")),
    };
    let parsed = manifest
        .as_ref()
        .map(|value| serde_json::from_value::<ProfilePackageJson>(value.clone()))
        .transpose()
        .map_err(|error| format!("INTERNAL_PLUGIN_MANIFEST_SCHEMA_FAILED: {error}"))?;
    if prepare {
        crate::service::perm::ensure_writable_path(profile, dsh_home, "PROFILE_NOT_WRITABLE")?;
    }
    let migration_pending =
        repair_loader_state(profile, dsh_home, internal, &mut manifest, prepare)?;
    let empty_dependencies = HashMap::new();
    let dependencies = parsed
        .as_ref()
        .map(|manifest| &manifest.dependencies)
        .unwrap_or(&empty_dependencies);
    let keep: HashSet<&str> = internal
        .iter()
        .flat_map(|preset| [preset.id.as_str(), installed_name(preset)])
        .collect();
    let mut orphans = manifest
        .as_ref()
        .map(|manifest| collect_dangling_local_link_deps(manifest, &keep, profile))
        .unwrap_or_default();
    let mut need = Vec::new();
    let mut sources_ready = true;
    for preset in internal {
        let name = installed_name(preset);
        if super::disable::has_patch_disable(&profile, &preset.id)
            || super::disable::has_patch_disable(&profile, name)
        {
            continue;
        }
        let Some(bundled) = resolve_bundle(&preset.id) else {
            sources_ready = false;
            let name = installed_name(preset).to_string();
            let is_orphan = dependencies
                .get(&name)
                .is_some_and(|spec| is_local_link_dep(spec));
            if is_orphan {
                log::warn!(
                    "INTERNAL_PLUGIN_BUNDLE_MISSING_UNINSTALLING: {name}（link 指向的捆绑目录已不存在，卸载悬空内置插件）"
                );
                orphans.push(name);
            } else {
                log::warn!(
                    "INTERNAL_PLUGIN_BUNDLE_MISSING: {}（release 需构建期 build:plugins；debug 无匹配的 packages/* 包）",
                    preset.id
                );
            }
            continue;
        };
        let name = installed_name(preset).to_string();
        if !internal_plugin_entry_is_ready(&bundled) {
            return Err(format!(
                "INTERNAL_PLUGIN_SOURCE_MISSING: 内置插件 {name} 的捆绑源目录 {} 缺失或不可读（应用资源目录可能被移动/删除或盘符变更）。pnpm 会静默跳过安装（exit 0 无产物），导致服务启动时 loader 抛 ERR_MODULE_NOT_FOUND。请重新安装应用或恢复应用资源目录后重试。",
                bundled.display()
            ));
        }
        let expected = bundled_dep_spec(&bundled);
        let dep_ok = dependencies
            .get(&name)
            .is_some_and(|actual| dep_matches_spec(actual, &expected));
        let entry = profile.join("node_modules").join(&name);
        let link_ok = internal_plugin_entry_is_ready(&entry);
        let bundle_ok = manifest
            .as_ref()
            .and_then(|value| value.pointer("/dsh/profile/bundles"))
            .and_then(|bundles| bundles.as_array())
            .is_some_and(|bundles| {
                bundles
                    .iter()
                    .any(|bundle| bundle.as_str() == Some(name.as_str()))
            });
        if !dep_ok || !link_ok || !bundle_ok {
            log::info!(
                "INTERNAL_PLUGIN_NEEDS_REINSTALL: {name}（dep_ok={dep_ok}, link_ok={link_ok}, bundle_ok={bundle_ok}, expected={expected}）"
            );
            need.push((preset.id.clone(), name, entry));
        }
    }
    orphans.sort();
    orphans.dedup();
    Ok(InternalPluginCheck {
        manifest,
        parsed,
        need,
        orphans,
        sources_ready,
        migration_pending,
    })
}

fn require_running_plugins_ready(
    profile: &Path,
    check: &InternalPluginCheck,
) -> Result<(), String> {
    let fallback = profile
        .join(".dsh-module-fallback")
        .try_exists()
        .map_err(|error| format!("INTERNAL_PLUGIN_FALLBACK_READ_FAILED: {error}"))?;
    if check.manifest.is_none()
        || !check.sources_ready
        || !check.need.is_empty()
        || !check.orphans.is_empty()
        || check.migration_pending
        || fallback
    {
        return Err(format!(
            "INTERNAL_PLUGIN_RESTART_REQUIRED: 内置插件需要修复；请停止当前服务后通过正常启动重试（need={:?}, orphans={:?}, migration={}, sources_ready={}, fallback={}）。",
            check.need.iter().map(|(id, _, _)| id.as_str()).collect::<Vec<_>>(),
            check.orphans,
            check.migration_pending,
            check.sources_ready,
            fallback,
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EarlyPluginState {
    Ready,
    Defer,
}

pub(crate) fn prepare_early_plugins(
    profile: &Path,
    dsh_home: &Path,
    resources: Option<&Path>,
    manifest: Option<&config::manifest::Manifest>,
    core_version: Option<&str>,
) -> Result<EarlyPluginState, String> {
    let Some(manifest) = manifest.filter(|manifest| !manifest.plugins.built_in.is_empty()) else {
        return Ok(EarlyPluginState::Defer);
    };
    let presets = presets_from_manifest(manifest, resources);
    prepare_early_plugins_from(profile, dsh_home, &presets, core_version, |id| {
        bundled_plugin_dir_from(resources, id)
    })
}

fn prepare_early_plugins_from(
    profile: &Path,
    dsh_home: &Path,
    presets: &[PreinstallPluginInfo],
    core_version: Option<&str>,
    resolve_bundle: impl Fn(&str) -> Option<PathBuf>,
) -> Result<EarlyPluginState, String> {
    if presets.is_empty()
        || presets.iter().any(|preset| {
            matches!(
                preset.version,
                Some(config::manifest::PluginVersion::Matrix(_))
            )
        }) && !core_version.is_some_and(|version| semver::Version::parse(version).is_ok())
    {
        return Ok(EarlyPluginState::Defer);
    }
    let internal: Vec<_> = presets
        .iter()
        .filter(|preset| preset.internal && !preset.unsupported_on(core_version))
        .collect();
    let check = match prepare_internal_plugins(profile, dsh_home, &internal, true, resolve_bundle) {
        Ok(check) => check,
        Err(error) if error.starts_with("INTERNAL_PLUGIN_SOURCE_MISSING:") => {
            log::warn!("{error}");
            return Ok(EarlyPluginState::Defer);
        }
        Err(error) => return Err(error),
    };
    let Some(parsed) = check.parsed.as_ref() else {
        return Ok(EarlyPluginState::Defer);
    };
    if !check.sources_ready
        || !check.need.is_empty()
        || !check.orphans.is_empty()
        || profile
            .join(".dsh-module-fallback")
            .try_exists()
            .map_err(|error| format!("INTERNAL_PLUGIN_FALLBACK_READ_FAILED: {error}"))?
    {
        return Ok(EarlyPluginState::Defer);
    }
    let bundles = parsed
        .dsh
        .as_ref()
        .and_then(|dsh| dsh.profile.as_ref())
        .map(|profile| profile.bundles.as_slice())
        .unwrap_or_default();
    if !super::verify::missing_plugin_ids(
        &presets,
        &parsed.dependencies,
        bundles,
        &profile.join("node_modules"),
    )
    .is_empty()
    {
        return Ok(EarlyPluginState::Defer);
    }
    Ok(EarlyPluginState::Ready)
}

async fn ensure_inner(
    app_handle: &AppHandle,
    internal: &[&PreinstallPluginInfo],
    cancel: tokio::sync::watch::Receiver<bool>,
    owner: ProcessOwner,
) -> Result<(), String> {
    let profile = profile_dir(app_handle);
    let manifest_path = profile.join("package.json");
    let InternalPluginCheck {
        mut manifest,
        need,
        orphans,
        ..
    } = prepare_internal_plugins(
        &profile,
        &config::get_dsh_data_path(app_handle),
        internal,
        true,
        |id| bundled_plugin_dir(app_handle, id),
    )?;
    if !orphans.is_empty() {
        log::info!("Uninstalling orphaned internal preset plugins: {orphans:?}");
        for name in &orphans {
            if !super::recovery::is_actionable_plugin_ref(name) {
                log::warn!("INTERNAL_PLUGIN_ORPHAN_SKIPPED: {name}（核心/官方包不执行孤儿卸载）");
                continue;
            }
            match super::uninstall_recovery(app_handle, name) {
                Ok(()) => {
                    if let Some(value) = manifest.as_mut() {
                        remove_internal_plugins_from_manifest(
                            value,
                            &HashSet::from([name.as_str()]),
                        );
                    }
                }
                Err(e) => log::warn!("INTERNAL_PLUGIN_ORPHAN_UNINSTALL_FAILED: {name}: {e}"),
            }
        }
    }
    // best-effort（issue #466）：这只是旧目录的 housekeeping，失败绝不能中止本轮
    remove_legacy_profile_module_fallback_best_effort(&profile);
    if need.is_empty() {
        return Ok(());
    }

    let ids: Vec<String> = need.iter().map(|(id, _, _)| id.clone()).collect();
    log::info!("Reinstalling internal preset plugins: {ids:?}");
    emit_phase(
        app_handle,
        "progress",
        InternalPluginPhaseDetail::Installing,
        0,
        ids.len(),
    );

    if let Some(value) = manifest.as_mut() {
        let names: HashSet<&str> = need.iter().map(|(_, name, _)| name.as_str()).collect();
        if remove_internal_plugins_from_manifest(value, &names) {
            write_profile_manifest(&manifest_path, value)?;
        }
    }

    // （issue #264），一处失败即令整个安装放弃、`link:` 依赖不落盘，下次启动又判定
    let mut entries = HashSet::new();
    for (_, _, entry) in &need {
        if !entries.insert(entry.clone()) {
            continue;
        }
        if internal_plugin_entry_is_ready(entry) {
            log::info!(
                "INTERNAL_PLUGIN_ENTRY_HEALTHY: keeping intact link {} (no delete + recreate)",
                entry.display()
            );
            continue;
        }
        remove_stale_plugin_entry(entry).map_err(|e| {
            format!(
                "INTERNAL_PLUGIN_STALE_ENTRY_REMOVE_FAILED: {}: {e}",
                entry.display()
            )
        })?;
    }
    let install_result = install_internal(app_handle, &ids, cancel, owner).await;
    if let Err(e) = install_result {
        remove_legacy_profile_module_fallback_best_effort(&profile);
        if is_link_materialization_failure(&e, &need) {
            crate::service::workflow::instances::emit(
                app_handle,
                PREINSTALL_LOG_EVENT,
                PreinstallLogPayload {
                    line: "[harness] pnpm 未把插件链接落盘，改由应用直接建立链接…".to_string(),
                },
            );
            match materialize_internal_links(app_handle, &profile, &need) {
                Ok(()) => {
                    log::warn!(
                        "INTERNAL_PLUGIN_OFFLINE_LINK_RECOVERED: internal plugin links materialized after pnpm failure: {e}"
                    );
                    return Ok(());
                }
                Err(fallback) => {
                    log::error!("INTERNAL_PLUGIN_OFFLINE_LINK_FAILED: {fallback}");
                    return Err(format!(
                        "INTERNAL_PLUGIN_INSTALL_FAILED: {e}（离线自建链接兜底失败：{fallback}）\n{}",
                        link_readback_hint(&profile)
                    ));
                }
            }
        }
        if is_untraceable_link_readback_failure(&e) {
            log::warn!("INTERNAL_PLUGIN_OFFLINE_LINK_UNATTRIBUTED: {e}");
            return Err(format!(
                "INTERNAL_PLUGIN_INSTALL_FAILED: {e}\n{}",
                link_readback_hint(&profile)
            ));
        }
        return Err(format!("INTERNAL_PLUGIN_INSTALL_FAILED: {e}"));
    }

    let missing_manifest = presets_missing_from_manifest(&profile, &need);
    let unlinked = unlinked_internal_entries(&need);
    if missing_manifest.is_some() || !unlinked.is_empty() {
        materialize_internal_links(app_handle, &profile, &need)
            .map_err(|error| format!("INTERNAL_PLUGIN_MANIFEST_RECOVER_FAILED: {error}"))?;
        log::warn!(
            "INTERNAL_PLUGIN_MANIFEST_RECOVERED: repaired internal plugins after install (missing_manifest={missing_manifest:?}, unlinked={unlinked:?})"
        );
    }

    Ok(())
}

fn unlinked_internal_entries(need: &[(String, String, PathBuf)]) -> Vec<String> {
    need.iter()
        .filter(|(_, _, entry)| !internal_plugin_entry_is_ready(entry))
        .map(|(_, name, _)| name.clone())
        .collect()
}

fn presets_missing_from_manifest(
    profile: &Path,
    need: &[(String, String, PathBuf)],
) -> Option<Vec<String>> {
    let manifest = std::fs::read_to_string(profile.join("package.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
    let dependencies = manifest
        .as_ref()
        .and_then(|value| value.get("dependencies"));
    let bundles = manifest
        .as_ref()
        .and_then(|value| value.pointer("/dsh/profile/bundles"))
        .and_then(|value| value.as_array());
    let missing: Vec<String> = need
        .iter()
        .filter(|(_, name, _)| {
            let dep_ok = dependencies
                .and_then(|deps| deps.get(name.as_str()))
                .and_then(|value| value.as_str())
                .is_some();
            let bundle_ok = bundles.is_some_and(|list| {
                list.iter()
                    .any(|bundle| bundle.as_str() == Some(name.as_str()))
            });
            !dep_ok || !bundle_ok
        })
        .map(|(_, name, _)| name.clone())
        .collect();
    (!missing.is_empty()).then_some(missing)
}

fn link_readback_hint(profile: &Path) -> String {
    format!(
        "提示：若本机安全软件（实时防护 / EDR / 云盘同步）拦截了档案目录的目录链接，\
         请把 {} 加入排除项后重试。",
        profile.join("node_modules").display()
    )
}

fn looks_like_link_readback_failure(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("preinstall_failed")
        && lower.contains("node_modules")
        && (lower.contains("code -4094") || lower.contains("unknown error, open"))
}

fn is_untraceable_link_readback_failure(error: &str) -> bool {
    failed_open_path(error).is_none() && looks_like_link_readback_failure(error)
}

fn is_link_materialization_failure(error: &str, need: &[(String, String, PathBuf)]) -> bool {
    if error
        .to_ascii_lowercase()
        .contains("preinstall_silent_fail")
    {
        return true;
    }
    if names_unresolved_internal_bundle(error, need) {
        return true;
    }
    if !looks_like_link_readback_failure(error) {
        return false;
    }
    match failed_open_path(error) {
        Some(path) => need
            .iter()
            .any(|(_, _, entry)| path_lives_under(&path, entry)),
        None => false,
    }
}

fn names_unresolved_internal_bundle(error: &str, need: &[(String, String, PathBuf)]) -> bool {
    if !error.contains("cannot resolve profile bundle") {
        return false;
    }
    need.iter().any(|(_, name, _)| {
        error.contains(&format!("cannot resolve profile bundle \"{name}\""))
            || error.contains(&format!("cannot resolve profile bundle '{name}'"))
    })
}

fn failed_open_path(error: &str) -> Option<String> {
    const MARKER: &str = "unknown error, open ";
    let lower = error.to_ascii_lowercase();
    let start = lower.find(MARKER)? + MARKER.len();
    let rest = error.get(start..)?.trim_start();
    let rest = rest.strip_prefix('\'')?;
    let end = rest.find('\'')?;
    Some(normalize_link_path(&rest[..end]))
}

fn normalize_link_path(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_string()
}

fn path_lives_under(path: &str, entry: &Path) -> bool {
    let entry = normalize_link_path(&entry.to_string_lossy()).to_ascii_lowercase();
    let path = path.to_ascii_lowercase();
    path == entry || path.starts_with(&format!("{entry}/"))
}

fn materialize_internal_links(
    app_handle: &AppHandle,
    profile: &Path,
    need: &[(String, String, PathBuf)],
) -> Result<(), String> {
    let mut links = Vec::with_capacity(need.len());
    for (id, name, entry) in need {
        let bundled = bundled_plugin_dir(app_handle, id)
            .ok_or_else(|| format!("INTERNAL_PLUGIN_BUNDLE_MISSING: {id}"))?;
        links.push(OfflineLink {
            id: id.clone(),
            name: name.clone(),
            spec: bundled_dep_spec(&bundled),
            bundled,
            entry: entry.clone(),
        });
    }
    materialize_links(profile, &links)?;
    for (id, _, _) in need {
        if let Err(e) = super::errors::clear(app_handle, id) {
            log::warn!("failed to clear plugin error for {id} after offline link: {e}");
        }
    }
    Ok(())
}

/// 插件核对与重装（issue #466）。返回 `Err` 的原函数保留完整错误细节供日志定位。
fn remove_legacy_profile_module_fallback_best_effort(profile: &Path) {
    if let Err(e) = remove_legacy_profile_module_fallback(profile) {
        log::warn!("INTERNAL_PLUGIN_FALLBACK_CLEANUP_FAILED: {e}");
    }
}

fn remove_legacy_profile_module_fallback(profile: &Path) -> Result<(), String> {
    let fallback = profile.join(".dsh-module-fallback");
    if !fallback.is_dir() {
        return Ok(());
    }

    let node_modules = profile.join("node_modules");
    if node_modules.is_dir() {
        remove_legacy_fallback_links(&node_modules)?;
    }

    std::fs::remove_dir_all(&fallback).map_err(|e| {
        format!(
            "INTERNAL_PLUGIN_FALLBACK_REMOVE_FAILED: {}: {e}",
            fallback.display()
        )
    })?;
    log::info!(
        "Removed legacy profile-local module fallback: {}",
        fallback.display()
    );
    Ok(())
}

fn remove_legacy_fallback_links(root: &Path) -> Result<(), String> {
    let entries = std::fs::read_dir(root).map_err(|e| {
        format!(
            "INTERNAL_PLUGIN_FALLBACK_READ_FAILED: {}: {e}",
            root.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| {
            format!(
                "INTERNAL_PLUGIN_FALLBACK_ENTRY_FAILED: {}: {e}",
                root.display()
            )
        })?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(|e| {
            format!(
                "INTERNAL_PLUGIN_FALLBACK_STAT_FAILED: {}: {e}",
                path.display()
            )
        })?;
        if metadata.file_type().is_symlink() {
            if std::fs::read_link(&path)
                .ok()
                .is_some_and(|target| is_legacy_profile_fallback_target(&target))
            {
                remove_stale_plugin_entry(&path).map_err(|e| {
                    format!(
                        "INTERNAL_PLUGIN_FALLBACK_LINK_REMOVE_FAILED: {}: {e}",
                        path.display()
                    )
                })?;
            }
        } else if metadata.is_dir() {
            remove_legacy_fallback_links(&path)?;
        }
    }
    Ok(())
}

fn is_legacy_profile_fallback_target(target: &Path) -> bool {
    target
        .components()
        .any(|component| component.as_os_str() == ".dsh-module-fallback")
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn unscoped_flight_cleanup_uses_the_acquired_coordinator() {
        use crate::service::workflow::instances;
        let acquired = super::ensure_lock("ensure-stored-profile");
        let (id, _, _, _, _, _) = acquired.lock().await.start();
        instances::scope("ensure-stored-profile".into(), async {
            let spawned = super::ensure_lock(&instances::current_profile().unwrap());
            assert!(std::sync::Arc::ptr_eq(&acquired, &spawned));
            spawned.lock().await.finish(id);
        })
        .await;
        assert!(acquired.lock().await.subscribe().is_none());
    }

    #[tokio::test]
    async fn internal_plugin_flights_are_shared_only_within_profile() {
        use crate::service::workflow::instances;
        instances::scope("ensure-profile-a".into(), async {
            let first = super::ensure_lock(&instances::current_profile().unwrap());
            assert!(std::sync::Arc::ptr_eq(
                &first,
                &super::ensure_lock(&instances::current_profile().unwrap())
            ));
            let (id, _, _, _, _, _) = first.lock().await.start();
            instances::scope("ensure-profile-b".into(), async {
                let second = super::ensure_lock(&instances::current_profile().unwrap());
                assert!(!std::sync::Arc::ptr_eq(&first, &second));
                assert!(second.lock().await.subscribe().is_none());
            })
            .await;
            assert!(first.lock().await.subscribe().is_some());
            first.lock().await.finish(id);
        })
        .await;
    }

    use super::*;

    #[test]
    fn loader_rejects_non_utf8_layers_without_rewriting_and_recovers_after_quarantine() {
        for bytes in [
            b"\xff\xfe[\0]\0\n\0".as_slice(),
            b"- id: plugin\n  name: \xc4\xe3\xba\xc3\n".as_slice(),
        ] {
            for home_layer in [false, true] {
                let temp = tempfile::tempdir().unwrap();
                let home = temp.path();
                let profile = home.join("profiles/tauri");
                std::fs::create_dir_all(&profile).unwrap();
                let path = if home_layer { home } else { &profile }.join("cordis.patch.yml");
                std::fs::write(&path, bytes).unwrap();

                for prepare in [false, true] {
                    let error =
                        repair_loader_state(&profile, home, &[], &mut None, prepare).unwrap_err();
                    assert!(
                        error.starts_with("INTERNAL_PLUGIN_PATCH_ENCODING_FAILED: "),
                        "{error}"
                    );
                    assert!(error.contains(&path.display().to_string()), "{error}");
                    assert!(error.contains("UTF-8"), "{error}");
                    assert_eq!(std::fs::read(&path).unwrap(), bytes);
                }

                let recovery_profile = if home_layer {
                    let safe = home.join("profiles/safe");
                    std::fs::create_dir_all(&safe).unwrap();
                    safe
                } else {
                    profile
                };
                let report =
                    super::super::patch_guard::quarantine_patch_layers_in(&recovery_profile, home);
                assert!(!report.has_failures(), "{report:?}");
                assert_eq!(report.quarantined.len(), 1);
                assert_eq!(std::fs::read(&report.quarantined[0].backup).unwrap(), bytes);
                assert_eq!(
                    repair_loader_state(&recovery_profile, home, &[], &mut None, true),
                    Ok(false)
                );
            }
        }
    }

    #[test]
    fn loader_keeps_io_errors_distinct_from_encoding_errors() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("profiles/tauri");
        std::fs::create_dir_all(profile.join("cordis.patch.yml")).unwrap();
        let error = repair_loader_state(&profile, temp.path(), &[], &mut None, true).unwrap_err();
        assert!(
            error.starts_with("INTERNAL_PLUGIN_PATCH_READ_FAILED: "),
            "{error}"
        );
        assert!(profile.join("cordis.patch.yml").is_dir());
    }

    #[test]
    fn loader_repair_keeps_js_expressions_in_patch_layers() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("profiles/tauri");
        std::fs::create_dir_all(&profile).unwrap();
        let patch = profile.join("cordis.patch.yml");
        std::fs::write(
            &patch,
            "- insert:\n  - id: builtin\n  - id: user-plugin\n    config:\n      token: !!js process.env.TOKEN\n",
        )
        .unwrap();
        let builtin = PreinstallPluginInfo {
            id: "builtin".into(),
            spec: "builtin".into(),
            internal: true,
            package: None,
            version: None,
            name: "builtin".into(),
            description: String::new(),
            repo_url: String::new(),
            recommended: false,
            fix: false,
            default_checked: false,
            default_unchecked: false,
            win_only: false,
        };

        assert_eq!(
            repair_loader_state(&profile, temp.path(), &[&builtin], &mut None, true),
            Ok(false)
        );
        assert_eq!(
            std::fs::read_to_string(&patch).unwrap(),
            "- insert:\n  - id: user-plugin\n    config:\n      token: !!js process.env.TOKEN\n"
        );
    }

    #[test]
    fn early_prepare_uses_normal_kernel_and_defers_repairs_without_installing() {
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().join("profiles/web");
        let source = root.path().join("resources/builtin");
        let entry = profile.join("node_modules/builtin");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&entry).unwrap();
        std::fs::write(source.join("package.json"), "{}").unwrap();
        std::fs::write(entry.join("package.json"), "{}").unwrap();
        let preset = |id: &str, internal| PreinstallPluginInfo {
            id: id.into(),
            spec: id.into(),
            internal,
            package: None,
            version: None,
            name: id.into(),
            description: String::new(),
            repo_url: String::new(),
            recommended: false,
            fix: false,
            default_checked: false,
            default_unchecked: false,
            win_only: false,
        };
        let presets = vec![preset("builtin", true), preset("community", false)];
        let path = profile.join("package.json");
        let healthy = serde_json::json!({
            "dependencies": {"builtin": bundled_dep_spec(&source)},
            "dsh": {"profile": {"bundles": ["builtin", "builtin"]}}
        });
        let check = |manifest: &serde_json::Value| {
            std::fs::write(&path, manifest.to_string()).unwrap();
            prepare_early_plugins_from(&profile, root.path(), &presets, None, |_| {
                Some(source.clone())
            })
        };
        std::fs::write(
            profile.join("cordis.patch.yml"),
            "- insert:\n  - id: builtin\n  - id: user-plugin\n",
        )
        .unwrap();
        assert_eq!(check(&healthy).unwrap(), EarlyPluginState::Ready);
        let migrated: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            migrated["dsh"]["profile"]["bundles"],
            serde_json::json!(["builtin"])
        );
        assert!(std::fs::read_to_string(profile.join("cordis.patch.yml"))
            .unwrap()
            .contains("user-plugin"));
        assert_eq!(
            prepare_early_plugins_from(&profile, root.path(), &presets, None, |_| {
                Some(source.clone())
            })
            .unwrap(),
            EarlyPluginState::Ready
        );
        let running_root = "- id: builtin\n  name: runtime-composed\n";
        std::fs::write(profile.join("cordis.yml"), running_root).unwrap();
        let running_check = || {
            prepare_internal_plugins(&profile, root.path(), &[&presets[0]], false, |_| {
                Some(source.clone())
            })
        };
        let running = running_check().unwrap();
        require_running_plugins_ready(&profile, &running).unwrap();
        assert_eq!(
            std::fs::read_to_string(profile.join("cordis.yml")).unwrap(),
            running_root
        );
        let mut needs_repair = migrated.clone();
        needs_repair["dsh"]["profile"]["bundles"] = serde_json::json!([]);
        std::fs::write(&path, needs_repair.to_string()).unwrap();
        let running = running_check().unwrap();
        assert_eq!(
            running
                .need
                .iter()
                .map(|(id, _, _)| id.as_str())
                .collect::<Vec<_>>(),
            vec!["builtin"]
        );
        assert!(require_running_plugins_ready(&profile, &running)
            .unwrap_err()
            .starts_with("INTERNAL_PLUGIN_RESTART_REQUIRED:"));
        assert_eq!(
            std::fs::read_to_string(profile.join("cordis.yml")).unwrap(),
            running_root
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            needs_repair.to_string()
        );
        assert_eq!(
            std::fs::read_to_string(entry.join("package.json")).unwrap(),
            "{}"
        );
        std::fs::write(&path, healthy.to_string()).unwrap();
        let running = running_check().unwrap();
        assert!(running.migration_pending);
        assert!(require_running_plugins_ready(&profile, &running)
            .unwrap_err()
            .starts_with("INTERNAL_PLUGIN_RESTART_REQUIRED:"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), healthy.to_string());
        assert_eq!(check(&healthy).unwrap(), EarlyPluginState::Ready);
        assert_eq!(
            prepare_early_plugins_from(&profile, root.path(), &presets, None, |_| None).unwrap(),
            EarlyPluginState::Defer
        );
        std::fs::remove_file(source.join("package.json")).unwrap();
        assert_eq!(check(&healthy).unwrap(), EarlyPluginState::Defer);
        std::fs::write(source.join("package.json"), "{}").unwrap();
        let mut missing_bundle = healthy.clone();
        missing_bundle["dsh"]["profile"]["bundles"] = serde_json::json!([]);
        assert_eq!(check(&missing_bundle).unwrap(), EarlyPluginState::Defer);
        assert_eq!(
            std::fs::read_to_string(entry.join("package.json")).unwrap(),
            "{}"
        );
        let mut wrong_dep = healthy.clone();
        wrong_dep["dependencies"]["builtin"] = serde_json::json!("link:/missing/source");
        assert_eq!(check(&wrong_dep).unwrap(), EarlyPluginState::Defer);
        let mut missing_preset = healthy.clone();
        missing_preset["dependencies"]["community"] = serde_json::json!("1.0.0");
        assert_eq!(check(&missing_preset).unwrap(), EarlyPluginState::Defer);
        let mut dangling = healthy.clone();
        dangling["dependencies"]["removed"] = serde_json::json!("link:missing-source");
        assert_eq!(check(&dangling).unwrap(), EarlyPluginState::Defer);
        let old_plugin = profile.join("node_modules/old-plugin");
        std::fs::create_dir_all(&old_plugin).unwrap();
        std::fs::write(old_plugin.join("data.json"), "user data").unwrap();
        let mut with_old_plugin = migrated.clone();
        with_old_plugin["dependencies"]["old-plugin"] = serde_json::json!("1.0.0");
        with_old_plugin["dsh"]["profile"]["bundles"] = serde_json::json!(["builtin", "old-plugin"]);
        assert_eq!(check(&with_old_plugin).unwrap(), EarlyPluginState::Ready);
        let preserved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(preserved, with_old_plugin);
        assert_eq!(
            std::fs::read_to_string(old_plugin.join("data.json")).unwrap(),
            "user data"
        );
        std::fs::remove_file(entry.join("package.json")).unwrap();
        assert_eq!(check(&healthy).unwrap(), EarlyPluginState::Defer);
        assert!(!entry.join("package.json").exists());
        std::fs::write(&path, "{").unwrap();
        assert!(
            prepare_early_plugins_from(&profile, root.path(), &presets, None, |_| None)
                .unwrap_err()
                .starts_with("INTERNAL_PLUGIN_MANIFEST_PARSE_FAILED:")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{");
        assert_eq!(
            prepare_early_plugins(&profile, root.path(), None, None, None).unwrap(),
            EarlyPluginState::Defer
        );
    }

    #[test]
    fn post_install_manifest_check_rejects_missing_or_unreadable_manifest() {
        let root = tempfile::tempdir().unwrap();
        let need = need_entries(&["dsh-tauri", "dsh-tauri-ui"]);
        let expected = Some(vec!["dsh-tauri".to_string(), "dsh-tauri-ui".to_string()]);
        assert_eq!(presets_missing_from_manifest(root.path(), &need), expected);
        std::fs::create_dir(root.path().join("package.json")).unwrap();
        assert_eq!(presets_missing_from_manifest(root.path(), &need), expected);
    }

    #[test]
    fn post_install_manifest_check_rejects_truncated_manifest() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("package.json"),
            br#"{"dependencies":{"dsh-tauri":"#,
        )
        .unwrap();
        assert_eq!(
            presets_missing_from_manifest(root.path(), &need_entries(&["dsh-tauri"])),
            Some(vec!["dsh-tauri".to_string()])
        );
    }

    #[test]
    fn post_install_manifest_check_requires_dependency_and_bundle_registration() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("package.json"), serde_json::json!({
            "dependencies": {"dsh-tauri": "link:/resources/dsh-tauri", "dsh-tauri-ui": "link:/resources/dsh-tauri-ui"},
            "dsh": {"profile": {"bundles": ["dsh-tauri", "dsh-tauri-pet", "community-plugin"]}}
        }).to_string()).unwrap();
        assert_eq!(
            presets_missing_from_manifest(
                root.path(),
                &need_entries(&["dsh-tauri", "dsh-tauri-ui", "dsh-tauri-pet"])
            ),
            Some(vec![
                "dsh-tauri-ui".to_string(),
                "dsh-tauri-pet".to_string()
            ])
        );
        assert_eq!(
            presets_missing_from_manifest(root.path(), &need_entries(&["dsh-tauri"])),
            None
        );
        assert_eq!(presets_missing_from_manifest(root.path(), &[]), None);
    }

    fn need_entries(names: &[&str]) -> Vec<(String, String, PathBuf)> {
        let profile = Path::new("C:\\Users\\w00012491\\.dsh\\profiles\\tauri");
        names
            .iter()
            .map(|name| {
                (
                    (*name).to_string(),
                    (*name).to_string(),
                    profile.join("node_modules").join(name),
                )
            })
            .collect()
    }

    /// issue #264 的**恒定**版本：pnpm 建好 `link:` 目录链接后回读 package.json
    #[test]
    fn link_materialization_failure_detects_uv_unknown_readback() {
        let error = "PREINSTALL_FAILED: dsh plugin exited with code -4094: UNKNOWN  UNKNOWN: \
                     unknown error, open 'C:\\Users\\w00012491\\.dsh\\profiles\\tauri\\node_modules\\\
                     dsh-tauri-scheduler\\package.json'";
        let need = need_entries(&["dsh-tauri-scheduler"]);
        assert!(is_link_materialization_failure(error, &need));
        assert!(is_link_materialization_failure(
            &error.to_ascii_lowercase(),
            &need
        ));
    }

    #[test]
    fn link_materialization_failure_detects_silent_install() {
        assert!(is_link_materialization_failure(
            "PREINSTALL_SILENT_FAIL: dsh plugin exited with code 0, but no install artifact was \
             created for [dsh-tauri]. Expected package manifests: [...]",
            &need_entries(&["dsh-tauri"])
        ));
    }

    /// issue #1156：pnpm 以 0 退出（或 pnpm 没落盘却让 dsh 继续结算）时，dsh 的
    #[test]
    fn link_materialization_failure_detects_unresolved_internal_bundle() {
        let error = "PREINSTALL_FAILED: dsh plugin exited with code 1: throw new Error(`${binName}: \
                     cannot resolve profile bundle ${JSON.stringify(packageName)} ...`);\n\
                     Error: dsh: cannot resolve profile bundle \"dsh-tauri\" from the dsh \
                     installation or C:\\Users\\A1\\.dsh\\profiles\\web; run 'dsh plugin --profile \
                     web install' if its dependency is not installed";
        assert!(is_link_materialization_failure(
            error,
            &need_entries(&["dsh-tauri", "dsh-tauri-ui"])
        ));
        assert!(is_link_materialization_failure(
            "cannot resolve profile bundle '@scope/dsh-tauri' from the dsh installation",
            &need_entries(&["@scope/dsh-tauri"])
        ));
        let scoped = vec![(
            "dsh-tauri".to_string(),
            "@scope/dsh-tauri".to_string(),
            PathBuf::from("node_modules/@scope/dsh-tauri"),
        )];
        assert!(!is_link_materialization_failure(
            "cannot resolve profile bundle \"dsh-tauri\" from the dsh installation",
            &scoped
        ));
        assert!(is_link_materialization_failure(
            "cannot resolve profile bundle \"@scope/dsh-tauri\" from the dsh installation",
            &scoped
        ));
        assert!(!is_link_materialization_failure(
            error,
            &need_entries(&["dsh-tauri-ui"])
        ));
        assert!(!is_link_materialization_failure(
            "cannot resolve profile bundle \"someone-else\" from the dsh installation",
            &need_entries(&["dsh-tauri"])
        ));
    }

    /// pnpm 以 0 退出但入口没落盘时，收尾必须识别出来并重新物化（issue #1156）。
    #[test]
    fn post_install_repair_detects_unlinked_internal_entries() {
        let root = tempfile::tempdir().unwrap();
        let profile = root.path();
        let need: Vec<(String, String, PathBuf)> = ["dsh-tauri"]
            .into_iter()
            .map(|name| {
                (
                    name.to_string(),
                    name.to_string(),
                    profile.join("node_modules").join(name),
                )
            })
            .collect();

        assert_eq!(
            unlinked_internal_entries(&need),
            vec!["dsh-tauri".to_string()]
        );
        std::fs::create_dir_all(profile.join("node_modules").join("dsh-tauri")).unwrap();
        std::fs::write(
            profile.join("node_modules").join("dsh-tauri").join("package.json"),
            r#"{"name":"dsh-tauri"}"#,
        )
        .unwrap();
        assert!(unlinked_internal_entries(&need).is_empty());
    }

    #[test]
    fn link_materialization_failure_rejects_foreign_dependency_path() {
        let error = "PREINSTALL_FAILED: dsh plugin exited with code -4094: UNKNOWN  UNKNOWN: \
                     unknown error, open 'C:\\Users\\w00012491\\.dsh\\profiles\\tauri\\node_modules\\\
                     someone-else-plugin\\package.json'";
        assert!(!is_link_materialization_failure(
            error,
            &need_entries(&["dsh-tauri-scheduler"])
        ));
    }

    #[test]
    fn link_materialization_failure_rejects_unattributed_diagnostic() {
        let error = "PREINSTALL_FAILED: dsh plugin exited with code -4094: UNKNOWN: cannot \
                     materialize node_modules entry";
        let need = need_entries(&["dsh-tauri-scheduler"]);

        assert!(!is_link_materialization_failure(error, &need));
        assert!(is_untraceable_link_readback_failure(error));
        assert!(!is_untraceable_link_readback_failure(
            "PREINSTALL_FAILED: dsh plugin exited with code -4094: UNKNOWN: unknown error, open \
             'C:\\Users\\w00012491\\.dsh\\profiles\\tauri\\node_modules\\someone-else\\package.json'"
        ));
    }

    #[test]
    fn link_materialization_failure_rejects_unrelated_failures() {
        let need = need_entries(&["dsh-tauri"]);
        assert!(!is_link_materialization_failure(
            "PREINSTALL_FAILED: dsh plugin exited with code 1: ERR_PNPM_SPEC_NOT_SUPPORTED",
            &need
        ));
        assert!(!is_link_materialization_failure(
            "NETWORK_ERROR: plugin registry request failed; check network or proxy settings and retry.",
            &need
        ));
        assert!(!is_link_materialization_failure(
            "PREINSTALL_ENTRY_FAILED:\ndsh-tauri: PLUGIN_ENTRY_MISSING",
            &need
        ));
        assert!(!is_link_materialization_failure(
            "PREINSTALL_FAILED: dsh plugin exited with code -4094: some unrelated output",
            &need
        ));
        assert!(!is_untraceable_link_readback_failure(
            "PREINSTALL_FAILED: dsh plugin exited with code -4094: some unrelated output"
        ));
    }

    #[test]
    fn legacy_fallback_target_detection_is_path_component_aware() {
        let base = Path::new("home")
            .join("test")
            .join(".dsh")
            .join("profiles")
            .join("web");
        assert!(is_legacy_profile_fallback_target(
            &base
                .join(".dsh-module-fallback")
                .join("node_modules")
                .join("anymatch"),
        ));
        assert!(!is_legacy_profile_fallback_target(
            &base.join("node_modules").join("anymatch"),
        ));
        assert!(!is_legacy_profile_fallback_target(
            &base.join(".dsh-module-fallback-old").join("anymatch"),
        ));
    }

    #[tokio::test]
    async fn coordinator_coalesces_waiters_and_releases_for_retry() {
        let mut coordinator = EnsureCoordinator::default();
        let (first_id, first_owner, result_tx, first, _, _) = coordinator.start();
        let EnsureSubscription::Running(duplicate) = coordinator.subscribe().unwrap() else {
            panic!("running flight must coalesce running waiters");
        };
        assert_eq!(coordinator.active.as_ref().unwrap().owner, first_owner);
        coordinator.finish(first_id);
        assert!(coordinator.subscribe().is_none());
        result_tx.send(Some(Ok(()))).unwrap();
        assert_eq!(first.borrow().as_ref(), Some(&Ok(())));
        assert_eq!(duplicate.borrow().as_ref(), Some(&Ok(())));

        let (retry_id, _, _, _, _, _) = coordinator.start();
        assert_ne!(retry_id, first_id);
    }

    #[tokio::test]
    async fn cancel_then_immediate_retry_starts_one_fresh_flight() {
        type FlightResultSender = tokio::sync::watch::Sender<Option<Result<(), String>>>;

        let coordinator =
            std::sync::Arc::new(tokio::sync::Mutex::new(EnsureCoordinator::default()));
        let fresh_flight = std::sync::Arc::new(tokio::sync::Mutex::new(
            None::<(u64, ProcessOwner, FlightResultSender)>,
        ));
        let fresh_started = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cancelling_subscribers = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cancelling_subscribed = std::sync::Arc::new(tokio::sync::Notify::new());

        let (cancelled_id, cancelled_owner, cancelled_tx) = {
            let mut coordinator = coordinator.lock().await;
            let (id, owner, result_tx, _, _, mut cancel_signal) = coordinator.start();
            coordinator.begin_cancel().unwrap();
            assert!(*cancel_signal.borrow_and_update());
            (id, owner, result_tx)
        };

        async fn retry(
            coordinator: std::sync::Arc<tokio::sync::Mutex<EnsureCoordinator>>,
            fresh_flight: std::sync::Arc<
                tokio::sync::Mutex<Option<(u64, ProcessOwner, FlightResultSender)>>,
            >,
            fresh_started: std::sync::Arc<std::sync::atomic::AtomicUsize>,
            cancelling_subscribers: std::sync::Arc<std::sync::atomic::AtomicUsize>,
            cancelling_subscribed: std::sync::Arc<tokio::sync::Notify>,
        ) -> Result<(), String> {
            receive_current_or_next_flight(|| {
                let coordinator = coordinator.clone();
                let fresh_flight = fresh_flight.clone();
                let fresh_started = fresh_started.clone();
                let cancelling_subscribers = cancelling_subscribers.clone();
                let cancelling_subscribed = cancelling_subscribed.clone();
                async move {
                    let mut coordinator = coordinator.lock().await;
                    if let Some(subscription) = coordinator.subscribe() {
                        if matches!(&subscription, EnsureSubscription::Cancelling(_)) {
                            cancelling_subscribers
                                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            cancelling_subscribed.notify_one();
                        }

                        return subscription;
                    }
                    let (id, owner, result_tx, result_rx, _, _) = coordinator.start();
                    fresh_started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    *fresh_flight.lock().await = Some((id, owner, result_tx));
                    EnsureSubscription::Running(result_rx)
                }
            })
            .await
        }

        let first_retry = tokio::spawn(retry(
            coordinator.clone(),
            fresh_flight.clone(),
            fresh_started.clone(),
            cancelling_subscribers.clone(),
            cancelling_subscribed.clone(),
        ));
        let second_retry = tokio::spawn(retry(
            coordinator.clone(),
            fresh_flight.clone(),
            fresh_started.clone(),
            cancelling_subscribers.clone(),
            cancelling_subscribed.clone(),
        ));

        while cancelling_subscribers.load(std::sync::atomic::Ordering::SeqCst) < 2 {
            cancelling_subscribed.notified().await;
        }
        {
            let mut coordinator = coordinator.lock().await;
            coordinator.finish(cancelled_id);
        }
        cancelled_tx
            .send(Some(Err(
                "INTERNAL_PLUGIN_INSTALL_CANCELLED: plugin install was cancelled".to_string(),
            )))
            .unwrap();

        let (retry_id, retry_owner, retry_tx) = loop {
            if let Some(flight) = fresh_flight.lock().await.take() {
                break flight;
            }
            tokio::task::yield_now().await;
        };
        assert_ne!(retry_owner, cancelled_owner);
        assert_eq!(fresh_started.load(std::sync::atomic::Ordering::SeqCst), 1);

        coordinator.lock().await.finish(retry_id);
        retry_tx.send(Some(Ok(()))).unwrap();
        assert!(first_retry.await.unwrap().is_ok());
        assert!(second_retry.await.unwrap().is_ok());
        assert_eq!(fresh_started.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(coordinator.lock().await.subscribe().is_none());
    }

    #[tokio::test]
    async fn stuck_owner_is_bounded_and_blocks_new_flights_without_hanging_waiters() {
        let released = wait_for_release_with(
            || true,
            std::time::Duration::from_millis(5),
            std::time::Duration::from_millis(1),
        )
        .await;
        assert!(!released);

        let mut coordinator = EnsureCoordinator::default();
        let (id, _, result_tx, mut result, _, _) = coordinator.start();
        let reason = "INTERNAL_PLUGIN_PROCESS_REAP_TIMEOUT: test owner remained active".to_string();
        result_tx.send(Some(Err(reason.clone()))).unwrap();
        coordinator.mark_cleanup_failed(id, reason.clone());

        assert!(matches!(
            coordinator.subscribe(),
            Some(EnsureSubscription::CleanupFailed(error)) if error == reason
        ));
        let outcome = receive_flight_result(&mut result).await.unwrap();
        assert_eq!(outcome, Err(reason));
        assert_eq!(coordinator.next_id, 1);
    }

    /// issue #466：旧版 profile-local fallback 目录不可删（典型：属主是 root）时，
    #[cfg(unix)]
    #[test]
    fn unwritable_fallback_cleanup_is_best_effort() {
        use std::os::unix::fs::PermissionsExt;

        let profile =
            std::env::temp_dir().join(format!("dsh-fallback-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&profile);
        let nested = profile.join(".dsh-module-fallback").join("node_modules");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("anymatch"), b"").unwrap();

        let mut locked = std::fs::metadata(&nested).unwrap().permissions();
        locked.set_mode(0o555);
        std::fs::set_permissions(&nested, locked).unwrap();

        if let Err(error) = remove_legacy_profile_module_fallback(&profile) {
            assert!(
                error.starts_with("INTERNAL_PLUGIN_FALLBACK_REMOVE_FAILED"),
                "{error}"
            );
        }
        remove_legacy_profile_module_fallback_best_effort(&profile);

        if let Ok(metadata) = std::fs::metadata(&nested) {
            let mut open = metadata.permissions();
            open.set_mode(0o755);
            let _ = std::fs::set_permissions(&nested, open);
        }
        let _ = std::fs::remove_dir_all(&profile);
    }
}
