
use crate::config;
use crate::service::{download, fs_guard, workflow};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::local::{find_user_dsh_bin, local_core};
use super::source::{
    active_is_bundled, active_source, core_supports_bundled_plugins, CoreSource, HarnessCore,
};
use crate::{AppHandle};

const BUNDLED_CORE_ID: &str = "app-bundled";

fn dependencies_dir(app_handle: &AppHandle) -> PathBuf {
    config::get_base_dir(app_handle).join("dependencies")
}

fn slot_dir(app_handle: &AppHandle, tag: &str) -> PathBuf {
    dependencies_dir(app_handle).join(tag)
}

fn existing_slot_dir(app_handle: &AppHandle, tag: &str) -> Option<PathBuf> {
    let deps = dependencies_dir(app_handle);
    let slot = safe_slot_path(&deps, tag).ok()?;
    slot.is_dir().then_some(slot)
}

fn safe_slot_path(deps: &Path, tag: &str) -> Result<PathBuf, String> {
    fs_guard::validate_id(tag)?;
    let path = deps.join(tag);
    if path.exists() {
        fs_guard::ensure_within(&path, deps)?;
    }
    Ok(path)
}

fn read_manifest_dsh_version(dir: &Path) -> Option<String> {
    let content = std::fs::read_to_string(dir.join("package.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("dependencies")?
        .get("@deepseek-ai/dsh")?
        .as_str()
        .map(|s| s.trim_start_matches(['^', '~', '=', '>', '<']).to_string())
}

pub async fn list(app_handle: &AppHandle) -> Vec<HarnessCore> {
    let (release_metas, remote_catalog_available) = fetch_release_catalog(app_handle).await;
    rows_with_release_catalog(app_handle, release_metas, remote_catalog_available)
}

async fn fetch_release_catalog(app_handle: &AppHandle) -> (Vec<download::DshPkgReleaseMeta>, bool) {
    match download::fetch_dsh_pkg_releases(app_handle).await {
        Ok(metas) => (metas, true),
        Err(e) => {
            log::warn!(
                "Failed to fetch dsh pkg releases ({}), falling back to git tags",
                e
            );
            match download::fetch_dsh_pkg_tags(app_handle).await {
                Ok(tags) => (
                    tags.into_iter()
                        .map(|(tag, _)| download::DshPkgReleaseMeta {
                            tag,
                            prerelease: false,
                        })
                        .collect(),
                    true,
                ),
                Err(e) => {
                    log::warn!("Failed to fetch dsh pkg tags: {}", e);
                    (Vec::new(), false)
                }
            }
        }
    }
}

fn list_local(app_handle: &AppHandle) -> Vec<HarnessCore> {
    rows_with_release_catalog(app_handle, Vec::new(), false)
}

fn rows_with_release_catalog(
    app_handle: &AppHandle,
    release_metas: Vec<download::DshPkgReleaseMeta>,
    remote_catalog_available: bool,
) -> Vec<HarnessCore> {
    let source = active_source(app_handle);
    let local = local_core(app_handle);
    let local_bin = local
        .as_ref()
        .map(|c| c.bin.to_string_lossy().into_owned())
        .or_else(|| find_user_dsh_bin(app_handle).map(|b| b.to_string_lossy().into_owned()));

    let mut rows: Vec<HarnessCore> = vec![HarnessCore {
        id: "local".to_string(),
        source: CoreSource::Local,
        version: local
            .as_ref()
            .map(|c| c.version.clone())
            .unwrap_or_default(),
        tag: String::new(),
        path: local_bin.clone().unwrap_or_default(),
        dir: local
            .as_ref()
            .map(|c| c.package_dir.to_string_lossy().into_owned())
            .unwrap_or_default(),
        present: local.is_some(),
        active: source == CoreSource::Local,
        removable: false,
        preview: false,
        above_recommended: local
            .as_ref()
            .is_some_and(|c| config::is_dsh_version_above_recommended(app_handle, &c.version)),
        orphaned: false,
        bundled: false,
        recommended_version: config::recommended_dsh_version(app_handle),
        error: None,
    }];

    let bundled_dir = config::dependencies::bundled_core_dir(app_handle);
    let bundled_active = active_is_bundled(app_handle);
    if let Some(dir) = &bundled_dir {
        let version = read_manifest_dsh_version(dir).unwrap_or_default();
        let dir_str = dir.to_string_lossy().into_owned();
        rows.insert(
            0,
            HarnessCore {
                id: BUNDLED_CORE_ID.to_string(),
                source: CoreSource::App,
                above_recommended: config::is_dsh_version_above_recommended(app_handle, &version),
                version,
                tag: String::new(),
                path: dir_str.clone(),
                dir: dir_str,
                present: dir
                    .join(config::dependencies::entry_relative(
                        app_handle,
                        config::dependencies::DEP_DSH,
                    ))
                    .is_file(),
                active: bundled_active,
                removable: false,
                preview: false,
                orphaned: false,
                bundled: true,
                recommended_version: config::recommended_dsh_version(app_handle),
                error: None,
            },
        );
    }

    let active_tag = config::get_dsh_pkg_tag(app_handle);
    let active_dir = config::get_dsh_install_path(app_handle);
    let active_present = config::get_dsh_binary_path(app_handle).exists();
    let manifest_version = config::get_dsh_version(app_handle).or_else(|| {
        (source == CoreSource::App && active_present)
            .then(|| read_manifest_dsh_version(&active_dir))
            .flatten()
    });
    // （CORE_VERSION_NOT_FOUND），并吞掉核心更新提示（issue #790）。
    let installed_release = (!bundled_active)
        .then(|| trusted_release_version(active_tag.as_deref(), manifest_version.as_deref()))
        .flatten();

    let mut version_tags: Vec<(String, String, bool)> = Vec::new(); // (version, tag, preview)，保持首次出现顺序
    for meta in &release_metas {
        let Some(version) = download::parse_version_from_tag(&meta.tag) else {
            continue;
        };
        let preview = meta.prerelease || download::is_preview_tag(&meta.tag);
        if let Some(entry) = version_tags.iter_mut().find(|(v, _, _)| v == &version) {
            entry.1 = meta.tag.clone();
            entry.2 = preview;
        } else {
            version_tags.push((version, meta.tag.clone(), preview));
        }
    }

    version_tags.sort_by(|(a, _, _), (b, _, _)| {
        match (semver::Version::parse(a), semver::Version::parse(b)) {
            (Ok(a), Ok(b)) => b.cmp(&a),
            _ => b.cmp(a),
        }
    });

    let mut active_rendered = false;
    for (version, catalog_tag, preview) in &version_tags {
        // "已安装"，避免本地核心出现后预打包被当作未下载/消失（issue #54）。
        let in_place = installed_release.as_deref() == Some(version.as_str());
        let is_active = in_place && source == CoreSource::App;
        // 卸载会找不到目录（issue #790）。
        let tag = if in_place {
            active_tag.as_deref().unwrap_or(catalog_tag.as_str())
        } else {
            catalog_tag.as_str()
        };
        if in_place {
            active_rendered = true;
        }
        let slot = existing_slot_dir(app_handle, tag);
        let removable = !in_place && slot.is_some();
        let (present, path, dir) = if in_place {
            let s = active_dir.to_string_lossy().into_owned();
            (active_present, s.clone(), s)
        } else if let Some(slot) = slot {
            let s = slot.to_string_lossy().into_owned();
            (true, s.clone(), s)
        } else {
            (false, String::new(), String::new())
        };
        rows.push(HarnessCore {
            id: format!("app-{tag}"),
            source: CoreSource::App,
            version: version.clone(),
            tag: tag.to_string(),
            path,
            dir,
            present,
            active: is_active,
            removable,
            preview: *preview,
            above_recommended: config::is_dsh_version_above_recommended(app_handle, version),
            orphaned: false,
            bundled: false,
            recommended_version: config::recommended_dsh_version(app_handle),
            error: None,
        });
    }

    // `set_active("app")`。它没有槽位目录，因此既不列 tag 也不提供卸载（issue #790）。
    if !active_rendered && active_present && !bundled_active {
        rows.push(HarnessCore {
            id: "app".to_string(),
            source: CoreSource::App,
            version: manifest_version.clone().unwrap_or_default(),
            tag: String::new(),
            path: active_dir.to_string_lossy().into_owned(),
            dir: active_dir.to_string_lossy().into_owned(),
            present: true,
            active: source == CoreSource::App,
            removable: false,
            orphaned: false,
            bundled: false,
            preview: active_tag.as_deref().is_some_and(download::is_preview_tag),
            above_recommended: manifest_version
                .as_deref()
                .is_some_and(|v| config::is_dsh_version_above_recommended(app_handle, v)),
            recommended_version: config::recommended_dsh_version(app_handle),
            error: None,
        });
    }

    // （issue #790）。
    let mut seen_versions: HashSet<String> =
        version_tags.iter().map(|(v, _, _)| v.clone()).collect();
    let known_tags: HashSet<String> = version_tags.iter().map(|(_, tag, _)| tag.clone()).collect();
    let mut seen_tags = known_tags.clone();
    if !bundled_active {
        if let Some(v) = &installed_release {
            seen_versions.insert(v.clone());
        }
    }
    if let Ok(entries) = std::fs::read_dir(dependencies_dir(app_handle)) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let tag = if name.starts_with("dsh-") || name.starts_with("src-") {
                Some(name.clone())
            } else {
                None
            };
            let Some(tag) = tag else { continue };
            if !entry.path().is_dir() {
                continue;
            }
            let version = read_manifest_dsh_version(&entry.path())
                .or_else(|| download::parse_version_from_tag(&tag))
                .unwrap_or_default();
            if version.is_empty() || !seen_tags.insert(tag.clone()) {
                continue;
            }
            let orphaned = remote_catalog_available && !known_tags.contains(&tag);
            if !orphaned && !seen_versions.insert(version.clone()) {
                continue;
            }
            let dir = entry.path();
            rows.push(HarnessCore {
                id: format!("app-{tag}"),
                source: CoreSource::App,
                version: version.clone(),
                tag: tag.clone(),
                path: dir.to_string_lossy().into_owned(),
                dir: dir.to_string_lossy().into_owned(),
                present: true,
                active: false,
                removable: true,
                preview: download::is_preview_tag(&tag),
                above_recommended: config::is_dsh_version_above_recommended(app_handle, &version),
                orphaned,
                bundled: false,
                recommended_version: config::recommended_dsh_version(app_handle),
                error: None,
            });
        }
    }

    rows
}

