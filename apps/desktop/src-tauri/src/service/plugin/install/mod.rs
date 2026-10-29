
use crate::config;
use crate::service::cli;
use crate::service::core;
use crate::service::profile::{
    active_profile, allow_profile_release_age, profile_dir_of, release_age_excludes_at,
};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use tauri::{Emitter};
use crate::{AppHandle, WebviewWindow};

pub(crate) use super::ensure_profile_npmrc;
pub(crate) use super::errors;
pub(crate) use super::installed::{installed_name, is_installed, profile_dir};
pub(crate) use super::preset::{
    bundled_dep_spec, bundled_plugin_dir, load_presets, PreinstallPluginInfo,
};
pub(crate) use super::process::{
    acquire_operation_lock, acquire_process_lock, new_process_owner, run_plugin_process, PidGuard,
    PreinstallLogPayload, ProcessOwner, PREINSTALL_LOG_EVENT,
};
pub(crate) use super::recovery::is_actionable_plugin_ref;
pub(crate) use super::uninstall_recovery;
pub(crate) use crate::service::profile::ensure_profile_pnpm_policy;

mod allowlist;
mod artifact;
mod diagnose;
mod env;
mod inspect;
mod pnpm;
mod single;
mod spec;

pub(crate) use env::build_plugin_envs;
pub use inspect::inspect_specs;
pub(crate) use inspect::packument;
pub(crate) use pnpm::{
    bundled_pnpm_major, harness_prefer_bundled_pnpm, harness_prefer_bundled_pnpm_from,
    pnpm_major_version_at_bounded, profile_store_major,
};
pub use single::{remove_many, update_many};
pub use diagnose::{IncompatibleVersion, PolicyBlockedVersion};

use allowlist::{add_allow_build_keys, parse_allowlist_keys};
use artifact::{ensure_plugin_entry_built, verify_installed_products};
use diagnose::{
    diagnostic_suffix, git_transport_hint, incompatible_versions, network_error_hint,
    pick_error_message, policy_blocked_versions, policy_verification_network_failure,
    store_mismatch_hint,
};
use pnpm::ensure_pnpm;
use single::single_plugin_args;
pub(crate) use spec::local_spec_from_path;
use spec::{bundled_dir_of, normalize_git_spec, preset_spec_for_install};

const MAX_ALLOW_LIST_RETRIES: usize = 8;

/// 下次启动又判定 `dep_ok=false` 而重装，形成不可恢复的启动死循环（issue #264）。
const TRANSIENT_FS_RETRIES: usize = 8;
const TRANSIENT_FS_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

fn capped_backoff(retry: usize, base_secs: u64, cap_secs: u64) -> std::time::Duration {
    std::time::Duration::from_secs(
        base_secs
            .checked_shl(retry.saturating_sub(1) as u32)
            .unwrap_or(cap_secs)
            .min(cap_secs),
    )
}

fn transient_fs_retry_delay(retry: usize) -> std::time::Duration {
    capped_backoff(retry, TRANSIENT_FS_RETRY_DELAY.as_secs(), 64)
}

const POLICY_VERIFICATION_RETRIES: usize = 4;
const POLICY_VERIFICATION_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(5);

fn policy_verification_retry_delay(retry: usize) -> std::time::Duration {
    capped_backoff(retry, POLICY_VERIFICATION_RETRY_DELAY.as_secs(), 30)
}

#[derive(Debug)]
pub(crate) struct InstallTarget {
    pub id: String,
    pub name: Option<String>,
    pub spec: String,
}

pub async fn install(app_handle: &AppHandle, ids: &[String]) -> Result<(), String> {
    let targets = preset_targets(app_handle, ids)?;
    install_with_cancel(app_handle, &targets, None, new_process_owner()).await
}

pub(crate) async fn install_internal(
    app_handle: &AppHandle,
    ids: &[String],
    cancel: tokio::sync::watch::Receiver<bool>,
    owner: ProcessOwner,
) -> Result<(), String> {
    let targets = preset_targets(app_handle, ids)?;
    install_with_cancel(app_handle, &targets, Some(cancel), owner).await
}

pub async fn install_specs(app_handle: &AppHandle, specs: &[String]) -> Result<(), String> {
    let targets = spec_targets(app_handle, specs)?;
    if targets.is_empty() {
        return Err("PLUGIN_SPECS_EMPTY: no plugin specs provided".to_string());
    }
    install_with_cancel(app_handle, &targets, None, new_process_owner()).await
}

