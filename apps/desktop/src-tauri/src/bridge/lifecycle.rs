use std::sync::OnceLock;

use crate::config;
use crate::service::cli;
use crate::service::core;
use crate::service::download::{self, Installable};
use crate::service::workflow;
use crate::{AppHandle, WebviewWindow};

static INSTALL_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn active_dsh_version(app_handle: &AppHandle) -> Option<String> {
    core::active_version(app_handle).or_else(|| config::get_dsh_version(app_handle))
}

fn version_order(candidate: Option<&str>, baseline: Option<&str>) -> Option<std::cmp::Ordering> {
    let candidate = semver::Version::parse(candidate?).ok()?;
    let baseline = semver::Version::parse(baseline?).ok()?;
    Some(candidate.cmp(&baseline))
}

fn is_update_suppressed(latest_version: Option<&str>, active_version: Option<&str>) -> bool {
    matches!(
        version_order(latest_version, active_version),
        Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
    )
}

fn install_lock() -> &'static tokio::sync::Mutex<()> {
    INSTALL_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn reset_install_status(app_handle: &AppHandle) {
    workflow::status::set_status(workflow::status::Status::Stopped);
    workflow::status::emit_status(app_handle);
}

fn sync_cli_link(app_handle: &AppHandle) {
    let setting = config::get_store_dat_setting(app_handle);
    let result = if setting.cli_link_enabled {
        cli::ensure(app_handle)
    } else {
        cli::remove(app_handle)
    };
    if let Err(e) = result {
        log::warn!("cli link sync failed: {e}");
    }
}

#[tauri::command]
pub async fn install_dependencies(
    window: WebviewWindow,
    app_handle: AppHandle,
) -> Result<bool, String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {

    let Ok(_install_guard) = install_lock().try_lock() else {
        log::info!("Installation process already running, skipping");
        return Ok(false);
    };

    {
        let _transition = workflow::acquire_core_transition().await?;
        workflow::instances::require_exclusive_profile()?;
        if workflow::has_owned_process() {
            workflow::stop(app_handle.clone()).await?;
        }
        let handle = app_handle.clone();
        let profile = crate::service::profile::active_profile(&app_handle);
        tauri::async_runtime::spawn_blocking(move || {
            workflow::instances::scope_sync(profile, || {
                workflow::terminate_stale_harness_processes(&handle)
            })
        })
        .await
        .map_err(|error| format!("CORE_CLEANUP_STOP_FAILED: {error}"))??;
        let root = config::dependencies::managed_root(&app_handle, config::dependencies::DEP_DSH);
        let base = config::get_base_dir(&app_handle);
        let mut protected = vec![crate::config::get_dsh_data_path(&app_handle)];
        if let Some(resources) = config::manifest::resource_root(&app_handle) {
            protected.push(resources);
        }
        core::cleanup_active_folder_core(&root, &root, &protected).await?;
        let deps = base.join("dependencies");
        let active = root.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("core-slot-cleanup".into())
            .spawn(move || {
                tauri::async_runtime::block_on(core::cleanup_other_folder_cores(
                    &deps,
                    &active,
                    &protected,
                ));
            })
        {
            log::warn!("CORE_CLEANUP_BACKGROUND_FAILED: {error}");
        }
    }

    let dsh_files_ok = download::Dsh.check_installed(&app_handle);
    let pnpm_ok = download::Pnpm.check_installed(&app_handle);

    download::record_mappings(&app_handle);

    if dsh_files_ok && pnpm_ok {
        let setting = config::get_store_dat_setting(&app_handle);
        if !setting.installed {
            log::info!(
                "Runtime files already present although store says not installed, healing installed flag"
            );
            let mut setting = config::get_store_dat_setting(&app_handle);
            setting.installed = true;
            config::set_store_dat_setting(&app_handle, setting);
            sync_cli_link(&app_handle);
            return Ok(false);
        }
    }

    let recommended_version = config::recommended_dsh_version(&app_handle);
    let installed_version = dsh_files_ok
        .then(|| active_dsh_version(&app_handle))
        .flatten();
    let preserve_installed =
        version_order(installed_version.as_deref(), recommended_version.as_deref())
            == Some(std::cmp::Ordering::Greater);
    let dsh_latest = if preserve_installed {
        log::info!(
            "Keeping installed dsh version above recommendation: {}",
            installed_version.as_deref().unwrap_or_default()
        );
        None
    } else {
        Some(match recommended_version {
            Some(version) => download::fetch_dsh_pkg_version(&app_handle, &version).await,
            None => download::fetch_latest_dsh_pkg_info(&app_handle).await,
        })
    };

    let dsh_need_install = match dsh_latest.as_ref() {
        None => false,
        Some(Ok(latest)) if dsh_files_ok => {
            let record_commit = config::get_dsh_pkg_commit(&app_handle);
            let record_tag = config::get_dsh_pkg_tag(&app_handle);
            let legacy_tags = if record_tag.is_none() {
                download::fetch_dsh_pkg_tags(&app_handle).await.unwrap_or_default()
            } else {
                Vec::new()
            };
            match download::resolve_update(
                record_commit.as_deref(),
                record_tag.as_deref(),
                installed_version.as_deref(),
                latest,
                &legacy_tags,
            ) {
                // 吞掉核心更新提示（issue #790），所以 commit 相符也要补齐 tag。
                download::UpdateCheck::UpToDate | download::UpdateCheck::HealUpToDate => {
                    if record_tag.as_deref() != Some(latest.tag.as_str())
                        || record_commit.as_deref() != Some(latest.commit.as_str())
                    {
                        log::info!(
                            "Installed Harness files already at latest release, healing stale record: {} ({})",
                            latest.tag,
                            latest.commit
                        );
                        config::set_dsh_pkg_identity(
                            &app_handle,
                            latest.commit.clone(),
                            latest.tag.clone(),
                        );
                    }
                    false
                }
                download::UpdateCheck::UpdateAvailable => {
                    if latest.digest.is_none() {
                        log::warn!(
                            "New dsh release {} found but trusted digest unavailable (API rate-limited), keeping local install",
                            latest.tag
                        );
                        false
                    } else {
                        true
                    }
                }
            }
        }
        Some(Ok(_)) => true,
        Some(Err(e)) => {
            log::warn!(
                "Failed to check latest dsh release info, keeping local install: {}",
                e
            );
            !dsh_files_ok
        }
    };

    if !dsh_need_install && pnpm_ok {
        log::info!("Dependencies already installed and up to date, skipping installation");
        let mut setting = config::get_store_dat_setting(&app_handle);
        if !setting.installed {
            setting.installed = true;
            config::set_store_dat_setting(&app_handle, setting);
        }
        sync_cli_link(&app_handle);
        return Ok(false);
    }

    log::info!("Dependencies missing or outdated, starting installation process");
    workflow::status::set_status(workflow::status::Status::Installing);
    workflow::status::emit_status(&app_handle);
    let install_target = dsh_latest.and_then(Result::ok);
    let updated = match workflow::install(&app_handle, install_target).await {
        Ok(updated) => updated,
        Err(e) => {
            log::error!("Installation failed, resetting status: {e}");
            reset_install_status(&app_handle);
            return Err(e);
        }
    };
    log::debug!("Installation completed, marked as installed");
    let mut setting = config::get_store_dat_setting(&app_handle);
    setting.installed = true;
    config::set_store_dat_setting(&app_handle, setting);
    sync_cli_link(&app_handle);
    Ok(updated)

    }).await
}