/// 身份，不能凭它判定某个 release 已安装，否则会吞掉核心更新提示（issue #790）。
pub async fn has_installed_version(app_handle: &AppHandle, version: &str) -> bool {
    any_release_installed(&list(app_handle).await, version)
}

fn any_release_installed(cores: &[HarnessCore], version: &str) -> bool {
    cores
        .iter()
        .any(|c| release_installed_on_disk(c) && c.version == version)
}

/// 的副本"，证明不了某个 release 已下载（issue #790）。
fn release_installed_on_disk(core: &HarnessCore) -> bool {
    core.present && core.source == CoreSource::App && (core.bundled || !core.tag.is_empty())
}

/// `app` 兜底行如实呈现（issue #790）。
fn trusted_release_version(
    active_tag: Option<&str>,
    manifest_version: Option<&str>,
) -> Option<String> {
    let version = active_tag.and_then(download::parse_version_from_tag)?;
    (Some(version.as_str()) == manifest_version).then_some(version)
}

async fn stop_harness_for_core_switch(app_handle: &AppHandle) -> Result<(), String> {
    workflow::instances::require_exclusive_profile()?;
    if workflow::has_owned_process() {
        workflow::stop(app_handle.clone()).await?;
    }
    let handle = app_handle.clone();
    let profile = crate::service::profile::active_profile(app_handle);
    tauri::async_runtime::spawn_blocking(move || {
        workflow::instances::scope_sync(profile, || {
            workflow::terminate_stale_harness_processes(&handle)
        })
    })
    .await
    .map_err(|e| format!("CORE_SWITCH_STOP_FAILED: {e}"))??;
    Ok(())
}

