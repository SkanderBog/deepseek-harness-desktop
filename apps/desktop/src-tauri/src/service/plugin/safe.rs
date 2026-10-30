use std::collections::HashSet;

use super::recovery::is_actionable_plugin_ref;
use super::{installed_name, list_installed, load_presets, uninstall_recovery};
use crate::AppHandle;
use crate::service::profile::{SAFE_PROFILE, active_profile};

fn user_plugin_names(installed: &HashSet<String>, builtin: &HashSet<String>) -> Vec<String> {
    let mut names: Vec<String> = installed
        .iter()
        .filter(|name| is_actionable_plugin_ref(name) && !builtin.contains(*name))
        .cloned()
        .collect();
    names.sort();
    names
}

fn builtin_plugin_names(app_handle: &AppHandle) -> HashSet<String> {
    load_presets(app_handle)
        .iter()
        .filter(|plugin| plugin.internal)
        .map(|plugin| installed_name(plugin).to_string())
        .collect()
}

pub(crate) fn purge_user_plugins_in_safe_profile(app_handle: &AppHandle) -> Result<(), String> {
    if active_profile(app_handle) != SAFE_PROFILE {
        return Ok(());
    }

    let builtin = builtin_plugin_names(app_handle);
    if builtin.is_empty() {
        log::warn!(
            "SAFE_MODE_PLUGIN_PURGE_SKIPPED: built-in plugin manifest unavailable, cannot tell user plugins apart"
        );
        return Ok(());
    }

    let names = user_plugin_names(&list_installed(app_handle), &builtin);
    if names.is_empty() {
        return Ok(());
    }

    log::info!(
        "safe mode: removing {} user plugin(s) from the safe profile: {names:?}",
        names.len()
    );
    let mut failures = Vec::new();
    for name in &names {
        match uninstall_recovery(app_handle, name) {
            Ok(()) => log::info!("SAFE_MODE_PLUGIN_PURGE: removed {name}"),
            Err(e) => {
                log::warn!("SAFE_MODE_PLUGIN_PURGE_FAILED: {name}: {e}");
                failures.push(format!("{name}: {e}"));
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "SAFE_MODE_PLUGIN_PURGE_FAILED: {}",
            failures.join("; ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::manifest;
    use std::path::PathBuf;

    fn manifest_package_names(
        section: fn(&manifest::Plugins) -> &[manifest::PluginEntry],
    ) -> Vec<String> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/manifest.jsonc");
        let manifest = manifest::read_at(&path).expect("resource manifest should be valid");
        section(&manifest.plugins)
            .iter()
            .map(|entry| entry.package.clone().unwrap_or_else(|| entry.id.clone()))
            .collect()
    }

    fn names(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn purges_user_plugins_but_keeps_core_and_builtin() {
        let installed = names(&[
            "@deepseek-ai/dsh-base",
            "@deepseek-ai/dsh-web-app",
            "dsh-tauri",
            "dsh-tauri-ui",
            "dshmarket",
            "dsh-notification",
        ]);
        let builtin = names(&["dsh-tauri", "dsh-tauri-ui"]);

        assert_eq!(
            user_plugin_names(&installed, &builtin),
            vec!["dsh-notification".to_string(), "dshmarket".to_string()]
        );
    }

    #[test]
    fn pristine_safe_profile_needs_no_purge() {
        let installed = names(&["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"]);
        assert!(user_plugin_names(&installed, &HashSet::new()).is_empty());
    }

    #[test]
    fn invalid_refs_are_never_targeted() {
        let installed = names(&["", "..", "foo/../../target", "with space", "dsh-ok"]);
        assert_eq!(
            user_plugin_names(&installed, &HashSet::new()),
            vec!["dsh-ok".to_string()]
        );
    }

    #[test]
    fn shipped_internal_manifest_is_protected() {
        let builtin = manifest_package_names(|plugins| &plugins.built_in);
        let presets = manifest_package_names(|plugins| &plugins.preset);
        assert!(builtin.iter().any(|name| name == "dsh-tauri"));
        assert!(presets.iter().any(|name| name == "dshmarket"));

        let mut installed: HashSet<String> = builtin.iter().cloned().collect();
        installed.extend(presets.iter().cloned());
        let purged = user_plugin_names(&installed, &builtin.iter().cloned().collect());

        assert!(!purged.iter().any(|name| builtin.contains(name)));
        assert!(purged.contains(&"dshmarket".to_string()));
    }
}
