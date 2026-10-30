use std::ffi::OsString;
use std::path::Path;

use crate::config;
use crate::service::cli;
use crate::service::core;
use crate::service::profile::active_profile;
use crate::service::workflow;

use super::artifact::{ensure_plugin_entry_built, installed_package_name};
use super::build_plugin_envs;
use super::diagnose::{
    IncompatibleVersion, PolicyBlockedVersion, git_transport_hint, incompatible_versions,
    network_error_hint, pick_error_message, policy_blocked_versions,
    policy_verification_network_failure, store_mismatch_hint,
};
use super::errors;
use super::harness_prefer_bundled_pnpm;
use super::is_actionable_plugin_ref;
use super::is_installed;
use super::new_process_owner;
use super::pnpm::ensure_pnpm;
use super::profile_dir;
use super::run_plugin_with_allow_build_retry;
use super::uninstall_recovery;
use crate::AppHandle;
use crate::service::plugin::update::known_latest;
use crate::service::profile::profile_release_age_excluded;

pub async fn update_many(app_handle: &AppHandle, specs: &[String]) -> Result<(), String> {
    let mut requested: Vec<(String, Option<String>)> =
        specs.iter().map(|spec| split_upgrade_spec(spec)).collect();
    if requested.iter().any(|(_, version)| version.is_none()) {
        resolve_missing_targets(app_handle, &mut requested).await;
    }
    let explicit: Vec<(String, String)> = requested
        .iter()
        .filter_map(|(id, version)| Some((id.clone(), version.clone()?)))
        .collect();
    let implicit: Vec<String> = requested
        .iter()
        .filter(|(_, version)| version.is_none())
        .map(|(id, _)| id.clone())
        .collect();
    let mut failures = Vec::new();
    if !explicit.is_empty() {
        failures.extend(install_targets(app_handle, &explicit).await);
    }
    if !implicit.is_empty() {
        failures.extend(update_to_latest(app_handle, &implicit).await);
    }
    match update_failure_payload(failures) {
        Some(message) => Err(message),
        None => Ok(()),
    }
}

async fn resolve_missing_targets(
    app_handle: &AppHandle,
    requested: &mut [(String, Option<String>)],
) {
    if let Err(e) = crate::service::plugin::update::refresh(app_handle).await {
        log::warn!("failed to probe plugin updates before upgrading: {e}");
        return;
    }
    let profile = profile_dir(app_handle);
    for (id, version) in requested.iter_mut() {
        if version.is_some() {
            continue;
        }
        let Some(latest) = known_latest(app_handle, id) else {
            continue;
        };
        if !is_registry_version(&latest)
            || installed_package_version(&profile, id).as_deref() == Some(latest.as_str())
        {
            continue;
        }
        *version = Some(latest);
    }
}

async fn install_targets(
    app_handle: &AppHandle,
    targets: &[(String, String)],
) -> Vec<UpdateFailure> {
    let profile = profile_dir(app_handle);
    let specs: Vec<String> = targets
        .iter()
        .map(|(id, version)| format!("{id}@{version}"))
        .collect();
    let ids: Vec<String> = targets.iter().map(|(id, _)| id.clone()).collect();
    let before: Vec<Option<String>> = ids
        .iter()
        .map(|id| dependency_fingerprint(&profile, id))
        .collect();
    if let Err(e) = run_plugin_command(app_handle, &ids, "add", &specs).await {
        return UpdateFailure::stage(&specs, e);
    }
    let mut failures = Vec::new();
    for (index, (id, version)) in targets.iter().enumerate() {
        if let Err(e) =
            verify_update_landed(app_handle, id, before[index].as_deref(), Some(version)).await
        {
            failures.push(UpdateFailure::new(id, e));
        }
    }
    failures
}

async fn update_to_latest(app_handle: &AppHandle, ids: &[String]) -> Vec<UpdateFailure> {
    let profile = profile_dir(app_handle);
    let before: Vec<Option<String>> = ids
        .iter()
        .map(|id| dependency_fingerprint(&profile, id))
        .collect();
    let args = update_pnpm_args(ids);
    if let Err(e) = run_plugin_command(app_handle, ids, "update", &args).await {
        return UpdateFailure::stage(ids, e);
    }
    let mut failures = Vec::new();
    let mut forced = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        match verify_update_landed(app_handle, id, before[index].as_deref(), None).await {
            Ok(()) => {}
            Err(e) => match force_upgrade_spec(id, &e) {
                Some(spec) => forced.push(spec),
                None => failures.push(UpdateFailure::new(id, e)),
            },
        }
    }
    if forced.is_empty() {
        return failures;
    }
    log::warn!("dsh plugin update was blocked by the declared source, forcing {forced:?}");
    let forced_ids: Vec<String> = forced
        .iter()
        .map(|spec| split_upgrade_spec(spec).0)
        .collect();
    if let Err(e) = run_plugin_command(app_handle, &forced_ids, "add", &forced).await {
        failures.extend(UpdateFailure::stage(&forced, e));
        return failures;
    }
    for (index, id) in ids.iter().enumerate() {
        let target = format!("{id}@");
        if !forced.iter().any(|spec| spec.starts_with(&target)) {
            continue;
        }
        if let Err(e) = verify_update_landed(app_handle, id, before[index].as_deref(), None).await {
            failures.push(UpdateFailure::new(id, e));
        }
    }
    failures
}