/// 缺陷（pnpm issue #3948 / #7243 / #13276）：公开仓库一旦落进 git+ssh，在没有
fn preset_targets(app_handle: &AppHandle, ids: &[String]) -> Result<Vec<InstallTarget>, String> {
    if ids.is_empty() {
        return Err("PREINSTALL_EMPTY: no plugins selected".to_string());
    }

    let presets = load_presets(app_handle);
    let preset_map: HashMap<&str, &PreinstallPluginInfo> =
        presets.iter().map(|p| (p.id.as_str(), p)).collect();
    let core_version = crate::service::core::active_version(app_handle);

    let mut targets = Vec::with_capacity(ids.len());
    for id in ids {
        let preset = preset_map
            .get(id.as_str())
            .ok_or_else(|| format!("PREINSTALL_INVALID_ID: {id}"))?;
        let raw = normalize_git_spec(&preset_spec_for_install(
            preset,
            bundled_dir_of(app_handle, preset),
            core_version.as_deref(),
        )?);
        targets.push(InstallTarget {
            id: preset.id.clone(),
            name: Some(installed_name(preset).to_string()),
            spec: raw,
        });
    }
    Ok(targets)
}

fn spec_targets(app_handle: &AppHandle, specs: &[String]) -> Result<Vec<InstallTarget>, String> {
    let core_version = crate::service::core::active_version(app_handle);
    let presets = load_presets(app_handle);
    let base = crate::service::core::active_core_dir(app_handle);
    let mut targets = Vec::with_capacity(specs.len());
    for spec in specs {
        let spec = spec.trim();
        if spec.is_empty() {
            continue;
        }
        if let Some(local) = spec::local_path_spec(spec) {
            targets.push(local_target(&local, &base)?);
            continue;
        }
        if let Some(preset) = presets
            .iter()
            .find(|preset| preset.id == spec || preset.spec == spec)
        {
            if let Ok(raw) = preset_spec_for_install(
                preset,
                bundled_dir_of(app_handle, preset),
                core_version.as_deref(),
            ) {
                targets.push(InstallTarget {
                    id: preset.id.clone(),
                    name: Some(installed_name(preset).to_string()),
                    spec: normalize_git_spec(&raw),
                });
                continue;
            }
        }
        let raw = normalize_git_spec(spec);
        let name = spec::package_name_of_spec(&raw, &base);
        targets.push(InstallTarget {
            id: name.clone().unwrap_or_else(|| raw.clone()),
            name,
            spec: raw,
        });
    }
    Ok(targets)
}

fn local_target(raw: &Path, base: &Path) -> Result<InstallTarget, String> {
    let spec = raw.to_string_lossy();
    let Some(dir) = spec::local_dir_of(&spec, base) else {
        return Err(format!("PLUGIN_LOCAL_SPEC_INVALID: {spec}"));
    };
    if !dir.is_dir() {
        return Err(format!(
            "PLUGIN_LOCAL_DIR_MISSING: local plugin directory not found: {}",
            dir.display()
        ));
    }
    let name = spec::local_package_name(&dir);
    let link = format!("link:{}", spec::forward_slashes(&dir));
    Ok(InstallTarget {
        id: name.clone().unwrap_or_else(|| link.clone()),
        name,
        spec: link,
    })
}

fn locked_release_age_exemptions(
    profile: &Path,
    blocked: &[PolicyBlockedVersion],
) -> Option<Vec<String>> {
    if blocked.is_empty() {
        return None;
    }
    let content = std::fs::read_to_string(profile.join("pnpm-lock.yaml")).ok()?;
    let lockfile: serde_yaml::Value = serde_yaml::from_str(&content).ok()?;
    let packages = lockfile.get("packages")?.as_mapping()?;
    let mut entries = Vec::with_capacity(blocked.len());
    for item in blocked {
        let entry = format!("{}@{}", item.name, item.version);
        if !packages
            .keys()
            .filter_map(serde_yaml::Value::as_str)
            .any(|key| {
                key.strip_prefix(&entry).is_some_and(|suffix| {
                    suffix.is_empty() || (suffix.starts_with('(') && suffix.ends_with(')'))
                })
            })
        {
            return None;
        }
        entries.push(entry);
    }
    Some(entries)
}

fn heal_locked_release_age(profile: &Path, output: &str) -> bool {
    let blocked = policy_blocked_versions(output);
    let Some(entries) = locked_release_age_exemptions(profile, &blocked) else {
        return false;
    };
    match release_age_excludes_at(profile, &entries) {
        Ok(()) => {
            log::warn!(
                "pnpm release-age policy blocked {} entries that are already locked; recorded exact exemptions and retrying with the default window",
                entries.len()
            );
            true
        }
        Err(error) => {
            log::warn!("failed to record the release-age exemptions for locked entries: {error}");
            false
        }
    }
}