pub async fn set_active(app_handle: &AppHandle, id: &str) -> Result<HarnessCore, String> {
    let transition_guard = if id == "app" || id == BUNDLED_CORE_ID || id == "local" {
        Some(workflow::acquire_core_transition().await?)
    } else {
        None
    };
    if id == "local" {
        let Some(core) = local_core(app_handle) else {
            return Err("CORE_LOCAL_NOT_FOUND: no local core detected".to_string());
        };
        // 低于最低支持版本的本地核心无法加载随包插件（issue #596）：显式切换同样
        let baseline = config::manifest::minimum_dsh_version(app_handle);
        if !core_supports_bundled_plugins(&core.version, baseline.as_deref()) {
            return Err(format!(
                "CORE_LOCAL_UNSUPPORTED: local dsh {} is below the minimum supported core; update it (`npm install -g @deepseek-ai/dsh@latest`) or keep a bundled version",
                core.version,
            ));
        }
        stop_harness_for_core_switch(app_handle).await?;
        config::update_store_dat_setting(app_handle, |setting| {
            setting.active_core = Some(CoreSource::Local.as_str().to_string());
        });
        config::dependencies::record(app_handle, config::dependencies::DEP_DSH, None);
    } else if id == BUNDLED_CORE_ID {
        let Some(bundled) = config::dependencies::bundled_core_dir(app_handle) else {
            return Err("CORE_BUNDLED_NOT_FOUND: this install ships no bundled core".to_string());
        };
        let entry = config::dependencies::entry_relative(app_handle, config::dependencies::DEP_DSH);
        if !bundled.join(entry).is_file() {
            return Err("CORE_BUNDLED_NOT_FOUND: bundled core files are missing".to_string());
        }
        stop_harness_for_core_switch(app_handle).await?;
        config::update_store_dat_setting(app_handle, |setting| {
            setting.active_core = Some(CoreSource::App.as_str().to_string());
            setting.dsh_pkg_tag = None;
            setting.dsh_pkg_commit = None;
        });
        config::dependencies::record(app_handle, config::dependencies::DEP_DSH, Some(bundled));
    } else if id == "app" {
        if !config::get_dsh_binary_path(app_handle).exists() {
            return Err("CORE_APP_NOT_FOUND: bundled core is not installed".to_string());
        }
        stop_harness_for_core_switch(app_handle).await?;
        config::update_store_dat_setting(app_handle, |setting| {
            setting.active_core = Some(CoreSource::App.as_str().to_string());
        });
        record_managed_core(app_handle);
    } else if let Some(tag) = id.strip_prefix("app-") {
        switch_app_version(app_handle, tag).await?;
    } else {
        return Err(format!("CORE_INVALID_ID: {id}"));
    }
    drop(transition_guard);

    list_local(app_handle)
        .into_iter()
        .find(|c| c.active)
        .ok_or_else(|| "CORE_NOT_FOUND: active core disappeared after switch".to_string())
}

