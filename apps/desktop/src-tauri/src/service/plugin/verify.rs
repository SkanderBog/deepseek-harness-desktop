//! 动机（issue #90）：首次引导「安装全部推荐插件」后，若安装只把插件写入了

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::config;
use crate::service::cli;

use super::errors;
use super::install::{build_plugin_envs, new_process_owner, run_plugin_process};
use super::installed::{installed_name, profile_dir, ProfilePackageJson};
use super::preset::{load_presets, PreinstallPluginInfo};
use super::process::is_process_cleanup_failure;
use crate::{AppHandle};

pub(super) fn missing_plugin_ids(
    presets: &[PreinstallPluginInfo],
    dependencies: &HashMap<String, String>,
    bundles: &[String],
    node_modules: &Path,
) -> Vec<String> {
    presets
        .iter()
        .filter(|preset| {
            let name = installed_name(preset);
            let referenced =
                dependencies.contains_key(name) || bundles.iter().any(|bundle| bundle == name);
            if !referenced {
                return false;
            }
            !node_modules.join(name).join("package.json").is_file()
        })
        .map(|preset| preset.id.clone())
        .collect()
}

fn record_missing(app_handle: &AppHandle, ids: &[String], detail: &str) {
    for id in ids {
        if let Err(e) = errors::record(app_handle, id, "install", detail) {
            log::warn!("failed to record plugin integrity error for {id}: {e}");
        }
    }
}

pub(crate) async fn ensure_preset_plugins(app_handle: &AppHandle) -> Result<(), String> {
    let presets = load_presets(app_handle);
    if presets.is_empty() {
        return Ok(());
    }

    let profile = profile_dir(app_handle);
    let (dependencies, bundles) = match std::fs::read_to_string(profile.join("package.json")) {
        Ok(raw) => match serde_json::from_str::<ProfilePackageJson>(&raw) {
            Ok(manifest) => (
                manifest.dependencies,
                manifest
                    .dsh
                    .and_then(|d| d.profile)
                    .map(|p| p.bundles)
                    .unwrap_or_default(),
            ),
            Err(_) => return Ok(()),
        },
        Err(_) => return Ok(()),
    };

    let node_modules = profile.join("node_modules");
    let missing = missing_plugin_ids(&presets, &dependencies, &bundles, &node_modules);
    if missing.is_empty() {
        return Ok(());
    }
    log::warn!(
        "PRESET_PLUGIN_INTEGRITY: {} preset plugin(s) referenced by profile but missing from node_modules: {missing:?}",
        missing.len()
    );

    if let Err(e) = repair_with_pnpm_install(app_handle, &profile).await {
        if is_process_cleanup_failure(&e) {
            return Err(e);
        }
        log::warn!("PRESET_PLUGIN_REPAIR_FAILED: {e}");
        record_missing(app_handle, &missing, &e);
        return Ok(());
    }

    let still_missing = missing_plugin_ids(&presets, &dependencies, &bundles, &node_modules);
    if still_missing.is_empty() {
        log::info!("preset plugin integrity repaired via pnpm install: {missing:?}");
        return Ok(());
    }
    let detail = format!(
        "PRESET_PLUGIN_STILL_MISSING: 修复后仍缺失以下插件产物: {still_missing:?}。可尝试卸载后重新安装。"
    );
    log::warn!("{detail}");
    record_missing(app_handle, &still_missing, &detail);
    Ok(())
}

async fn repair_with_pnpm_install(app_handle: &AppHandle, profile: &Path) -> Result<(), String> {
    let (program, mut args) = pnpm_direct(app_handle).await?.ok_or_else(|| {
        "PNPM_NOT_FOUND: 无捆绑 pnpm 且无用户 pnpm，无法修复插件依赖（如需可手动执行 pnpm install）"
            .to_string()
    })?;
    let bundled = config::get_pnpm_binary_path(app_handle);
    let prefer_bundled = args
        .iter()
        .any(|arg| arg.as_os_str() == bundled.as_os_str());
    args.push(OsString::from("install"));

    let window = crate::utils::operation_window(app_handle)
        .ok_or("WINDOW_NOT_FOUND: current profile window missing")?;
    let envs = build_plugin_envs(app_handle, prefer_bundled);
    log::info!(
        "repairing profile node_modules via pnpm install in {}",
        profile.display()
    );
    let (exit_code, output) = run_plugin_process(
        &program,
        &args,
        profile,
        &envs,
        &window,
        new_process_owner(),
    )
    .await?;
    if exit_code != 0 {
        let tail: String = output
            .chars()
            .rev()
            .take(REPAIR_OUTPUT_LIMIT)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        return Err(format!(
            "PNPM_INSTALL_FAILED: exit code {exit_code}: {tail}"
        ));
    }
    Ok(())
}