async fn install_with_cancel(
    app_handle: &AppHandle,
    targets: &[InstallTarget],
    cancel: Option<tokio::sync::watch::Receiver<bool>>,
    owner: ProcessOwner,
) -> Result<(), String> {
    if targets.is_empty() {
        return Err("PREINSTALL_EMPTY: no plugins selected".to_string());
    }

    let specs: Vec<&str> = targets.iter().map(|t| t.spec.as_str()).collect();
    let needs_git = specs.iter().any(|s| s.starts_with("git+"));

    // git 托管插件安装前预检（issue #369）：Linux/macOS 完全依赖系统 git（不在
    if needs_git && !env::git_available(app_handle) {
        return Err(
            "GIT_NOT_FOUND: selected plugins include git-hosted dependencies, but no usable git \
             was found on this system. Install git (e.g. Debian/Ubuntu: `sudo apt install git`; \
             macOS: `brew install git`) or uncheck those plugins and retry."
                .to_string(),
        );
    }

    cli::ensure_shims(app_handle)?;

    let node = config::get_node_binary_path(app_handle);
    let dsh_bin = core::active_dsh_binary(app_handle);
    if !node.exists() {
        return Err("NODE_NOT_FOUND: Node.js runtime missing".to_string());
    }
    if !dsh_bin.exists() {
        return Err("HARNESS_NOT_FOUND: dsh CLI missing".to_string());
    }

    let window = crate::utils::operation_window(app_handle)
        .ok_or("WINDOW_NOT_FOUND: current profile window missing")?;

    let prefer_bundled_pnpm = ensure_pnpm(app_handle, &window, owner).await?;
    // 环境以 ERR_PNPM_ABORTED_REMOVE_MODULES_DIR_NO_TTY 中止（issue #130）。
    super::ensure_profile_npmrc(app_handle)?;
    // 精确的已审查 zod 版本，避免 registry 元数据瞬时失败阻断插件安装（issue #222）。
    super::ensure_profile_pnpm_policy(app_handle)?;
    // （issue #303：自动快照失败不阻塞主流程，避免升级被陈旧快照问题拖垮）。
    for target in targets {
        if is_installed(app_handle, &target.id) {
            super::snapshot::create_best_effort(app_handle, &target.id);
        }
    }

    let envs = build_plugin_envs(app_handle, prefer_bundled_pnpm);

    let profile = active_profile(app_handle);
    let profile_path = profile_dir_of(app_handle, &profile);
    let mut args = vec![
        dsh_bin.as_os_str().to_os_string(),
        OsString::from("plugin"),
        OsString::from("--profile"),
        OsString::from(&profile),
        OsString::from("add"),
    ];
    args.extend(specs.iter().map(OsString::from));

    let cwd = crate::service::core::active_core_dir(app_handle);
    log::info!("Running dsh plugin install for {specs:?}");

    let (exit_code, last_output, last_attempt) = run_plugin_install_with_transient_retry(
        app_handle,
        &node,
        &args,
        &cwd,
        &envs,
        &window,
        "install",
        cancel.as_ref(),
        owner,
    )
    .await?;

    let (exit_code, last_output, last_attempt) =
        if exit_code != 0 && heal_locked_release_age(&profile_path, &last_attempt) {
            run_plugin_install_with_transient_retry(
                app_handle,
                &node,
                &args,
                &cwd,
                &envs,
                &window,
                "install",
                cancel.as_ref(),
                owner,
            )
            .await?
        } else {
            (exit_code, last_output, last_attempt)
        };

    if exit_code != 0 {
        log::error!("dsh plugin install failed with exit code {exit_code}");
        let detail = pick_error_message(&last_output, None);
        if !detail.is_empty() {
            log::error!("dsh plugin install diagnostic: {detail}");
        }
        let incompatible = incompatible_versions(&last_attempt);
        if !incompatible.is_empty() {
            log::warn!(
                "dsh rejected the install for incompatible plugin versions: {incompatible:?}"
            );
            let payload = serde_json::to_string(&incompatible)
                .map_err(|e| format!("PREINSTALL_SERIALIZE: {e}"))?;
            return Err(format!("PLUGIN_VERSION_INCOMPATIBLE: {payload}"));
        }
        let policy_blocked = policy_blocked_versions(&last_attempt);
        if !policy_blocked.is_empty() {
            log::warn!(
                "pnpm release-age policy rejected {} profile entries: {policy_blocked:?}",
                policy_blocked.len()
            );
            let payload = serde_json::to_string(&policy_blocked)
                .map_err(|e| format!("PREINSTALL_SERIALIZE: {e}"))?;
            return Err(format!("PLUGIN_POLICY_BLOCKED: {payload}"));
        }
        let network_error = network_error_hint(&last_attempt).is_some()
            || policy_verification_network_failure(&last_attempt)
            || (exit_code == 3 && last_attempt.trim().is_empty());
        let hint = git_transport_hint(&last_attempt);
        let store_hint = store_mismatch_hint(&last_attempt);
        let network_hint = network_error.then_some(
            "NETWORK_ERROR: plugin registry request failed; check network or proxy settings and retry.",
        );
        let message = if network_error {
            network_hint.unwrap_or_default().to_string()
        } else if let Some(store_hint) = store_hint.as_deref() {
            store_hint.to_string()
        } else {
            pick_error_message(&last_output, hint)
        };
        for target in targets {
            if let Err(e) = errors::record(app_handle, &target.id, "install", &message) {
                log::warn!("failed to record plugin error for {}: {e}", target.id);
            }
        }
        if let Some(network_hint) = network_hint {
            log::warn!("network failure detected during plugin install: {network_hint}");
            let _ = window.emit(
                PREINSTALL_LOG_EVENT,
                PreinstallLogPayload {
                    line: format!("[network] {network_hint}"),
                },
            );
            return Err(network_hint.to_string());
        }
        if let Some(hint) = hint {
            log::warn!("git transport failure detected during plugin install: {hint}");
            let _ = window.emit(
                PREINSTALL_LOG_EVENT,
                PreinstallLogPayload {
                    line: format!("[pnpm] {hint}"),
                },
            );
            return Err(format!(
                "PREINSTALL_FAILED: dsh plugin exited with code {exit_code} ({hint})"
            ));
        }
        if let Some(store_hint) = store_hint {
            log::warn!("pnpm store incompatibility detected during plugin install: {store_hint}");
            let _ = window.emit(
                PREINSTALL_LOG_EVENT,
                PreinstallLogPayload {
                    line: format!("[pnpm] {store_hint}"),
                },
            );
            return Err(format!(
                "PREINSTALL_FAILED: dsh plugin exited with code {exit_code} ({store_hint})"
            ));
        }
        let detail = pick_error_message(&last_output, None);
        return Err(format!(
            "PREINSTALL_FAILED: dsh plugin exited with code {exit_code}{}",
            diagnostic_suffix(&detail)
        ));
    }

    verify_installed_products(app_handle, targets, &last_output)?;

    let mut entry_errors = Vec::new();
    for target in targets {
        let Some(name) = target.name.as_deref() else {
            continue;
        };
        let pkg_dir = profile_dir(app_handle).join("node_modules").join(name);
        if let Err(e) =
            ensure_plugin_entry_built(app_handle, &target.id, &pkg_dir, &envs, &window).await
        {
            if let Err(err) = errors::record(app_handle, &target.id, "install", &e) {
                log::warn!("failed to record plugin error for {}: {err}", target.id);
            }
            entry_errors.push(format!("{}: {e}", target.id));
        }
    }
    if !entry_errors.is_empty() {
        return Err(format!(
            "PREINSTALL_ENTRY_FAILED:\n{}",
            entry_errors.join("\n")
        ));
    }

    let _ = window.emit(
        PREINSTALL_LOG_EVENT,
        PreinstallLogPayload {
            line: format!("[harness] 已安装 {} 个插件", targets.len()),
        },
    );

    log::info!(
        "Preinstall plugins installed successfully: {:?}",
        targets.iter().map(|t| &t.id).collect::<Vec<_>>()
    );
    Ok(())
}