async fn switch_app_version(app_handle: &AppHandle, tag: &str) -> Result<(), String> {
    let _transition_guard = workflow::acquire_core_transition().await?;
    workflow::instances::require_exclusive_profile()?;
    let deps = dependencies_dir(app_handle);
    let active_dir = config::dependencies::managed_root(app_handle, config::dependencies::DEP_DSH);
    fs_guard::validate_id(tag)?;
    let cur_tag = config::get_dsh_pkg_tag(app_handle);

    if cur_tag.as_deref() == Some(tag) && config::get_dsh_binary_path(app_handle).is_file() {
        stop_harness_for_core_switch(app_handle).await?;
        config::update_store_dat_setting(app_handle, |setting| {
            setting.active_core = Some(CoreSource::App.as_str().to_string());
        });
        return Ok(());
    }
    let target_dir = existing_slot_dir(app_handle, tag)
        .ok_or_else(|| format!("CORE_VERSION_NOT_DOWNLOADED: {tag}"))?;

    if config::dependencies::bundled_core_dir(app_handle).is_some() {
        stop_harness_for_core_switch(app_handle).await?;
        let commit = match download::fetch_dsh_pkg_tags(app_handle).await {
            Ok(tags) => tags.into_iter().find(|(t, _)| t == tag).map(|(_, c)| c),
            Err(e) => {
                log::warn!("failed to resolve commit for tag {tag}: {e}");
                None
            }
        };
        config::update_store_dat_setting(app_handle, |setting| {
            setting.active_core = Some(CoreSource::App.as_str().to_string());
            setting.dsh_pkg_tag = Some(tag.to_string());
            if let Some(commit) = commit {
                setting.dsh_pkg_commit = Some(commit);
            }
        });
        config::dependencies::record(app_handle, config::dependencies::DEP_DSH, Some(target_dir));
        return Ok(());
    }

    if workflow::has_owned_process() {
        workflow::stop(app_handle.clone()).await.map_err(|e| {
            format!(
                "CORE_SWITCH_STOP_FAILED: failed to stop harness before core switch at {}: {e}",
                active_dir.display()
            )
        })?;
    }
    {
        let handle = app_handle.clone();
        let profile = crate::service::profile::active_profile(app_handle);
        tauri::async_runtime::spawn_blocking(move || {
            workflow::instances::scope_sync(profile, || {
                workflow::terminate_stale_harness_processes(&handle)
            })
        })
        .await
        .map_err(|e| format!("CORE_SWITCH_STOP_FAILED: {e}"))??;
    }

    let backup_tag = cur_tag.clone().unwrap_or_else(|| {
        format!(
            "dsh-{}",
            config::get_dsh_version(app_handle).unwrap_or_else(|| "unknown".to_string())
        )
    });
    let backup_dir = safe_slot_path(&deps, &backup_tag)?;

    let holding = deps.join(format!(".backup-holding-{}", std::process::id()));
    let stale_backup = deps.join(format!(".backup-stale-{}", std::process::id()));

    if backup_dir.exists() {
        download::rename_with_retry(&backup_dir, &holding)
            .await
            .map_err(|e| {
                format!(
                    "CORE_SWITCH_FAILED: {} -> {}: {e}",
                    backup_dir.display(),
                    holding.display()
                )
            })?;
    }

    if active_dir.exists() {
        if let Err(e) = download::rename_with_retry(&active_dir, &backup_dir).await {
            if holding.exists() {
                let _ = download::rename_with_retry(&holding, &backup_dir).await;
            }
            return Err(format!(
                "CORE_SWITCH_FAILED: {} -> {}: {e}",
                active_dir.display(),
                backup_dir.display()
            ));
        }
    }

    if let Err(e) = download::rename_with_retry(&target_dir, &active_dir).await {
        let rollback1 = download::rename_with_retry(&backup_dir, &active_dir).await;
        let rollback2 = if holding.exists() {
            download::rename_with_retry(&holding, &backup_dir).await
        } else {
            Ok::<(), std::io::Error>(())
        };
        return Err(format!(
            "CORE_SWITCH_FAILED: {} -> {}: {e} (rollback active: {}, rollback backup: {})",
            target_dir.display(),
            active_dir.display(),
            rollback1.is_ok(),
            rollback2.is_ok()
        ));
    }

    if holding.exists() && !download::remove_dir_with_retry(&holding).await {
        log::warn!(
            "CORE_SWITCH_WARNING: failed to remove stale backup {} after successful switch",
            holding.display()
        );
        if let Err(e) = download::rename_with_retry(&holding, &stale_backup).await {
            log::warn!(
                "CORE_SWITCH_WARNING: failed to rename stale backup to {}: {e}",
                stale_backup.display()
            );
        }
    }

    let commit = match download::fetch_dsh_pkg_tags(app_handle).await {
        Ok(tags) => tags.into_iter().find(|(t, _)| t == tag).map(|(_, c)| c),
        Err(e) => {
            log::warn!("failed to resolve commit for tag {tag}: {e}");
            None
        }
    };
    config::update_store_dat_setting(app_handle, |setting| {
        setting.active_core = Some(CoreSource::App.as_str().to_string());
        setting.dsh_pkg_tag = Some(tag.to_string());
        if let Some(commit) = commit {
            setting.dsh_pkg_commit = Some(commit);
        }
    });
    record_managed_core(app_handle);
    Ok(())
}