const REPAIR_OUTPUT_LIMIT: usize = 4000;

pub(crate) async fn pnpm_direct(
    app_handle: &AppHandle,
) -> Result<Option<(PathBuf, Vec<OsString>)>, String> {
    use super::install::{bundled_pnpm_major, pnpm_major_version_at_bounded, profile_store_major};

    let bundled = config::get_pnpm_binary_path(app_handle);
    let bundled_ready = bundled.exists();
    let store = profile_store_major(app_handle);
    let bundled_matches =
        bundled_ready && store.is_none_or(|s| bundled_pnpm_major(app_handle) == Some(s));

    if bundled_matches {
        let node = config::get_node_binary_path(app_handle);
        return Ok(Some((node, vec![bundled.into_os_string()])));
    }

    #[cfg(windows)]
    let user_pnpm = cli::find_user_pnpm_executable(app_handle);
    #[cfg(not(windows))]
    let user_pnpm = cli::find_user_pnpm(app_handle);
    if let Some(user) = user_pnpm {
        let matches = match store {
            None => true,
            Some(store) => {
                pnpm_major_version_at_bounded(
                    &user,
                    Some(&config::get_node_binary_path(app_handle)),
                    None,
                    super::process::new_process_owner(),
                )
                .await?
                    == Some(store)
            }
        };
        if matches {
            return Ok(Some((user, Vec::new())));
        }
    }

    if bundled_ready {
        let node = config::get_node_binary_path(app_handle);
        return Ok(Some((node, vec![bundled.into_os_string()])));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn preset(id: &str, package: Option<&str>) -> PreinstallPluginInfo {
        PreinstallPluginInfo {
            id: id.into(),
            spec: String::new(),
            package: package.map(String::from),
            name: String::new(),
            description: String::new(),
            repo_url: String::new(),
            recommended: false,
            fix: false,
            default_checked: false,
            default_unchecked: false,
            version: None,
            win_only: false,
            internal: false,
        }
    }

    fn setup(dir_label: &str, present: &[&str]) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("dsh-verify-{}-{}", std::process::id(), dir_label));
        let _ = fs::remove_dir_all(&root);
        let node_modules = root.join("node_modules");
        for name in present {
            let dir = node_modules.join(name);
            fs::create_dir_all(&dir).expect("create node_modules pkg dir");
            fs::write(dir.join("package.json"), "{}").expect("write package.json");
        }
        root
    }

    #[test]
    fn missing_detects_absent_but_referenced() {
        let presets = vec![preset("dshmarket", None), preset("dsh-notification", None)];
        let deps = HashMap::from([
            ("dshmarket".to_string(), "1.0.0".to_string()),
            (
                "dsh-notification".to_string(),
                "github:omdsh-dev/dsh-notification".to_string(),
            ),
        ]);
        let bundles = vec!["dshmarket".to_string()];
        let root = setup("absent", &["dshmarket"]);
        let missing = missing_plugin_ids(&presets, &deps, &bundles, &root.join("node_modules"));
        assert_eq!(missing, vec!["dsh-notification"]);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_ignores_unreferenced_and_present() {
        let presets = vec![
            preset("dshmarket", None),
            preset("dsh-better-sidebar", None),
        ];
        let deps = HashMap::from([("dshmarket".to_string(), "^1.0.0".to_string())]);
        let bundles: Vec<String> = Vec::new();
        let root = setup("ok", &["dshmarket"]);
        let missing = missing_plugin_ids(&presets, &deps, &bundles, &root.join("node_modules"));
        assert!(missing.is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_resolves_scoped_package_name() {
        let presets = vec![preset(
            "dsh-session-context-menu",
            Some("@baihejiangnan/dsh-session-context-menu"),
        )];
        let deps = HashMap::from([(
            "@baihejiangnan/dsh-session-context-menu".to_string(),
            "github:baihejiangnan/dsh-session-context-menu".to_string(),
        )]);
        let bundles = vec!["@baihejiangnan/dsh-session-context-menu".to_string()];
        let root = setup("scoped", &["@baihejiangnan/dsh-session-context-menu"]);
        let missing = missing_plugin_ids(&presets, &deps, &bundles, &root.join("node_modules"));
        assert!(missing.is_empty());
        let missing = missing_plugin_ids(
            &presets,
            &deps,
            &bundles,
            &root.join("missing-node_modules"),
        );
        assert_eq!(missing, vec!["dsh-session-context-menu"]);
        fs::remove_dir_all(&root).ok();
    }
}