pub async fn allow_version_exemptions(
    app_handle: &AppHandle,
    versions: &[IncompatibleVersion],
) -> Result<(), String> {
    if versions.is_empty() {
        return Err("PLUGIN_EXEMPTION_EMPTY: no plugin version exemption to grant".to_string());
    }
    let window = crate::utils::operation_window(app_handle)
        .ok_or("WINDOW_NOT_FOUND: current profile window missing")?;
    let node = config::get_node_binary_path(app_handle);
    let dsh_bin = core::active_dsh_binary(app_handle);
    if !node.exists() {
        return Err("NODE_NOT_FOUND: Node.js runtime missing".to_string());
    }
    if !dsh_bin.exists() {
        return Err("HARNESS_NOT_FOUND: dsh CLI missing".to_string());
    }

    let envs = build_plugin_envs(app_handle, false);
    let cwd = crate::service::core::active_core_dir(app_handle);
    let profile = active_profile(app_handle);
    let owner = new_process_owner();
    let mut failures: Vec<String> = Vec::new();
    for entry in versions {
        let mut args = vec![dsh_bin.as_os_str().to_os_string()];
        args.extend(single_plugin_args(
            &profile,
            "allow-version",
            &[
                format!("{}@{}", entry.name, entry.version),
                "--dsh-version".to_string(),
                entry.runtime_version.clone(),
                "--accept-risk".to_string(),
            ],
        ));
        log::info!(
            "Granting plugin version exemption {}@{} for DSH {}",
            entry.name,
            entry.version,
            entry.runtime_version
        );
        let (exit_code, output) =
            run_plugin_process(&node, &args, &cwd, &envs, &window, owner).await?;
        if exit_code != 0 {
            let detail = pick_error_message(&output, None);
            log::error!(
                "granting plugin version exemption {}@{} failed with exit code {exit_code}: {detail}",
                entry.name,
                entry.version
            );
            failures.push(format!(
                "{}@{}: dsh plugin exited with code {exit_code}{}",
                entry.name,
                entry.version,
                diagnostic_suffix(&detail)
            ));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("PLUGIN_EXEMPTION_FAILED: {}", failures.join("; ")))
    }
}