fn record_managed_core(app_handle: &AppHandle) {
    config::dependencies::record(
        app_handle,
        config::dependencies::DEP_DSH,
        Some(config::dependencies::managed_root(
            app_handle,
            config::dependencies::DEP_DSH,
        )),
    );
}

pub async fn download_version(app_handle: &AppHandle, tag: &str) -> Result<HarnessCore, String> {
    fs_guard::validate_id(tag)?;
    let dest = slot_dir(app_handle, tag);
    if dest.exists() {
        return Ok(row_for_tag(app_handle, tag, &dest));
    }

    let info = download::fetch_dsh_pkg_asset(app_handle, tag)
        .await
        .map_err(|e| format!("CORE_METADATA_FAILED: {e}"))?;
    let digest = info.digest.ok_or_else(|| {
        format!("CORE_INTEGRITY_UNAVAILABLE: trusted SHA-256 unavailable for {tag}, cannot download safely")
    })?;

    let window = crate::utils::operation_window(app_handle)
        .ok_or("WINDOW_NOT_FOUND: current profile window missing")?;
    let mut tracker = download::ProgressTracker::new(&window, 2);
    tracker.start_phase("download", &format!("正在下载核心版本 {tag}"));
    let mut urls = vec![info.asset_url.clone()];
    urls.extend(config::mirror_download_urls(&info.asset_url));
    let buffer = download::download_file_from_sources(&tracker, urls)
        .await
        .map_err(|e| format!("CORE_DOWNLOAD_FAILED: {e}"))?;
    download::verify_sha256(&buffer, &digest).map_err(|e| format!("CORE_INTEGRITY_FAILED: {e}"))?;
    tracker.end_phase();
    let name = info
        .asset_url
        .rsplit('/')
        .next()
        .unwrap_or(&info.asset_url)
        .to_string();
    tracker.start_phase("extract", &format!("正在解压核心版本 {tag}"));
    download::ensure_extract(&tracker, name, buffer, dest.clone())
        .await
        .map_err(|e| format!("CORE_EXTRACT_FAILED: {e}"))?;
    tracker.end_phase();
    log::info!("Downloaded dsh core {tag} to {}", dest.display());

    Ok(row_for_tag(app_handle, tag, &dest))
}