fn split_upgrade_spec(spec: &str) -> (String, Option<String>) {
    let Some((id, version)) = spec.rsplit_once('@') else {
        return (spec.to_string(), None);
    };
    if id.is_empty() {
        return (spec.to_string(), None);
    }
    if !is_registry_version(version) {
        return (id.to_string(), None);
    }
    (id.to_string(), Some(version.to_string()))
}

fn policy_refusal_from_specs(specs: &[String], failure: &str) -> Option<String> {
    if !failure.contains("ERR_PNPM_MINIMUM_RELEASE_AGE_VIOLATION")
        && !failure.contains("minimumReleaseAge")
    {
        return None;
    }
    let blocked: Vec<serde_json::Value> = specs
        .iter()
        .filter_map(|spec| {
            let (id, version) = split_upgrade_spec(spec);
            let version = version?;
            Some(serde_json::json!({ "name": id, "version": version }))
        })
        .collect();
    if blocked.is_empty() {
        return None;
    }
    Some(format!(
        "{POLICY_BLOCKED_PREFIX} {}",
        serde_json::Value::Array(blocked)
    ))
}

fn force_upgrade_spec(id: &str, failure: &str) -> Option<String> {
    let payload = failure.strip_prefix(UPDATE_HOLD_PREFIX)?;
    let payload: serde_json::Value = serde_json::from_str(payload).ok()?;
    if payload
        .get("retryable")
        .and_then(serde_json::Value::as_bool)
        != Some(false)
    {
        return None;
    }
    let latest = payload.get("latest").and_then(serde_json::Value::as_str)?;
    if !is_registry_version(latest) {
        return None;
    }
    Some(format!("{id}@{latest}"))
}

const UPDATE_HOLD_PREFIX: &str = "PLUGIN_UPDATE_NO_CHANGE:";

const INCOMPATIBLE_PREFIX: &str = "PLUGIN_VERSION_INCOMPATIBLE:";

const POLICY_BLOCKED_PREFIX: &str = "PLUGIN_POLICY_BLOCKED:";

const UPDATE_FAILED_PREFIX: &str = "PLUGIN_UPDATE_FAILED:";

struct UpdateFailure {
    id: String,
    message: String,
}

impl UpdateFailure {
    fn new(id: &str, message: String) -> Self {
        Self {
            id: id.to_string(),
            message,
        }
    }

    fn stage(specs: &[String], message: String) -> Vec<Self> {
        let message = policy_refusal_from_specs(specs, &message).unwrap_or(message);
        specs
            .iter()
            .map(|spec| Self {
                id: split_upgrade_spec(spec).0,
                message: message.clone(),
            })
            .collect()
    }
}

#[derive(serde::Serialize)]
struct UpdateStageFailure {
    name: String,
    message: String,
}

fn update_failure_payload(failures: Vec<UpdateFailure>) -> Option<String> {
    let mut incompatible = Vec::new();
    let mut blocked = Vec::new();
    let mut holds = Vec::new();
    let mut others = Vec::new();
    let push_blocked = |entries: Vec<PolicyBlockedVersion>,
                        blocked: &mut Vec<PolicyBlockedVersion>| {
        for entry in entries {
            if !blocked.contains(&entry) {
                blocked.push(entry);
            }
        }
    };
    let push_incompatible =
        |entries: Vec<IncompatibleVersion>, incompatible: &mut Vec<IncompatibleVersion>| {
            for entry in entries {
                if !incompatible.contains(&entry) {
                    incompatible.push(entry);
                }
            }
        };
    for failure in failures {
        if let Some(payload) = failure.message.strip_prefix(INCOMPATIBLE_PREFIX) {
            if let Ok(entries) = serde_json::from_str::<Vec<IncompatibleVersion>>(payload) {
                push_incompatible(entries, &mut incompatible);
                continue;
            }
        }
        if let Some(payload) = failure.message.strip_prefix(POLICY_BLOCKED_PREFIX) {
            if let Ok(entries) = serde_json::from_str::<Vec<PolicyBlockedVersion>>(payload) {
                push_blocked(entries, &mut blocked);
                continue;
            }
        }
        let policy = policy_blocked_versions(&failure.message);
        if !policy.is_empty() {
            push_blocked(policy, &mut blocked);
            continue;
        }
        let payload = failure
            .message
            .strip_prefix(UPDATE_HOLD_PREFIX)
            .and_then(|payload| serde_json::from_str::<serde_json::Value>(payload).ok());
        match payload {
            Some(value) => holds.push(value),
            None => others.push(UpdateStageFailure {
                name: failure.id,
                message: failure.message,
            }),
        }
    }
    let mut messages = Vec::new();
    if !incompatible.is_empty() {
        messages.push(format!(
            "{INCOMPATIBLE_PREFIX} {}",
            serde_json::to_string(&incompatible).unwrap_or_default()
        ));
    }
    if !blocked.is_empty() {
        messages.push(format!(
            "{POLICY_BLOCKED_PREFIX} {}",
            serde_json::to_string(&blocked).unwrap_or_default()
        ));
    }
    if !holds.is_empty() {
        messages.push(format!(
            "{UPDATE_HOLD_PREFIX} {}",
            serde_json::Value::Array(holds)
        ));
    }
    if !others.is_empty() {
        messages.push(format!(
            "{UPDATE_FAILED_PREFIX} {}",
            serde_json::to_string(&others).unwrap_or_default()
        ));
    }
    if messages.is_empty() {
        None
    } else {
        Some(messages.join("\n"))
    }
}