pub fn allow_policy_versions(
    app_handle: &AppHandle,
    versions: &[PolicyBlockedVersion],
    expected_profile: Option<&str>,
) -> Result<(), String> {
    let entries: Vec<String> = versions
        .iter()
        .map(|entry| format!("{}@{}", entry.name, entry.version))
        .collect();
    allow_profile_release_age(app_handle, &entries, expected_profile)
}

#[allow(clippy::too_many_arguments)]
async fn run_plugin_with_allow_build_retry(
    app_handle: &AppHandle,
    node: &Path,
    args: &[OsString],
    cwd: &Path,
    envs: &HashMap<String, String>,
    window: &WebviewWindow,
    action: &str,
    cancel: Option<&tokio::sync::watch::Receiver<bool>>,
    owner: ProcessOwner,
) -> Result<(i32, String, String), String> {
    let _operation_guard = acquire_operation_lock().await;
    super::process::clear_orphan_plugin_writer_lock(&super::installed::profile_dir(app_handle));
    let mut retries = 0usize;
    let mut all_output = String::new();
    let mut last_attempt;
    let exit_code = loop {
        if cancel.is_some_and(|signal| *signal.borrow()) {
            return Err("PLUGIN_OPERATION_CANCELLED: plugin operation was cancelled".to_string());
        }
        let (code, captured) = run_plugin_process(node, args, cwd, envs, window, owner).await?;
        if cancel.is_some_and(|signal| *signal.borrow()) {
            return Err("PLUGIN_OPERATION_CANCELLED: plugin operation was cancelled".to_string());
        }
        append_command_output(&mut all_output, &captured);
        let new_keys = parse_allowlist_keys(&captured);
        last_attempt = captured;
        if !new_keys.is_empty() && retries < MAX_ALLOW_LIST_RETRIES {
            retries += 1;
            add_allow_build_keys(app_handle, &new_keys)?;
            log::info!("pnpm allowBuilds updated with {new_keys:?}, retrying {action} ({retries})");
            let _ = window.emit(
                PREINSTALL_LOG_EVENT,
                PreinstallLogPayload {
                    line: format!("[pnpm] 已放行插件构建（allowBuilds），重试{action}…"),
                },
            );
            continue;
        }
        if !new_keys.is_empty() {
            log::error!(
                "dsh plugin {action}: allowBuilds retry limit reached ({retries}), keys {new_keys:?} unresolved"
            );
            break if code == 0 { 1 } else { code };
        }
        if code != 0 {
            log::error!(
                "dsh plugin {action} failed with exit code {code}; no allowBuilds entries to add"
            );
        }
        break code;
    };
    Ok((exit_code, all_output, last_attempt))
}