pub async fn remove_version(app_handle: &AppHandle, id: &str) -> Result<(), String> {
    if id == BUNDLED_CORE_ID {
        return Err(
            "CORE_BUNDLED_PROTECTED: the bundled core ships with the app and cannot be removed"
                .to_string(),
        );
    }
    let Some(tag) = id.strip_prefix("app-") else {
        return Err(format!("CORE_INVALID_ID: {id}"));
    };
    fs_guard::validate_id(tag)?;
    let cur_tag = config::get_dsh_pkg_tag(app_handle);
    if cur_tag.as_deref() == Some(tag) && active_source(app_handle) == CoreSource::App {
        return Err(format!(
            "CORE_ACTIVE_VERSION: cannot remove in-use version {tag}"
        ));
    }
    let dir = existing_slot_dir(app_handle, tag)
        .ok_or_else(|| format!("CORE_VERSION_NOT_FOUND: {tag}"))?;

    match std::fs::remove_dir_all(&dir) {
        Ok(()) => {
            log::info!("Removed dsh core slot {tag} at {}", dir.display());
            return Ok(());
        }
        Err(e) => {
            log::warn!("dsh core slot {tag} is locked ({e}); stopping the service before retry");
        }
    }

    if workflow::has_owned_process() {
        if let Err(e) = workflow::stop(app_handle.clone()).await {
            log::warn!("failed to stop harness before core removal: {e}");
        }
    }
    if !download::remove_dir_with_retry(&dir).await {
        return Err(format!(
            "CORE_REMOVE_FAILED: cannot remove {}",
            dir.display()
        ));
    }
    log::info!("Removed dsh core slot {tag} after stopping the service");
    Ok(())
}

