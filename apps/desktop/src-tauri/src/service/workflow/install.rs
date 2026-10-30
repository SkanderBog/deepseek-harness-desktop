use crate::config;
use crate::service::download;
use crate::service::download::Installable;

use super::process::{has_owned_process, stop, terminate_stale_harness_processes};
use crate::AppHandle;

pub async fn install(
    app_handle: &AppHandle,
    mut dsh_latest: Option<download::LatestDshPkg>,
) -> Result<bool, String> {
    let _transition = super::process::acquire_core_transition().await?;
    super::instances::require_exclusive_profile()?;
    log::info!("Starting installation process");
    let mut dsh_updated = false;

    if has_owned_process() {
        log::info!("Stopping running Harness service before installation");
        stop(app_handle.clone()).await?;
    }
    {
        let handle = app_handle.clone();
        let profile = crate::service::profile::active_profile(app_handle);
        tauri::async_runtime::spawn_blocking(move || {
            super::instances::scope_sync(profile, || terminate_stale_harness_processes(&handle))
        })
        .await
        .map_err(|e| format!("STOP_FAILED: {e}"))??;
    }

    let window = crate::utils::operation_window(app_handle)
        .ok_or("WINDOW_NOT_FOUND: current profile window missing")?;
    log::debug!("Main window obtained");
    let tasks = download::tasks();
    let dsh_missing = !download::Dsh.check_installed(app_handle);
    if dsh_latest.is_none() && dsh_missing {
        for attempt in 0..3 {
            let metadata = match config::recommended_dsh_version(app_handle) {
                Some(version) => download::fetch_dsh_pkg_version(app_handle, &version).await,
                None => download::fetch_latest_dsh_pkg_info(app_handle).await,
            };
            match metadata {
                Ok(info) => {
                    dsh_latest = Some(info);
                    break;
                }
                Err(e) if attempt < 2 => {
                    log::warn!(
                        "Retrying dsh release metadata fetch ({}/3), will retry: {}",
                        attempt + 1,
                        e
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(
                        500 * (attempt as u64 + 1),
                    ))
                    .await;
                }
                Err(e) => {
                    return Err(format!(
                        "DSH_INTEGRITY_UNAVAILABLE: 无法获取 Harness 发行版的完整性校验信息（{}），请检查网络后重试",
                        e
                    ));
                }
            }
        }
    }
    let mut tracker = download::ProgressTracker::new(&window, tasks.len() * 2);
    log::info!("Task list created, {} tasks total", tasks.len());

    for (index, task) in tasks.iter().enumerate() {
        let kind = task.kind();
        log::debug!("Processing task {}/{}", index + 1, tasks.len());
        let outdated = kind == download::InstallKind::Dsh
            && dsh_latest.as_ref().is_some_and(|info| {
                let installed_version = config::get_dsh_version(app_handle);
                let latest_version = download::parse_version_from_tag(&info.tag);
                let version_differs =
                    match (installed_version.as_deref(), latest_version.as_deref()) {
                        (Some(a), Some(b)) => a != b,
                        _ => false,
                    };
                version_differs
                    || !download::record_matches_latest_release(
                        config::get_dsh_pkg_commit(app_handle).as_deref(),
                        config::get_dsh_pkg_tag(app_handle).as_deref(),
                        info,
                    )
            });
        if task.check_installed(app_handle) && !outdated {
            log::debug!(
                "Task {} already installed and up to date, skipping",
                index + 1
            );
            task.record_mapping(app_handle);
            tracker.skip_phases(2);
            continue;
        }

        log::info!("Task {} not installed, starting installation", index + 1);

        tracker.start_phase(
            "download",
            &format!(
                "{} {}",
                config::i18n::t("install.downloading"),
                task.title()
            ),
        );
        let (urls, name) = if kind == download::InstallKind::Dsh {
            let primary = dsh_latest
                .as_ref()
                .map(|info| info.asset_url.clone())
                .filter(|url| !url.is_empty())
                .unwrap_or(config::get_dsh_download_url()?);
            let name = primary.rsplit('/').next().unwrap_or("").to_string();
            let mut urls = vec![primary.clone()];
            urls.extend(config::mirror_download_urls(&primary));
            (urls, name)
        } else {
            let url = task.get_download_url()?;
            let name = url.rsplit('/').next().unwrap_or("").to_string();
            (vec![url], name)
        };
        log::debug!("Download URL: {}", urls.join(" -> "));
        log::debug!("File name: {}", name);
        let buffer = download::download_file_from_sources(&tracker, urls).await?;
        log::info!("Download completed, file size: {} bytes", buffer.len());
        let expected_digest = match kind {
            download::InstallKind::Dsh => dsh_latest
                .as_ref()
                .and_then(|info| info.digest.clone())
                .ok_or_else(|| {
                    "DSH_INTEGRITY_UNAVAILABLE: trusted release digest is required".to_string()
                })?,
            download::InstallKind::Pnpm => config::PNPM_SHA256.to_string(),
        };
        download::verify_sha256(&buffer, &expected_digest)?;
        log::info!("Download integrity verified for task {}", index + 1);
        tracker.end_phase();

        tracker.start_phase(
            "extract",
            &format!("{} {}", config::i18n::t("install.extracting"), task.title()),
        );
        let dest = task.get_install_path(app_handle);
        log::debug!("Installation path: {:?}", dest);
        download::ensure_extract(&tracker, name, buffer, dest).await?;
        log::info!("Extraction completed");
        task.record_mapping(app_handle);
        tracker.end_phase();

        if kind == download::InstallKind::Dsh {
            dsh_updated = true;
            if let Some(info) = &dsh_latest {
                config::set_dsh_pkg_identity(app_handle, info.commit.clone(), info.tag.clone());
            }
        }
    }

    log::info!("All installation tasks completed");
    tracker.update(
        100.0,
        config::i18n::t("install.done"),
        "All tasks completed".into(),
    );

    Ok(dsh_updated)
}

#[cfg(test)]
mod tests {
    #[test]
    fn installation_holds_core_transition_before_checks_and_shared_file_changes() {
        let source = include_str!("install.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        let transition = source
            .find("let _transition = super::process::acquire_core_transition().await?;")
            .unwrap();
        for effect in [
            "super::instances::require_exclusive_profile()?",
            "if has_owned_process()",
            "download::ensure_extract",
        ] {
            assert!(
                transition < source.find(effect).unwrap(),
                "{effect} must be serialized with launch"
            );
        }
        assert!(!source.contains("drop(_transition)"));
    }
}