/// 一层针对「reparse point 刚重建即被回读」的瞬时失败（issue #264）：Windows 下 pnpm
#[allow(clippy::too_many_arguments)]
async fn run_plugin_install_with_transient_retry(
    app_handle: &AppHandle,
    node: &Path,
    args: &[OsString],
    cwd: &Path,
    envs: &HashMap<String, String>,
    window: &WebviewWindow,
    action: &str,
    cancel: Option<&tokio::sync::watch::Receiver<bool>>,
    owner: ProcessOwner,
) -> Result<(i32, String, String), String> {
    let mut attempt = 0usize;
    let mut policy_attempt = 0usize;
    loop {
        let (exit_code, output, last_attempt) = run_plugin_with_allow_build_retry(
            app_handle, node, args, cwd, envs, window, action, cancel, owner,
        )
        .await?;
        if exit_code != 0
            && policy_attempt < POLICY_VERIFICATION_RETRIES
            && policy_verification_network_failure(&last_attempt)
        {
            policy_attempt += 1;
            let delay = policy_verification_retry_delay(policy_attempt);
            log::warn!(
                "dsh plugin {action} failed the lockfile supply-chain check because registry \
                 metadata could not be fetched; retrying ({policy_attempt}/{POLICY_VERIFICATION_RETRIES}) \
                 after {delay:?}"
            );
            let _ = window.emit(
                PREINSTALL_LOG_EVENT,
                PreinstallLogPayload {
                    line: format!(
                        "[harness] registry 元数据拉取失败，依赖校验未通过，正在重试（{policy_attempt}/{POLICY_VERIFICATION_RETRIES}）…"
                    ),
                },
            );
            if sleep_or_cancelled(delay, cancel).await {
                return Err(
                    "PLUGIN_OPERATION_CANCELLED: plugin operation was cancelled".to_string()
                );
            }
            continue;
        }
        if exit_code != 0
            && attempt < TRANSIENT_FS_RETRIES
            && is_transient_fs_install_failure(exit_code, &last_attempt)
        {
            attempt += 1;
            let delay = transient_fs_retry_delay(attempt);
            log::warn!(
                "dsh plugin {action} hit a transient filesystem error (exit code {exit_code}); \
                 retrying ({attempt}/{TRANSIENT_FS_RETRIES}) after {delay:?}"
            );
            let _ = window.emit(
                PREINSTALL_LOG_EVENT,
                PreinstallLogPayload {
                    line: format!(
                        "[harness] 插件安装遇到瞬时文件系统错误，正在重试（{attempt}/{TRANSIENT_FS_RETRIES}）…"
                    ),
                },
            );
            if sleep_or_cancelled(delay, cancel).await {
                return Err(
                    "PLUGIN_OPERATION_CANCELLED: plugin operation was cancelled".to_string()
                );
            }
            continue;
        }
        return Ok((exit_code, output, last_attempt));
    }
}

async fn sleep_or_cancelled(
    delay: std::time::Duration,
    cancel: Option<&tokio::sync::watch::Receiver<bool>>,
) -> bool {
    let Some(signal) = cancel else {
        tokio::time::sleep(delay).await;
        return false;
    };
    let mut signal = signal.clone();
    let probe = signal.clone();
    if *probe.borrow() {
        return true;
    }
    let timer = tokio::time::sleep(delay);
    tokio::pin!(timer);
    loop {
        tokio::select! {
            _ = &mut timer => return false,
            changed = signal.changed() => {
                if *probe.borrow() {
                    return true;
                }
                if changed.is_err() {
                    break;
                }
            }
        }
    }
    timer.await;
    false
}

fn is_transient_fs_install_failure(exit_code: i32, output: &str) -> bool {
    if exit_code == -4094 {
        return true;
    }
    let lower = output.to_ascii_lowercase();
    lower.contains("[unknown]") || lower.contains("unknown error")
}

