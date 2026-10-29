
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::preset::{load_deprecated_ids, load_presets, PreinstallPluginInfo};
use crate::{AppHandle};

#[derive(Deserialize)]
pub(crate) struct ProfilePackageJson {
    #[serde(default)]
    pub(crate) dependencies: HashMap<String, String>,
    #[serde(default)]
    pub(crate) dsh: Option<ProfileDshSection>,
}

#[derive(Deserialize)]
pub(crate) struct ProfileDshSection {
    #[serde(default)]
    pub(crate) profile: Option<ProfileInner>,
}

#[derive(Deserialize)]
pub(crate) struct ProfileInner {
    #[serde(default)]
    pub(crate) bundles: Vec<String>,
}

pub(crate) fn profile_dir(app_handle: &AppHandle) -> PathBuf {
    crate::service::profile::profile_dir_of(
        app_handle,
        &crate::service::profile::active_profile(app_handle),
    )
}

pub(crate) fn list_installed(app_handle: &AppHandle) -> HashSet<String> {
    declared_packages(app_handle).unwrap_or_default()
}

pub(crate) fn declared_packages(app_handle: &AppHandle) -> Option<HashSet<String>> {
    let manifest_path = profile_dir(app_handle).join("package.json");
    let content = std::fs::read_to_string(&manifest_path).ok()?;
    let manifest = serde_json::from_str::<ProfilePackageJson>(&content).ok()?;

    let mut set: HashSet<String> = manifest.dependencies.into_keys().collect();
    if let Some(dsh) = manifest.dsh {
        if let Some(profile) = dsh.profile {
            set.extend(profile.bundles);
        }
    }
    Some(set)
}

pub(crate) fn is_installed(app_handle: &AppHandle, id: &str) -> bool {
    list_installed(app_handle).contains(id)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreinstallPlugin {
    pub id: String,
    pub name: String,
    pub description: String,
    pub repo_url: String,
    pub recommended: bool,
    pub fix: bool,
    pub default_checked: bool,
    pub default_unchecked: bool,
    pub installed: bool,
    pub unsupported: bool,
}

pub(crate) fn installed_name(p: &PreinstallPluginInfo) -> &str {
    p.package.as_deref().unwrap_or(p.id.as_str())
}

pub fn list(app_handle: &AppHandle) -> Vec<PreinstallPlugin> {
    let installed = list_installed(app_handle);
    let is_windows = cfg!(windows);
    let deprecated_ids = load_deprecated_ids(app_handle);
    let core_version = crate::service::core::active_version(app_handle);

    load_presets(app_handle)
        .into_iter()
        .filter(|p| !p.win_only || is_windows)
        .filter(|p| !p.internal)
        .filter(|p| !deprecated_ids.contains(&p.id))
        .map(|p| {
            let is_installed = installed.contains(installed_name(&p));
            let unsupported = p.unsupported_on(core_version.as_deref());
            PreinstallPlugin {
                id: p.id,
                name: p.name,
                description: p.description,
                repo_url: p.repo_url,
                recommended: p.recommended,
                fix: p.fix,
                default_checked: p.default_checked,
                default_unchecked: p.default_unchecked,
                installed: is_installed,
                unsupported,
            }
        })
        .collect()
}

const NPMRC_KEY: &str = "confirmModulesPurge=false";

pub(crate) fn ensure_npmrc_at(npmrc_path: &PathBuf) -> Result<(), String> {
    let existing = match std::fs::read_to_string(npmrc_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("NPMRC_READ_FAILED: {e}")),
    };

    if existing.lines().any(|l| l.trim() == NPMRC_KEY) {
        return Ok(());
    }

    let mut content = existing;
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(NPMRC_KEY);
    content.push('\n');

    if let Some(dir) = npmrc_path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("NPMRC_DIR_CREATE_FAILED: {e}"))?;
    }
    std::fs::write(npmrc_path, content).map_err(|e| format!("NPMRC_WRITE_FAILED: {e}"))?;
    log::info!("Ensured profile .npmrc: {}", npmrc_path.display());
    Ok(())
}

pub(crate) fn ensure_profile_npmrc(app_handle: &AppHandle) -> Result<(), String> {
    ensure_npmrc_at(&profile_dir(app_handle).join(".npmrc"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_npmrc(label: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("dsh-npmrc-test-{}-{label}", std::process::id()))
            .join(".npmrc")
    }

    #[test]
    fn npmrc_created_when_missing() {
        let path = temp_npmrc("created");
        let _ = std::fs::remove_file(&path);
        ensure_npmrc_at(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.lines().any(|l| l.trim() == NPMRC_KEY));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn npmrc_preserves_existing_content_and_is_idempotent() {
        let path = temp_npmrc("preserve");
        let _ = std::fs::remove_file(&path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "registry=https://registry.npmjs.org/\n").unwrap();

        ensure_npmrc_at(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("registry=https://registry.npmjs.org/"));
        assert_eq!(content.matches(NPMRC_KEY).count(), 1);

        ensure_npmrc_at(&path).unwrap();
        let content2 = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("registry=https://registry.npmjs.org/"));
        assert_eq!(content2.matches(NPMRC_KEY).count(), 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn list_installed_parses_manifest() {
        let dir = std::env::temp_dir().join(format!("dsh-plugin-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest_json = serde_json::json!({
            "name": "dsh-profile-web",
            "private": true,
            "dependencies": {
                "dshmarket": "1.0.0",
                "@deepseek-ai/dsh-base": "1.0.0"
            },
            "dsh": {
                "profile": {
                    "bundles": ["@deepseek-ai/dsh-base", "dshmarket"]
                }
            }
        });
        std::fs::write(
            dir.join("package.json"),
            serde_json::to_string(&manifest_json).unwrap(),
        )
        .unwrap();

        let content = std::fs::read_to_string(dir.join("package.json")).unwrap();
        let parsed: ProfilePackageJson = serde_json::from_str(&content).unwrap();

        let mut set: HashSet<String> = parsed.dependencies.into_keys().collect();
        if let Some(dsh) = parsed.dsh {
            if let Some(profile) = dsh.profile {
                set.extend(profile.bundles);
            }
        }

        assert!(set.contains("dshmarket"));
        assert!(set.contains("@deepseek-ai/dsh-base"));
        assert_eq!(set.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn installed_name_resolves_package_else_id() {
        let installed = PreinstallPluginInfo {
            id: "dsh-session-context-menu".into(),
            spec: "github:baihejiangnan/dsh-session-context-menu".into(),
            package: Some("@baihejiangnan/dsh-session-context-menu".into()),
            name: "DSH Session Context Menu".into(),
            description: String::new(),
            repo_url: String::new(),
            recommended: false,
            fix: false,
            default_checked: true,
            default_unchecked: false,
            version: None,
            win_only: false,
            internal: false,
        };
        assert_eq!(
            installed_name(&installed),
            "@baihejiangnan/dsh-session-context-menu"
        );

        let plain = PreinstallPluginInfo {
            package: None,
            ..installed
        };
        assert_eq!(installed_name(&plain), "dsh-session-context-menu");
    }
}