#[tauri::command]
pub async fn check_dsh_update(
    app_handle: AppHandle,
) -> Result<Option<download::LatestDshPkg>, String> {
    let dsh_files_ok = download::Dsh.check_installed(&app_handle);
    if !dsh_files_ok {
        return Ok(None);
    }

    if config::get_store_dat_setting(&app_handle)
        .active_core
        .as_deref()
        == Some("app")
        && config::get_dsh_pkg_tag(&app_handle)
            .as_deref()
            .is_some_and(download::is_preview_tag)
    {
        log::info!("Suppressing dsh update because a preview core is active");
        return Ok(None);
    }

    let latest = download::fetch_latest_dsh_pkg_info(&app_handle).await?;

    if let Some(version) = download::parse_version_from_tag(&latest.tag) {
        if core::has_installed_version(&app_handle, &version).await {
            log::info!(
                "Suppressing dsh update toast because latest version is already installed: {}",
                version
            );
            return Ok(None);
        }
    }

    if let Some(version) = download::parse_version_from_tag(&latest.tag) {
        if config::is_dsh_version_above_recommended(&app_handle, &version) {
            log::info!(
                "Suppressing dsh update toast because latest is above recommended (preview/alpha): {}",
                version
            );
            return Ok(None);
        }
    }

    if let Some(version) = download::parse_version_from_tag(&latest.tag) {
        let active = active_dsh_version(&app_handle);
        if is_update_suppressed(Some(&version), active.as_deref()) {
            log::info!(
                "Suppressing dsh update toast because latest {} does not outrank the active core {}",
                version,
                active.as_deref().unwrap_or_default()
            );
            return Ok(None);
        }
    }

    Ok(Some(latest))
}

#[tauri::command]
pub async fn launch_harness(window: WebviewWindow, app_handle: AppHandle) -> Result<(), String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {
        workflow::launch(app_handle).await
    })
    .await
}