pub(super) fn append_command_output(all_output: &mut String, captured: &str) {
    if captured.is_empty() {
        return;
    }
    if !all_output.is_empty() && !all_output.ends_with('\n') {
        all_output.push('\n');
    }
    all_output.push_str(captured);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_plugin_fixture(
        tag: &str,
        manifest: Option<&str>,
    ) -> (std::path::PathBuf, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!("dsh-local-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("plugin");
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(manifest) = manifest {
            std::fs::write(dir.join("package.json"), manifest).unwrap();
        }
        let base = dunce::simplified(&std::fs::canonicalize(&base).unwrap()).to_path_buf();
        let dir = base.join("plugin");
        (base, dir)
    }

    #[test]
    fn local_target_emits_link_absolute_spec_and_manifest_name() {
        let (base, dir) = local_plugin_fixture(
            "manifest",
            Some("{\"name\":\"dsh-reverse-skill\",\"version\":\"1.0.2\"}"),
        );

        let target = local_target(std::path::Path::new("./plugin"), &base).unwrap();

        assert_eq!(target.id, "dsh-reverse-skill");
        assert_eq!(target.name.as_deref(), Some("dsh-reverse-skill"));
        assert_eq!(target.spec, format!("link:{}", spec::forward_slashes(&dir)));
        assert!(target.spec.starts_with("link:"));
        assert!(
            !target.spec.contains('\\'),
            "pnpm 只接受正斜杠形态: {}",
            target.spec
        );
        assert!(!target.spec.ends_with('/'));
        assert!(
            std::path::Path::new(target.spec.trim_start_matches("link:")).is_absolute(),
            "相对输入必须绝对化：pnpm 的 cwd 是 dsh 安装目录，不是调用方目录: {}",
            target.spec
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn local_target_falls_back_to_directory_name_without_manifest() {
        let (base, dir) = local_plugin_fixture("bare", None);

        let target = local_target(std::path::Path::new("./plugin"), &base).unwrap();

        assert_eq!(target.id, "plugin");
        assert_eq!(target.name.as_deref(), Some("plugin"));
        assert_eq!(target.spec, format!("link:{}", spec::forward_slashes(&dir)));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn local_target_rejects_missing_directory_before_pnpm_runs() {
        let (base, _) = local_plugin_fixture("missing", None);

        let err = local_target(std::path::Path::new("./absent"), &base).unwrap_err();

        assert!(err.starts_with("PLUGIN_LOCAL_DIR_MISSING"), "{err}");
        assert!(err.contains("absent"), "错误里要给出解析后的路径: {err}");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn local_target_rejects_a_file_path() {
        let (base, dir) = local_plugin_fixture("file", None);
        let file = dir.join("index.js");
        std::fs::write(&file, "").unwrap();

        let err = local_target(&file, &base).unwrap_err();

        assert!(err.starts_with("PLUGIN_LOCAL_DIR_MISSING"), "{err}");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn local_target_honours_absolute_paths_verbatim() {
        let (base, _) = local_plugin_fixture("absolute", None);
        let (other_base, other_dir) = local_plugin_fixture("absolute-other", None);

        let target = local_target(&other_dir, &base).unwrap();

        assert_eq!(
            target.spec,
            format!("link:{}", spec::forward_slashes(&other_dir))
        );
        assert_ne!(
            target.spec,
            format!("link:{}", spec::forward_slashes(&base.join("plugin")))
        );

        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&other_base);
    }

    #[test]
    fn locked_release_age_exemptions_require_every_blocked_entry_to_be_locked() {
        let dir = std::env::temp_dir().join(format!("dsh-heal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("pnpm-lock.yaml"),
            "lockfileVersion: '9.0'\nimporters:\n  .:\n    dependencies:\n      dshmarket:\n        specifier: ^2.12.0\n        version: 2.12.0\npackages:\n  dshmarket@2.12.0: {}\n  transitive@1.0.0: {}\n  transitive@1.1.0: {}\n  '@scope/pkg@2.0.0(peer@1.0.0)': {}\n  transitive@1.2.0-extra: {}\n  missing@1.0.0suffix: {}\n",
        )
        .unwrap();

        let locked = PolicyBlockedVersion {
            name: "dshmarket".to_string(),
            version: "2.12.0".to_string(),
        };
        let newer = PolicyBlockedVersion {
            name: "dshmarket".to_string(),
            version: "2.13.0".to_string(),
        };
        let absent = PolicyBlockedVersion {
            name: "elsewhere".to_string(),
            version: "1.0.0".to_string(),
        };

        assert_eq!(
            locked_release_age_exemptions(&dir, &[locked.clone()]),
            Some(vec!["dshmarket@2.12.0".to_string()])
        );
        let transitive = PolicyBlockedVersion {
            name: "transitive".to_string(),
            version: "1.0.0".to_string(),
        };
        let second = PolicyBlockedVersion {
            name: "transitive".to_string(),
            version: "1.1.0".to_string(),
        };
        let scoped = PolicyBlockedVersion {
            name: "@scope/pkg".to_string(),
            version: "2.0.0".to_string(),
        };
        assert_eq!(
            locked_release_age_exemptions(&dir, &[transitive.clone(), second, scoped]),
            Some(vec![
                "transitive@1.0.0".to_string(),
                "transitive@1.1.0".to_string(),
                "@scope/pkg@2.0.0".to_string()
            ])
        );
        assert_eq!(
            locked_release_age_exemptions(&dir, &[transitive.clone(), newer.clone()]),
            None
        );
        assert_eq!(
            locked_release_age_exemptions(&dir, &[transitive, absent.clone()]),
            None
        );
        for (name, version) in [
            ("transitive", "1.2.0"),
            ("missing", "1.0.0"),
            ("@scope/pkg", "2.1.0"),
        ] {
            assert_eq!(
                locked_release_age_exemptions(
                    &dir,
                    &[PolicyBlockedVersion {
                        name: name.to_string(),
                        version: version.to_string()
                    }]
                ),
                None
            );
        }
        assert_eq!(locked_release_age_exemptions(&dir, &[newer]), None);
        assert_eq!(locked_release_age_exemptions(&dir, &[absent]), None);
        assert_eq!(locked_release_age_exemptions(&dir, &[]), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn command_output_retains_earlier_retry_diagnostics() {
        let mut output = String::new();
        append_command_output(&mut output, "ERR_PNPM_IGNORED_BUILDS");
        append_command_output(&mut output, "");

        assert_eq!(output, "ERR_PNPM_IGNORED_BUILDS");
    }

    #[test]
    fn transient_fs_failure_detects_uv_unknown_exit_code() {
        assert!(is_transient_fs_install_failure(-4094, ""));
        assert!(is_transient_fs_install_failure(
            -4094,
            "some unrelated output"
        ));
    }

    #[test]
    fn transient_fs_failure_detects_unknown_error_open_message() {
        // 与 issue #264 报告中一致的特征串：Node fs.open 通过刚重建的 junction
        let output = "[UNKNOWN] unknown error, open 'C:\\Users\\x\\.dsh\\profiles\\web\\node_modules\\dsh-tauri\\package.json'";
        assert!(is_transient_fs_install_failure(1, output));
        assert!(is_transient_fs_install_failure(
            1,
            &output.to_ascii_lowercase()
        ));
    }

    #[test]
    fn capped_backoff_preserves_zero_and_shift_limit_boundaries() {
        for (retry, base, cap, seconds) in [
            (0, 1, 64, 1),
            (0, 5, 30, 5),
            (3, 5, 30, 20),
            (63, 1, 64, 64),
            (65, 1, 64, 64),
            (65, 5, 30, 30),
        ] {
            assert_eq!(
                capped_backoff(retry, base, cap),
                std::time::Duration::from_secs(seconds),
                "retry={retry}, base={base}, cap={cap}"
            );
        }
    }

    #[test]
    fn transient_fs_retry_delay_uses_exponential_backoff() {
        assert_eq!(
            transient_fs_retry_delay(1),
            std::time::Duration::from_secs(1)
        );
        assert_eq!(
            transient_fs_retry_delay(2),
            std::time::Duration::from_secs(2)
        );
        assert_eq!(
            transient_fs_retry_delay(8),
            std::time::Duration::from_secs(64)
        );
    }

    #[test]
    fn policy_verification_retry_delay_uses_capped_exponential_backoff() {
        assert_eq!(
            policy_verification_retry_delay(1),
            std::time::Duration::from_secs(5)
        );
        assert_eq!(
            policy_verification_retry_delay(2),
            std::time::Duration::from_secs(10)
        );
        assert_eq!(
            policy_verification_retry_delay(4),
            std::time::Duration::from_secs(30)
        );
    }

    #[test]
    fn transient_fs_failure_rejects_ordinary_failures() {
        assert!(!is_transient_fs_install_failure(
            1,
            "ERR_PNPM_SPEC_NOT_SUPPORTED"
        ));
        assert!(!is_transient_fs_install_failure(
            254,
            "ENOENT: no such file"
        ));
        assert!(!is_transient_fs_install_failure(
            3,
            "ERR_PNPM_FETCH_404 registry error"
        ));
    }

    #[test]
    fn transient_fs_failure_treats_exit_zero_output_as_transient_detection_only() {
        assert!(is_transient_fs_install_failure(0, "unknown error, open"));
    }

    #[tokio::test]
    async fn sleep_or_cancelled_returns_immediately_when_cancelled() {
        let (tx, rx) = tokio::sync::watch::channel(false);
        let wait = sleep_or_cancelled(std::time::Duration::from_secs(30), Some(&rx));
        let cancel = async {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let _ = tx.send(true);
        };

        let (cancelled, ()) = tokio::join!(wait, cancel);

        assert!(cancelled);
    }

    #[tokio::test]
    async fn sleep_or_cancelled_waits_out_the_delay_without_a_cancel_signal() {
        let (_tx, rx) = tokio::sync::watch::channel(false);

        assert!(!sleep_or_cancelled(std::time::Duration::from_millis(10), Some(&rx)).await);
        assert!(!sleep_or_cancelled(std::time::Duration::from_millis(10), None).await);
    }

    #[tokio::test]
    async fn sleep_or_cancelled_ignores_false_updates_during_the_wait() {
        let (tx, rx) = tokio::sync::watch::channel(false);
        let started = std::time::Instant::now();
        let wait = sleep_or_cancelled(std::time::Duration::from_millis(150), Some(&rx));
        let noise = async {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let _ = tx.send(false);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let _ = tx.send(false);
        };

        let (cancelled, ()) = tokio::join!(wait, noise);

        assert!(!cancelled);
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(120),
            "false 唤醒不得缩短退避，实际等了 {:?}",
            started.elapsed()
        );
    }
}