fn update_pnpm_args(ids: &[String]) -> Vec<String> {
    ids.iter()
        .flat_map(|id| [id.clone(), "--latest".to_string()])
        .collect()
}

fn is_registry_version(value: &str) -> bool {
    value.contains('.') && value.starts_with(|c: char| c.is_ascii_digit())
}

fn dependency_fingerprint(profile: &Path, id: &str) -> Option<String> {
    if let Some(entry) = lock_dependency_entry(profile, id) {
        return Some(entry);
    }
    installed_package_version(profile, id).map(|version| format!("{id}@{version}"))
}

fn lock_dependency_entry(profile: &Path, id: &str) -> Option<String> {
    lock_dependency(profile, id).map(|(specifier, version)| format!("{specifier} @ {version}"))
}

fn lock_dependency(profile: &Path, id: &str) -> Option<(String, String)> {
    let text = std::fs::read_to_string(profile.join("pnpm-lock.yaml")).ok()?;
    let lockfile: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
    let dependency = lockfile
        .get("importers")?
        .get(".")?
        .get("dependencies")?
        .get(id)?;
    let specifier = dependency
        .get("specifier")
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let version = dependency
        .get("version")
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some((specifier, version))
}

fn installed_package_version(profile: &Path, id: &str) -> Option<String> {
    let path = profile.join("node_modules").join(id).join("package.json");
    let content = std::fs::read_to_string(path).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&content).ok()?;
    manifest.get("version")?.as_str().map(String::from)
}