fn row_for_tag(app_handle: &AppHandle, tag: &str, dir: &Path) -> HarnessCore {
    let active = config::get_dsh_pkg_tag(app_handle).as_deref() == Some(tag)
        && active_source(app_handle) == CoreSource::App;
    let dir_str = dir.to_string_lossy().into_owned();
    HarnessCore {
        id: format!("app-{tag}"),
        source: CoreSource::App,
        version: download::parse_version_from_tag(tag).unwrap_or_default(),
        tag: tag.to_string(),
        path: dir_str.clone(),
        dir: dir_str,
        present: true,
        active,
        removable: true,
        preview: download::is_preview_tag(tag),
        above_recommended: download::parse_version_from_tag(tag)
            .is_some_and(|version| config::is_dsh_version_above_recommended(app_handle, &version)),
        orphaned: false,
        bundled: false,
        recommended_version: config::recommended_dsh_version(app_handle),
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_paths_reject_traversal_and_accept_release_tag() {
        let root = std::env::temp_dir().join(format!("dsh-core-slots-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();

        assert!(safe_slot_path(&root, "foo/../../target").is_err());
        let valid = "dsh-0.1.0-rc.8-32331963388";
        let expected = root.join(valid);
        assert_eq!(safe_slot_path(&root, valid).unwrap(), expected);

        std::fs::create_dir_all(&expected).unwrap();
        assert_eq!(safe_slot_path(&root, valid).unwrap(), expected);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn slot_paths_reject_symlink_escape() {
        let root = std::env::temp_dir().join(format!("dsh-core-slots-link-{}", std::process::id()));
        let outside =
            std::env::temp_dir().join(format!("dsh-core-slots-outside-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("dsh-evil")).unwrap();

        assert!(safe_slot_path(&root, "dsh-evil").is_err());
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[test]
    fn list_dedupes_versions_keeping_last_tag() {
        let metas = vec![
            ("dsh-0.2.0-preview.1-32490000001".to_string(), true),
            ("dsh-0.1.1-rc.1-32342588166".to_string(), false),
            ("dsh-0.1.0-rc.8-32331963388".to_string(), false),
            ("dsh-0.1.0-rc.8-32342588166".to_string(), true),
            ("dsh-0.1.0-rc.8-32342588167".to_string(), false),
            ("dsh-0.1.0-rc.7-31773193667".to_string(), false),
            ("dsh-0.1.0-rc.7-31773193668".to_string(), false),
            ("dsh-0.1.0-beta.1-32490000002".to_string(), false),
        ];
        let mut version_tags: Vec<(String, String, bool)> = Vec::new();
        for meta in &metas {
            let Some(version) = download::parse_version_from_tag(&meta.0) else {
                continue;
            };
            let preview = meta.1 || download::is_preview_tag(&meta.0);
            if let Some(entry) = version_tags.iter_mut().find(|(v, _, _)| v == &version) {
                entry.1 = meta.0.clone();
                entry.2 = preview;
            } else {
                version_tags.push((version, meta.0.clone(), preview));
            }
        }
        let versions: Vec<&str> = version_tags.iter().map(|(v, _, _)| v.as_str()).collect();
        let kept_tags: Vec<&str> = version_tags.iter().map(|(_, t, _)| t.as_str()).collect();
        let previews: Vec<bool> = version_tags.iter().map(|(_, _, p)| *p).collect();
        assert_eq!(
            versions,
            vec![
                "0.2.0-preview.1",
                "0.1.1-rc.1",
                "0.1.0-rc.8",
                "0.1.0-rc.7",
                "0.1.0-beta.1"
            ]
        );
        assert_eq!(kept_tags[2], "dsh-0.1.0-rc.8-32342588167");
        assert_eq!(kept_tags[3], "dsh-0.1.0-rc.7-31773193668");
        assert_eq!(previews, vec![true, false, false, false, true]);
    }

    #[test]
    fn trusted_release_version_requires_record_manifest_agreement() {
        assert_eq!(
            trusted_release_version(Some("dsh-0.2.0-rc.2-36556493178"), Some("0.2.0-rc.2")),
            Some("0.2.0-rc.2".to_string())
        );
        // 否则会把激活目录认领成记录里那个 tag 的已装行（issue #790）
        assert_eq!(
            trusted_release_version(Some("dsh-0.1.7-rc.2-36024748146"), Some("0.2.0-rc.2")),
            None
        );
        assert_eq!(trusted_release_version(None, Some("0.2.0-rc.2")), None);
        assert_eq!(
            trusted_release_version(Some("dsh-latest"), Some("0.2.0-rc.2")),
            None
        );
        assert_eq!(
            trusted_release_version(Some("dsh-0.2.0-rc.2-36556493178"), None),
            None
        );
    }

    fn core_row(
        id: &str,
        source: CoreSource,
        tag: &str,
        present: bool,
        bundled: bool,
    ) -> HarnessCore {
        HarnessCore {
            id: id.to_string(),
            source,
            version: "0.2.0-rc.2".to_string(),
            tag: tag.to_string(),
            path: String::new(),
            dir: String::new(),
            present,
            active: false,
            removable: false,
            preview: false,
            above_recommended: false,
            orphaned: false,
            bundled,
            recommended_version: None,
            error: None,
        }
    }

    #[test]
    fn release_installed_on_disk_needs_release_identity() {
        assert!(release_installed_on_disk(&core_row(
            "app-dsh-0.2.0-rc.2-36556493178",
            CoreSource::App,
            "dsh-0.2.0-rc.2-36556493178",
            true,
            false
        )));
        assert!(release_installed_on_disk(&core_row(
            "app-bundled",
            CoreSource::App,
            "",
            true,
            true
        )));
        // 某个 release 已下载——凭它判定会吞掉核心更新提示（issue #790）
        assert!(!release_installed_on_disk(&core_row(
            "app",
            CoreSource::App,
            "",
            true,
            false
        )));
        assert!(!release_installed_on_disk(&core_row(
            "local",
            CoreSource::Local,
            "dsh-0.2.0-rc.2-36556493178",
            true,
            false
        )));
        assert!(!release_installed_on_disk(&core_row(
            "app-dsh-0.2.0-rc.2-36556493178",
            CoreSource::App,
            "dsh-0.2.0-rc.2-36556493178",
            false,
            false
        )));
    }

    #[test]
    fn ghost_row_does_not_report_the_version_as_installed() {
        let tag = "dsh-0.2.0-rc.2-36556493178";
        assert!(any_release_installed(
            &[core_row(
                &format!("app-{tag}"),
                CoreSource::App,
                tag,
                true,
                false
            )],
            "0.2.0-rc.2"
        ));
        // 「已下载」，于是 check_dsh_update 吞掉更新提示（issue #790）
        assert!(!any_release_installed(
            &[core_row("app", CoreSource::App, "", true, false)],
            "0.2.0-rc.2"
        ));
        assert!(!any_release_installed(
            &[core_row("local", CoreSource::Local, tag, true, false)],
            "0.2.0-rc.2"
        ));
    }
}