#[tauri::command]
pub async fn shutdown_harness(window: WebviewWindow, app_handle: AppHandle) -> Result<(), String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(
        profile,
        async move { workflow::stop(app_handle).await },
    )
    .await
}

#[tauri::command]
pub async fn restart_harness(window: WebviewWindow, app_handle: AppHandle) -> Result<(), String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {
        workflow::restart(app_handle).await
    })
    .await
}

/// 引号）会让安全模式也起不来，把兜底入口一起废掉（issue #525）。这里在切档案前把
#[tauri::command]
pub async fn enter_safe_mode(
    window: WebviewWindow,
    app_handle: AppHandle,
) -> Result<crate::service::plugin::PatchQuarantineReport, String> {
    let _transition = crate::service::workflow::acquire_core_transition().await?;
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {
        crate::service::profile::ensure_safe_profile(&app_handle)?;
        let report = crate::service::plugin::quarantine_patch_layers_in(
            &crate::service::profile::profile_dir_of(
                &app_handle,
                crate::service::profile::SAFE_PROFILE,
            ),
            &config::get_dsh_data_path(&app_handle),
        );
        if report.has_failures() {
            return Err(crate::service::plugin::quarantine_failure_message(&report));
        }
        let old = crate::service::profile::active_profile(&app_handle);
        crate::service::profile::set_active(&app_handle, crate::service::profile::SAFE_PROFILE)?;
        crate::service::workflow::instances::bind_window(
            window.label(),
            crate::service::profile::SAFE_PROFILE,
        );
        if !crate::service::workflow::instances::is_attached(&old) {
            workflow::stop(app_handle.clone()).await?;
        }
        Ok(report)
    })
    .await
}

#[tauri::command]
pub async fn quarantine_broken_patch_layers(
    window: WebviewWindow,
    app_handle: AppHandle,
) -> Result<crate::service::plugin::PatchQuarantineReport, String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {
        let report = crate::service::plugin::quarantine_active_patch_layers(&app_handle);
        if report.has_failures() {
            return Err(crate::service::plugin::quarantine_failure_message(&report));
        }
        Ok(report)
    })
    .await
}

#[tauri::command]
pub async fn strip_unresolved_patch_entries(
    window: WebviewWindow,
    app_handle: AppHandle,
) -> Result<crate::service::plugin::PatchEntryStripReport, String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {
        crate::service::plugin::strip_active_unresolved_entries(&app_handle)
    })
    .await
}

#[tauri::command]
pub fn get_dsh_status(app_handle: AppHandle, window: WebviewWindow) -> workflow::status::Status {
    let profile = workflow::instances::window_profile(&app_handle, window.label());
    workflow::instances::scope_sync(profile, workflow::status::get_status)
}

#[tauri::command]
pub fn runtime_ready(app_handle: AppHandle) -> bool {
    download::Dsh.check_installed(&app_handle) && download::Pnpm.check_installed(&app_handle)
}

#[cfg(test)]
mod tests {
    use super::{install_lock, is_update_suppressed, version_order};
    use std::cmp::Ordering;

    #[test]
    fn older_latest_release_does_not_outrank_active_core() {
        assert!(is_update_suppressed(Some("0.1.7-rc.2"), Some("0.2.0-rc.1")));
        assert!(is_update_suppressed(Some("0.2.0-rc.1"), Some("0.2.0-rc.1")));
        assert!(!is_update_suppressed(
            Some("0.2.0-rc.1"),
            Some("0.1.7-rc.2")
        ));
        assert_eq!(
            version_order(Some("0.1.7-rc.10"), Some("0.1.7-rc.9")),
            Some(Ordering::Greater)
        );
        assert!(!is_update_suppressed(None, Some("0.2.0-rc.1")));
        assert!(!is_update_suppressed(Some("0.2.0-rc.1"), None));
        assert!(!is_update_suppressed(
            Some("not-a-version"),
            Some("0.2.0-rc.1")
        ));
    }

    #[test]
    fn install_lock_is_exclusive_while_held() {
        let lock = install_lock();
        let guard = lock.try_lock();
        assert!(guard.is_ok());
        assert!(lock.try_lock().is_err());
        drop(guard);
        assert!(lock.try_lock().is_ok());
    }

    #[test]
    fn newer_installed_dsh_is_preserved_from_recommended_downgrade() {
        let preserve = |installed: &str, recommended: &str| {
            version_order(Some(installed), Some(recommended)) == Some(Ordering::Greater)
        };
        assert!(preserve("0.1.1-rc.3", "0.1.1-rc.2"));
        assert!(!preserve("0.1.1-rc.2", "0.1.1-rc.2"));
        assert!(!preserve("0.1.1-rc.1", "0.1.1-rc.2"));
    }
}