pub async fn remove_many(app_handle: &AppHandle, ids: &[String]) -> Result<(), String> {
    let command_result = run_plugin_command(app_handle, ids, "remove", ids).await;
    let mut failures = Vec::new();
    for id in ids {
        // （典型：安装只写入了 profile 清单而产物缺失，见 issue #90）pnpm 甚至会
        if is_installed(app_handle, id) {
            if is_actionable_plugin_ref(id) {
                let outcome = match &command_result {
                    Ok(()) => "reported success".to_string(),
                    Err(e) => format!("failed: {e}"),
                };
                log::warn!(
                    "dsh plugin remove {outcome} but {id} is still referenced by profile manifest; forcing offline uninstall"
                );
                match uninstall_recovery(app_handle, id) {
                    Ok(()) => {
                        if let Err(e) = errors::clear(app_handle, id) {
                            log::warn!("failed to clear plugin error for {id}: {e}");
                        }
                    }
                    Err(e) => failures.push(format!("{id}: {e}")),
                }
            } else {
                if let Err(e) = &command_result {
                    failures.push(format!("{id}: {e}"));
                }
                log::warn!(
                    "dsh plugin remove reported success but protected package {id} is still referenced by profile manifest; skipping offline uninstall"
                );
            }
        }
        // （issue #303：插件已移除则快照随之失效，避免残留孤儿快照占用存储）。
        if !is_installed(app_handle, id) {
            super::super::snapshot::delete_best_effort(app_handle, id);
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

/// pnpm 收到的就是 `pnpm remove remove <id>`（issue #715 日志中的 `pnpm remove
pub(super) fn single_plugin_args(
    profile: &str,
    action: &str,
    sub_args: &[String],
) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("plugin"),
        OsString::from("--profile"),
        OsString::from(profile),
        OsString::from(action),
    ];
    args.extend(sub_args.iter().map(OsString::from));
    args
}

async fn run_plugin_command(
    app_handle: &AppHandle,
    ids: &[String],
    action: &str,
    sub_args: &[String],
) -> Result<(), String> {
    if ids.is_empty() {
        return Err("PLUGIN_EMPTY_ID: plugin id is empty".to_string());
    }
    let window = crate::utils::operation_window(app_handle)
        .ok_or("WINDOW_NOT_FOUND: current profile window missing")?;

    cli::ensure_shims(app_handle)?;

    let node = config::get_node_binary_path(app_handle);
    let dsh_bin = core::active_dsh_binary(app_handle);
    if !node.exists() {
        return Err("NODE_NOT_FOUND: Node.js runtime missing".to_string());
    }
    if !crate::config::dependencies::binary_at_present(&dsh_bin) {
        return Err("HARNESS_NOT_FOUND: dsh CLI missing".to_string());
    }

    let owner = new_process_owner();
    let prefer_bundled_pnpm = ensure_pnpm(app_handle, &window, owner).await?;
    super::ensure_profile_npmrc(app_handle)?;
    // 否则升级/卸载触发 pnpm lockfile 校验时同样会被 issue #222 的问题阻断。
    super::ensure_profile_pnpm_policy(app_handle)?;
    // （issue #303：自动快照失败不阻塞主流程；还原入口在插件面板）。
    if action == "update" {
        for id in ids {
            super::super::snapshot::create_best_effort(app_handle, id);
        }
    }

    let envs = build_plugin_envs(app_handle, prefer_bundled_pnpm);

    let profile = active_profile(app_handle);
    let profile_path = crate::service::profile::profile_dir_of(app_handle, &profile);
    let mut args = vec![dsh_bin.as_os_str().to_os_string()];
    args.extend(single_plugin_args(&profile, action, sub_args));

    let cwd = crate::service::core::active_core_dir(app_handle);
    log::info!("Running dsh plugin {action} for {ids:?}");
    let (exit_code, output, last_attempt) = run_plugin_with_allow_build_retry(
        app_handle, &node, &args, &cwd, &envs, &window, action, None, owner,
    )
    .await?;

    let (exit_code, output, last_attempt) =
        if exit_code != 0 && super::heal_locked_release_age(&profile_path, &last_attempt) {
            run_plugin_with_allow_build_retry(
                app_handle, &node, &args, &cwd, &envs, &window, action, None, owner,
            )
            .await?
        } else {
            (exit_code, output, last_attempt)
        };

    if exit_code != 0 {
        log::error!("dsh plugin {action} failed for {ids:?} with exit code {exit_code}");
        let incompatible = incompatible_versions(&last_attempt);
        if !incompatible.is_empty() {
            log::warn!(
                "dsh refused the {action} for incompatible plugin versions: {incompatible:?}"
            );
            return Err(format!(
                "{INCOMPATIBLE_PREFIX} {}",
                serde_json::to_string(&incompatible).unwrap_or_default()
            ));
        }
        let policy_blocked = policy_blocked_versions(&last_attempt);
        if !policy_blocked.is_empty() {
            log::warn!(
                "pnpm release-age policy rejected {} profile entries during {action}: {policy_blocked:?}",
                policy_blocked.len()
            );
            return Err(format!(
                "PLUGIN_POLICY_BLOCKED: {}",
                serde_json::to_string(&policy_blocked).unwrap_or_default()
            ));
        }
        let hint = git_transport_hint(&last_attempt);
        let network_error = network_error_hint(&last_attempt).is_some()
            || policy_verification_network_failure(&last_attempt)
            || (exit_code == 3 && last_attempt.trim().is_empty());
        let store_hint = store_mismatch_hint(&last_attempt);
        let message = if network_error {
            "NETWORK_ERROR: plugin registry request failed; check network or proxy settings and retry."
                .to_string()
        } else if let Some(store_hint) = store_hint.as_deref() {
            store_hint.to_string()
        } else {
            pick_error_message(&output, hint)
        };
        for id in ids {
            if let Err(e) = errors::record(app_handle, id, action, &message) {
                log::warn!("failed to record plugin error for {id}: {e}");
            }
        }
        if network_error {
            return Err("NETWORK_ERROR: plugin registry request failed; check network or proxy settings and retry.".to_string());
        }
        if let Some(store_hint) = store_hint {
            log::warn!("pnpm store incompatibility detected during plugin {action}: {store_hint}");
            return Err(format!(
                "PLUGIN_{}_FAILED: dsh plugin exited with code {exit_code} ({store_hint})",
                action.to_uppercase()
            ));
        }
        return Err(format!(
            "PLUGIN_{}_FAILED: dsh plugin exited with code {exit_code}",
            action.to_uppercase()
        ));
    }

    for id in ids {
        if let Err(e) = errors::clear(app_handle, id) {
            log::warn!("failed to clear plugin error for {id}: {e}");
        }
    }
    if action == "remove" && ids.iter().any(|id| id == "dsh-win-terminal-inspector") {
        if let Err(e) = workflow::win_inspector::apply(app_handle) {
            log::warn!("win inspector patch prune failed after remove: {e}");
        }
    }
    log::info!("dsh plugin {action} succeeded for {ids:?}");
    Ok(())
}

async fn verify_update_landed(
    app_handle: &AppHandle,
    id: &str,
    before: Option<&str>,
    expected: Option<&str>,
) -> Result<(), String> {
    let profile = profile_dir(app_handle);
    match update_verify_step(&profile, id, before, expected, || {
        known_latest(app_handle, id)
    }) {
        UpdateVerifyStep::Hold { detail, latest } => {
            let retryable = latest.as_deref().is_some_and(|latest| {
                !profile_release_age_excluded(app_handle, &format!("{id}@{latest}"))
            });
            log::warn!(
                "dsh plugin update made no change for {id}, still at {detail}, newest {latest:?}, actionable {retryable}"
            );
            return Err(format!(
                "{UPDATE_HOLD_PREFIX} {}",
                serde_json::json!({
                    "name": id,
                    "version": detail,
                    "latest": latest,
                    "retryable": retryable,
                })
            ));
        }
        UpdateVerifyStep::Entry => {}
    }
    let Some(name) = installed_package_name(app_handle, id) else {
        log::warn!("plugin {id} not resolvable to a package name, skipping entry verify");
        return Ok(());
    };
    let window = crate::utils::operation_window(app_handle)
        .ok_or("WINDOW_NOT_FOUND: current profile window missing")?;
    let envs = build_plugin_envs(app_handle, harness_prefer_bundled_pnpm(app_handle).await?);
    let pkg_dir = profile_dir(app_handle).join("node_modules").join(name);
    if let Err(e) = ensure_plugin_entry_built(app_handle, id, &pkg_dir, &envs, &window).await {
        if let Err(err) = errors::record(app_handle, id, "update", &e) {
            log::warn!("failed to record plugin error for {id}: {err}");
        }
        return Err(e);
    }
    Ok(())
}

fn update_verify_step(
    profile: &Path,
    id: &str,
    before: Option<&str>,
    expected: Option<&str>,
    known_latest: impl Fn() -> Option<String>,
) -> UpdateVerifyStep {
    let reached = expected.is_some_and(|expected| {
        installed_package_version(profile, id).as_deref() == Some(expected)
    });
    if !reached {
        if let Some(before) = before {
            if dependency_fingerprint(profile, id).as_deref() == Some(before) {
                let detail =
                    installed_package_version(profile, id).unwrap_or_else(|| before.to_string());
                let latest = expected
                    .filter(|expected| *expected != detail && is_registry_version(expected))
                    .map(str::to_string)
                    .or_else(|| {
                        known_latest()
                            .filter(|latest| latest != &detail && is_registry_version(latest))
                    });
                log::warn!(
                    "dsh plugin update made no change for {id}, still at {detail}, newest {latest:?}"
                );
                return UpdateVerifyStep::Hold { detail, latest };
            }
        }
    }
    UpdateVerifyStep::Entry
}

enum UpdateVerifyStep {
    Hold {
        detail: String,
        latest: Option<String>,
    },
    Entry,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hold(name: &str, latest: serde_json::Value, retryable: bool) -> String {
        format!(
            "{UPDATE_HOLD_PREFIX} {}",
            serde_json::json!({"name": name, "version": "1.0.0", "latest": latest, "retryable": retryable})
        )
    }

    fn payload_of(message: &str, prefix: &str) -> serde_json::Value {
        let line = message
            .split('\n')
            .find(|line| line.starts_with(prefix))
            .unwrap_or_else(|| panic!("no {prefix} line in {message}"));
        serde_json::from_str(line.strip_prefix(prefix).expect("prefix")).expect("json payload")
    }

    #[test]
    fn update_failure_payload_merges_holds_into_one_array() {
        let message = update_failure_payload(vec![
            UpdateFailure::new("a", hold("a", serde_json::json!("2.0.0"), true)),
            UpdateFailure::new("b", hold("b", serde_json::json!(null), false)),
        ])
        .expect("message");

        let items = payload_of(&message, UPDATE_HOLD_PREFIX);
        let items = items.as_array().expect("array");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["name"], "a");
        assert_eq!(items[1]["retryable"], false);
    }

    #[test]
    fn update_failure_payload_keeps_real_failures_attributed() {
        let message = update_failure_payload(vec![
            UpdateFailure::new("a", "PLUGIN_ENTRY_MISSING: a is broken".to_string()),
            UpdateFailure::new("b", hold("b", serde_json::json!("2.0.0"), true)),
        ])
        .expect("message");

        let holds = payload_of(&message, UPDATE_HOLD_PREFIX);
        assert_eq!(holds.as_array().expect("holds").len(), 1);
        let failures = payload_of(&message, UPDATE_FAILED_PREFIX);
        let failures = failures.as_array().expect("failures");
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0]["name"], "a");
        assert_eq!(failures[0]["message"], "PLUGIN_ENTRY_MISSING: a is broken");
        assert!(update_failure_payload(Vec::new()).is_none());
    }

    #[test]
    fn stage_failures_name_every_target_of_that_stage() {
        let ids = vec!["aaa".to_string(), "bbb".to_string()];
        let message = update_failure_payload(UpdateFailure::stage(
            &ids,
            "NETWORK_ERROR: plugin registry request failed; check network or proxy settings and retry."
                .to_string(),
        ))
        .expect("message");

        let failures = payload_of(&message, UPDATE_FAILED_PREFIX);
        let failures = failures.as_array().expect("failures");
        assert_eq!(failures.len(), 2);
        assert_eq!(failures[0]["name"], "aaa");
        assert_eq!(failures[1]["name"], "bbb");
        assert_eq!(
            failures[1]["message"],
            "NETWORK_ERROR: plugin registry request failed; check network or proxy settings and retry."
        );
    }

    #[test]
    fn stage_policy_refusal_stays_authorizable() {
        let specs = vec!["aaa@1.2.3".to_string(), "bbb".to_string()];
        let message = update_failure_payload(UpdateFailure::stage(
            &specs,
            "ERR_PNPM_MINIMUM_RELEASE_AGE_VIOLATION  aaa@1.2.3 was published recently".to_string(),
        ))
        .expect("message");

        let blocked = payload_of(&message, POLICY_BLOCKED_PREFIX);
        assert_eq!(
            blocked,
            serde_json::json!([{"name": "aaa", "version": "1.2.3"}])
        );
    }

    #[test]
    fn forced_stage_failures_only_name_the_forced_targets() {
        let forced = vec!["bbb@2.0.0".to_string()];
        let message = update_failure_payload(UpdateFailure::stage(
            &forced,
            "PLUGIN_ADD_FAILED: dsh plugin exited with code 1".to_string(),
        ))
        .expect("message");

        let failures = payload_of(&message, UPDATE_FAILED_PREFIX);
        let failures = failures.as_array().expect("failures");
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0]["name"], "bbb");
    }

    #[test]
    fn pinned_upgrades_are_forced_only_after_authorisation() {
        let hold = |latest: &str, retryable: bool| {
            format!(
                "{UPDATE_HOLD_PREFIX} {}",
                serde_json::json!({"name": "a", "latest": latest, "retryable": retryable})
            )
        };

        assert_eq!(
            force_upgrade_spec("a", &hold("0.1.174", false)),
            Some("a@0.1.174".to_string())
        );
        assert_eq!(force_upgrade_spec("a", &hold("0.1.174", true)), None);
        assert_eq!(force_upgrade_spec("a", &hold("main", false)), None);
        assert_eq!(force_upgrade_spec("a", &hold("", false)), None);
        assert_eq!(
            force_upgrade_spec("a", "PLUGIN_ENTRY_MISSING: broken"),
            None
        );
    }

    #[test]
    fn upgrade_specs_split_versions_only_when_they_are_registry_versions() {
        assert_eq!(
            split_upgrade_spec("aaa@1.2.3"),
            ("aaa".to_string(), Some("1.2.3".to_string()))
        );
        assert_eq!(
            split_upgrade_spec("@scope/aaa@1.2.3"),
            ("@scope/aaa".to_string(), Some("1.2.3".to_string()))
        );
        assert_eq!(
            split_upgrade_spec("@scope/aaa"),
            ("@scope/aaa".to_string(), None)
        );
        assert_eq!(split_upgrade_spec("aaa@next"), ("aaa".to_string(), None));
        assert_eq!(
            split_upgrade_spec("github:owner/repo#main"),
            ("github:owner/repo#main".to_string(), None)
        );
    }

    #[test]
    fn policy_refusals_name_the_requested_targets() {
        let specs = vec!["aaa@1.2.3".to_string(), "bbb@2.0.0".to_string()];
        let failure = "ERR_PNPM_MINIMUM_RELEASE_AGE_VIOLATION  aaa@1.2.3 was published at ...";

        assert_eq!(
            policy_refusal_from_specs(&specs, failure),
            Some(
                "PLUGIN_POLICY_BLOCKED: [{\"name\":\"aaa\",\"version\":\"1.2.3\"},{\"name\":\"bbb\",\"version\":\"2.0.0\"}]"
                    .to_string()
            )
        );
        assert_eq!(
            policy_refusal_from_specs(&specs, "PLUGIN_ENTRY_MISSING: broken"),
            None
        );
        assert_eq!(
            policy_refusal_from_specs(&["aaa@next".to_string()], failure),
            None
        );
    }

    #[test]
    fn incompatible_refusals_reach_the_authorisation_parser() {
        let specs = vec!["dsh-plugin-guide@0.1.0".to_string()];
        let refusal = format!(
            "{INCOMPATIBLE_PREFIX} {}",
            serde_json::json!([{
                "name": "dsh-plugin-guide",
                "version": "0.1.0",
                "runtime_version": "0.2.0-rc.2",
            }])
        );

        let message =
            update_failure_payload(UpdateFailure::stage(&specs, refusal)).expect("message");

        assert!(message.starts_with(INCOMPATIBLE_PREFIX));
        assert_eq!(
            payload_of(&message, INCOMPATIBLE_PREFIX),
            serde_json::json!([{
                "name": "dsh-plugin-guide",
                "version": "0.1.0",
                "runtime_version": "0.2.0-rc.2",
            }])
        );
        assert!(!message.contains(UPDATE_FAILED_PREFIX));
    }

    #[test]
    fn incompatible_refusals_keep_the_other_failures_attributed() {
        let specs = vec!["aaa@1.0.0".to_string()];
        let refusal = format!(
            "{INCOMPATIBLE_PREFIX} {}",
            serde_json::json!([{"name": "aaa", "version": "1.0.0", "runtime_version": "0.2.0-rc.2"}])
        );
        let mut failures = UpdateFailure::stage(&specs, refusal);
        failures.push(UpdateFailure::new(
            "bbb",
            "PLUGIN_ENTRY_MISSING: bbb is broken".to_string(),
        ));

        let message = update_failure_payload(failures).expect("message");
        let lines: Vec<&str> = message.split('\n').collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with(INCOMPATIBLE_PREFIX));
        assert!(lines[1].starts_with(UPDATE_FAILED_PREFIX));

        let failures = payload_of(&message, UPDATE_FAILED_PREFIX);
        let failures = failures.as_array().expect("array");
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0]["name"], "bbb");
    }

    #[test]
    fn incompatible_refusals_are_deduplicated() {
        let specs = vec!["aaa@1.0.0".to_string(), "bbb@2.0.0".to_string()];
        let refusal = format!(
            "{INCOMPATIBLE_PREFIX} {}",
            serde_json::json!([{"name": "aaa", "version": "1.0.0", "runtime_version": "0.2.0-rc.2"}])
        );

        let message =
            update_failure_payload(UpdateFailure::stage(&specs, refusal)).expect("message");

        let entries = payload_of(&message, INCOMPATIBLE_PREFIX);
        assert_eq!(entries.as_array().expect("array").len(), 1);
    }

    #[test]
    fn update_args_request_latest() {
        assert_eq!(
            update_pnpm_args(&["dsh-better-sidebar".to_string()]),
            vec!["dsh-better-sidebar", "--latest"]
        );
    }

    #[test]
    fn registry_versions_are_recognized_by_shape() {
        assert!(is_registry_version("0.1.174"));
        assert!(is_registry_version("1.66.5"));
        assert!(!is_registry_version("next"));
        assert!(!is_registry_version(
            "b3a69187e1bac1bf6162e3d37d005e58bc2ee74e"
        ));
        assert!(!is_registry_version(""));
    }

    /// 回归 issue #715：`dsh plugin` 把动作之后的参数原样转发给 pnpm，动作动词因此
    #[test]
    fn single_plugin_args_keep_the_pnpm_verb_once() {
        let remove = single_plugin_args("tauri", "remove", &["dshmarket".to_string()]);
        assert_eq!(
            remove,
            vec!["plugin", "--profile", "tauri", "remove", "dshmarket"]
        );

        let update = single_plugin_args(
            "tauri",
            "update",
            &update_pnpm_args(&["dsh-better-sidebar".to_string()]),
        );
        assert_eq!(
            update,
            vec![
                "plugin",
                "--profile",
                "tauri",
                "update",
                "dsh-better-sidebar",
                "--latest"
            ]
        );

        for (action, args) in [("remove", &remove), ("update", &update)] {
            let joined: Vec<String> = args
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect();
            assert_eq!(
                joined.iter().filter(|argument| *argument == action).count(),
                1,
                "{action} must appear exactly once in {joined:?}"
            );
        }
    }

    fn probe_profile(label: &str, lock: &str, installed: Option<&str>) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dsh-plugin-fp-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("node_modules").join("dsh-probe")).unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), lock).unwrap();
        if let Some(version) = installed {
            std::fs::write(
                dir.join("node_modules")
                    .join("dsh-probe")
                    .join("package.json"),
                format!(r#"{{"name":"dsh-probe","version":"{version}"}}"#),
            )
            .unwrap();
        }
        dir
    }

    const LOCK_CATALOG: &str = "lockfileVersion: '9.0'\n\nimporters:\n\n  .:\n    dependencies:\n      dsh-probe:\n        specifier: 'catalog:'\n        version: 0.18.1\n";

    #[test]
    fn fingerprint_uses_current_importer_entry() {
        let dir = probe_profile("entry", LOCK_CATALOG, Some("0.18.1"));
        assert_eq!(
            dependency_fingerprint(&dir, "dsh-probe").as_deref(),
            Some("catalog: @ 0.18.1")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fingerprint_changes_when_resolved_version_moves() {
        let before = probe_profile("bump-before", LOCK_CATALOG, Some("0.18.1"));
        let after = probe_profile(
            "bump-after",
            &LOCK_CATALOG.replace("version: 0.18.1", "version: 0.19.0"),
            Some("0.19.0"),
        );
        assert_ne!(
            dependency_fingerprint(&before, "dsh-probe"),
            dependency_fingerprint(&after, "dsh-probe")
        );
        std::fs::remove_dir_all(&before).ok();
        std::fs::remove_dir_all(&after).ok();
    }

    #[test]
    fn fingerprint_detects_git_commit_change_with_stable_version() {
        let lock = |sha: &str| {
            format!(
                "lockfileVersion: '9.0'\n\nimporters:\n\n  .:\n    dependencies:\n      dsh-probe:\n        specifier: 'catalog:'\n        version: https://codeload.github.com/o/r/tar.gz/{sha}\n"
            )
        };
        let before = probe_profile("git-before", &lock("aaaaaaaaaaaaaaaa"), Some("0.2.21"));
        let after = probe_profile("git-after", &lock("bbbbbbbbbbbbbbbb"), Some("0.2.21"));
        assert_ne!(
            dependency_fingerprint(&before, "dsh-probe"),
            dependency_fingerprint(&after, "dsh-probe")
        );
        std::fs::remove_dir_all(&before).ok();
        std::fs::remove_dir_all(&after).ok();
    }

    #[test]
    fn fingerprint_falls_back_to_installed_version() {
        let dir = probe_profile(
            "fallback",
            "lockfileVersion: '9.0'\n\nimporters:\n\n  .:\n    dependencies: {}\n",
            Some("0.18.1"),
        );
        assert_eq!(
            dependency_fingerprint(&dir, "dsh-probe").as_deref(),
            Some("dsh-probe@0.18.1")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fingerprint_is_none_when_nothing_is_readable() {
        let dir = probe_profile("unreadable", "", None);
        assert_eq!(dependency_fingerprint(&dir, "dsh-probe"), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reached_target_version_still_verifies_the_entry() {
        let dir = probe_profile("reached", LOCK_CATALOG, Some("0.19.0"));
        let fingerprint = dependency_fingerprint(&dir, "dsh-probe");
        assert_eq!(fingerprint.as_deref(), Some("catalog: @ 0.18.1"));
        let step = update_verify_step(
            &dir,
            "dsh-probe",
            fingerprint.as_deref(),
            Some("0.19.0"),
            || None,
        );
        assert!(
            matches!(step, UpdateVerifyStep::Entry),
            "版本已达目标必须继续核验入口"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unchanged_fingerprint_without_the_target_is_still_a_hold() {
        let dir = probe_profile("hold", LOCK_CATALOG, Some("0.18.1"));
        let fingerprint = dependency_fingerprint(&dir, "dsh-probe");
        let step = update_verify_step(
            &dir,
            "dsh-probe",
            fingerprint.as_deref(),
            Some("0.19.0"),
            || None,
        );
        match step {
            UpdateVerifyStep::Hold { detail, latest } => {
                assert_eq!(detail, "0.18.1");
                assert_eq!(latest.as_deref(), Some("0.19.0"));
            }
            UpdateVerifyStep::Entry => panic!("指纹未变且目标未达成时必须上报没落地"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hold_falls_back_to_the_probed_latest_version() {
        let dir = probe_profile("fallback-latest", LOCK_CATALOG, Some("0.18.1"));
        let fingerprint = dependency_fingerprint(&dir, "dsh-probe");
        let step = update_verify_step(
            &dir,
            "dsh-probe",
            fingerprint.as_deref(),
            Some("https://codeload.github.com/o/r/tar.gz/aaaaaaaaaaaaaaaa"),
            || Some("0.19.0".to_string()),
        );
        match step {
            UpdateVerifyStep::Hold { latest, .. } => {
                assert_eq!(latest.as_deref(), Some("0.19.0"));
            }
            UpdateVerifyStep::Entry => panic!("指纹未变且目标未达成时必须上报没落地"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn changed_fingerprint_goes_straight_to_entry_verification() {
        let dir = probe_profile("moved", LOCK_CATALOG, Some("0.19.0"));
        let step = update_verify_step(
            &dir,
            "dsh-probe",
            Some("catalog: @ 0.17.0"),
            Some("0.20.0"),
            || None,
        );
        assert!(matches!(step, UpdateVerifyStep::Entry));
        std::fs::remove_dir_all(&dir).ok();
    }
}
