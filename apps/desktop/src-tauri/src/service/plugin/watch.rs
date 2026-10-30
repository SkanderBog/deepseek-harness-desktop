use crate::service::workflow::instances;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::errors::{self, PluginError};
use super::installed::{ProfilePackageJson, installed_name, profile_dir};
use super::preset::{PreinstallPluginInfo, load_presets};
use crate::AppHandle;

pub(crate) const PLUGINS_UPDATED_EVENT: &str = "dsh-plugins-updated";

const DEBOUNCE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DshPlugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub repo_url: String,
    pub bundled: bool,
    pub disabled: bool,
    pub recommended: bool,
    pub fix: bool,
    pub internal: bool,
    pub update_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    pub has_snapshot: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<PluginError>,
}

#[derive(Deserialize, Default)]
struct PluginPackageJson {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    repository: Option<RepositoryField>,
    #[serde(default)]
    dsh: Option<PluginDshField>,
}

#[derive(Deserialize, Default)]
struct PluginDshField {
    #[serde(default)]
    bundle: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RepositoryField {
    Url(String),
    Object { url: Option<String> },
}

fn plugin_dir(profile: &Path, id: &str) -> PathBuf {
    profile.join("node_modules").join(id)
}

pub(crate) fn normalize_repo_url(url: &str) -> String {
    let mut normalized = url.trim().to_string();
    if let Some(rest) = normalized.strip_prefix("git+") {
        normalized = rest.to_string();
    }
    if let Some(rest) = normalized.strip_prefix("git://") {
        normalized = format!("https://{rest}");
    }
    if let Some(rest) = normalized.strip_suffix(".git") {
        normalized = rest.to_string();
    }
    normalized
}

fn read_plugin_meta(dir: &Path) -> Option<PluginPackageJson> {
    let content = std::fs::read_to_string(dir.join("package.json")).ok()?;
    serde_json::from_str(&content).ok()
}

pub(crate) fn parse_plugins(profile: &Path, presets: &[PreinstallPluginInfo]) -> Vec<DshPlugin> {
    let manifest_content = match std::fs::read_to_string(profile.join("package.json")) {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };
    let manifest: ProfilePackageJson = match serde_json::from_str(&manifest_content) {
        Ok(manifest) => manifest,
        Err(_) => return Vec::new(),
    };

    let bundled: HashSet<&str> = manifest
        .dsh
        .as_ref()
        .and_then(|dsh| dsh.profile.as_ref())
        .map(|profile| profile.bundles.iter().map(String::as_str).collect())
        .unwrap_or_default();

    let preset_map: HashMap<&str, &PreinstallPluginInfo> =
        presets.iter().map(|p| (p.id.as_str(), p)).collect();

    let internal_names: HashSet<String> = presets
        .iter()
        .filter(|p| p.internal)
        .map(|p| installed_name(p).to_string())
        .collect();

    let mut dep_ids: Vec<&String> = manifest.dependencies.keys().collect();
    dep_ids.sort_by_key(|id| (!bundled.contains(id.as_str()), id.as_str()));

    dep_ids
        .into_iter()
        .map(|id| {
            let preset = preset_map.get(id.as_str());
            let meta = read_plugin_meta(&plugin_dir(profile, id));
            let repo_url = meta
                .as_ref()
                .and_then(|m| match &m.repository {
                    Some(RepositoryField::Url(url)) => Some(url.clone()),
                    Some(RepositoryField::Object { url }) => url.clone(),
                    None => m.homepage.clone(),
                })
                .or_else(|| preset.map(|p| p.repo_url.clone()))
                .map(|url| normalize_repo_url(&url))
                .unwrap_or_default();
            let name = meta
                .as_ref()
                .and_then(|m| m.name.clone())
                .or_else(|| preset.map(|p| p.name.clone()))
                .unwrap_or_else(|| id.clone());
            let installed_bundle = meta
                .as_ref()
                .is_some_and(|m| m.dsh.as_ref().and_then(|dsh| dsh.bundle.as_ref()).is_some());
            DshPlugin {
                id: id.clone(),
                name,
                version: meta
                    .as_ref()
                    .and_then(|m| m.version.clone())
                    .unwrap_or_default(),
                description: meta
                    .as_ref()
                    .and_then(|m| m.description.clone())
                    .or_else(|| preset.map(|p| p.description.clone()))
                    .unwrap_or_default(),
                repo_url,
                bundled: bundled.contains(id.as_str()),
                disabled: crate::service::plugin::disable::has_patch_disable(profile, id)
                    || (installed_bundle && !bundled.contains(id.as_str())),
                recommended: preset.map(|p| p.recommended).unwrap_or(false),
                fix: preset.map(|p| p.fix).unwrap_or(false),
                internal: internal_names.contains(id.as_str()),
                update_available: false,
                latest_version: None,
                has_snapshot: false,
                error: None,
            }
        })
        .collect()
}

fn merge_current_errors(plugins: &mut [DshPlugin], registry: &HashMap<String, PluginError>) {
    for plugin in plugins {
        plugin.error = registry.get(&plugin.id).and_then(|error| {
            let recovered_install = error.action == "install" && !plugin.version.is_empty();
            (!recovered_install).then(|| error.clone())
        });
    }
}

pub fn list(app_handle: &AppHandle) -> Vec<DshPlugin> {
    let presets = load_presets(app_handle);
    let mut plugins = parse_plugins(&profile_dir(app_handle), &presets);
    let registry = errors::load(app_handle);
    merge_current_errors(&mut plugins, &registry);
    for plugin in &mut plugins {
        plugin.has_snapshot = super::snapshot::has_snapshot(app_handle, &plugin.id);
    }
    plugins
}

pub fn force_emit(app_handle: &AppHandle) {
    let fp = fingerprint(app_handle);
    let profile = crate::service::profile::active_profile(app_handle);
    let mut states = STATE.get_or_init(Mutex::default).lock().unwrap();
    let state = states.entry(profile).or_default();
    state.pending_fp = None;
    state.last_fp = fp;
    drop(states);
    emit(app_handle);
}

/// 触发插件列表刷新（issue #399）。profile 未初始化（首次运行）时返回 None。
fn fingerprint(app_handle: &AppHandle) -> Option<String> {
    let dir = profile_dir(app_handle);
    let manifest = std::fs::read_to_string(dir.join("package.json")).ok()?;
    let parsed = serde_json::from_str::<ProfilePackageJson>(&manifest).ok()?;
    let mut dep_ids: Vec<&String> = parsed.dependencies.keys().collect();
    dep_ids.sort();

    let mut parts = vec![manifest];
    for id in dep_ids {
        if let Ok(content) = std::fs::read_to_string(plugin_dir(&dir, id).join("package.json")) {
            parts.push(content);
        }
    }
    if let Ok(content) = std::fs::read_to_string(dir.join("cordis.patch.yml")) {
        parts.push(content);
    }
    Some(parts.join("\n---\n"))
}

#[derive(Default)]
struct WatchState {
    last_fp: Option<String>,
    last_emit: Option<Instant>,
    pending_fp: Option<String>,
}

static STATE: OnceLock<Mutex<HashMap<String, WatchState>>> = OnceLock::new();

pub fn check_and_emit(app_handle: &AppHandle) {
    if instances::current_profile().is_none() {
        for profile in instances::profiles() {
            instances::scope_sync(profile, || check_and_emit(app_handle));
        }
        return;
    }
    let fp = fingerprint(app_handle);
    let profile = crate::service::profile::active_profile(app_handle);
    let mut states = STATE.get_or_init(Mutex::default).lock().unwrap();
    let state = states.entry(profile).or_default();

    if state.last_fp.as_deref() == fp.as_deref() {
        return;
    }
    state.pending_fp = fp;
    let can_emit = state
        .last_emit
        .is_none_or(|last| last.elapsed() >= DEBOUNCE);
    if !can_emit {
        return;
    }
    state.last_emit = Some(Instant::now());
    state.last_fp = state.pending_fp.take();
    emit(app_handle);
}

fn emit(app_handle: &AppHandle) {
    let plugins = list(app_handle);
    log::debug!(
        "dsh plugins changed, emitting {} plugin(s) to frontend",
        plugins.len()
    );
    instances::emit(app_handle, PLUGINS_UPDATED_EVENT, &plugins);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_profile(tag: &str, packages: &[(&str, &str)]) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dsh-watch-test-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        let mut manifest = serde_json::json!({
            "name": "dsh-profile-web",
            "private": true,
            "dependencies": {},
            "dsh": { "profile": { "bundles": [] } }
        });
        let mut deps = serde_json::Map::new();
        let mut bundles = Vec::new();
        for (id, meta_json) in packages {
            deps.insert((*id).to_string(), serde_json::Value::String("1.0.0".into()));
            let pkg_dir = dir.join("node_modules").join(id);
            std::fs::create_dir_all(&pkg_dir).unwrap();
            std::fs::write(pkg_dir.join("package.json"), *meta_json).unwrap();
            if meta_json.contains("\"dsh\"") {
                bundles.push((*id).to_string());
            }
        }
        manifest["dependencies"] = serde_json::Value::Object(deps);
        manifest["dsh"]["profile"]["bundles"] =
            serde_json::Value::Array(bundles.into_iter().map(serde_json::Value::String).collect());
        std::fs::write(
            dir.join("package.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        dir
    }

    fn presets_for_test() -> Vec<PreinstallPluginInfo> {
        vec![PreinstallPluginInfo {
            id: "dshmarket".into(),
            spec: "dshmarket".into(),
            name: "DSH Market".into(),
            description: "Visual plugin market".into(),
            repo_url: "https://github.com/dsh-market/dsh-market".into(),
            recommended: true,
            fix: false,
            default_checked: false,
            default_unchecked: false,
            version: None,
            win_only: false,
            package: None,
            internal: false,
        }]
    }

    #[test]
    fn watch_state_default_starts_without_fingerprints_or_debounce() {
        let state = Mutex::<WatchState>::default();
        let state = state.lock().unwrap();
        assert_eq!(state.last_fp, None);
        assert_eq!(state.last_emit, None);
        assert_eq!(state.pending_fp, None);
    }

    #[test]
    fn repo_url_normalization_preserves_existing_boundaries() {
        for (input, expected) in [
            ("  git+git://example/repo.git  ", "https://example/repo"),
            ("git+https://example/repo.git", "https://example/repo"),
            ("git://example/repo.git", "https://example/repo"),
            ("https://example/repo.git/", "https://example/repo.git/"),
            (
                "https://example/repo.git?x=1",
                "https://example/repo.git?x=1",
            ),
            ("git+ssh://example/repo.git", "ssh://example/repo"),
            (
                "git+git+https://example/repo.git",
                "git+https://example/repo",
            ),
            ("Git://example/repo.git", "Git://example/repo"),
            ("  ", ""),
        ] {
            assert_eq!(normalize_repo_url(input), expected, "{input}");
        }
    }

    #[test]
    fn parse_plugins_lists_direct_deps_with_meta() {
        let dir = build_profile(
            "meta",
            &[
                (
                    "dshmarket",
                    r#"{"name":"dshmarket","version":"1.13.1","description":"market","repository":{"type":"git","url":"git+https://github.com/dsh-market/dsh-market.git"},"dsh":{"bundle":{}}}"#,
                ),
                (
                    "@anionex/dsh-turn-rewind",
                    r#"{"name":"@anionex/dsh-turn-rewind","version":"0.1.1","description":"rewind"}"#,
                ),
            ],
        );
        let plugins = parse_plugins(&dir, &presets_for_test());
        assert_eq!(plugins.len(), 2);

        let market = plugins.iter().find(|p| p.id == "dshmarket").unwrap();
        assert!(market.bundled);
        assert!(market.recommended);
        assert_eq!(market.version, "1.13.1");
        assert_eq!(market.repo_url, "https://github.com/dsh-market/dsh-market");

        let rewind = plugins
            .iter()
            .find(|p| p.id == "@anionex/dsh-turn-rewind")
            .unwrap();
        assert!(!rewind.bundled);
        assert!(!rewind.recommended);
        assert_eq!(rewind.name, "@anionex/dsh-turn-rewind");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_plugins_falls_back_to_preset_and_sorts_bundled_first() {
        let dir = build_profile(
            "fallback",
            &[
                ("dsh-at-file", r#"{"name":"dsh-at-file"}"#),
                ("dshmarket", r#"{"name":"dshmarket","dsh":{"bundle":{}}}"#),
            ],
        );
        let plugins = parse_plugins(&dir, &presets_for_test());
        assert_eq!(plugins[0].id, "dshmarket");
        let market = &plugins[0];
        assert_eq!(market.version, "");
        assert_eq!(market.description, "Visual plugin market");
        assert_eq!(market.repo_url, "https://github.com/dsh-market/dsh-market");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_plugins_marks_internal_presets() {
        let dir = build_profile(
            "internal",
            &[
                (
                    "dsh-tauri",
                    r#"{"name":"dsh-tauri","version":"0.2.0","description":"bridge"}"#,
                ),
                ("dshmarket", r#"{"name":"dshmarket","version":"1.13.1"}"#),
            ],
        );
        let mut presets = presets_for_test();
        presets.push(PreinstallPluginInfo {
            id: "dsh-tauri".into(),
            spec: "dsh-tauri@0.2.0".into(),
            name: "DSH Tauri".into(),
            description: "Message bridge".into(),
            repo_url: "https://github.com/dsh-tauri-desk/dsh-tauri".into(),
            recommended: true,
            fix: false,
            default_checked: false,
            default_unchecked: false,
            version: None,
            win_only: false,
            package: None,
            internal: true,
        });
        let plugins = parse_plugins(&dir, &presets);
        assert_eq!(plugins.len(), 2);
        let tauri = plugins.iter().find(|p| p.id == "dsh-tauri").unwrap();
        assert!(tauri.internal);
        let market = plugins.iter().find(|p| p.id == "dshmarket").unwrap();
        assert!(!market.internal);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn recovered_install_error_is_not_exposed_as_current_error() {
        let mut plugins = vec![DshPlugin {
            id: "dsh-notification".into(),
            name: "DSH Notification".into(),
            version: "0.1.3".into(),
            description: String::new(),
            repo_url: String::new(),
            bundled: true,
            disabled: false,
            recommended: false,
            fix: false,
            internal: false,
            update_available: false,
            latest_version: None,
            has_snapshot: false,
            error: None,
        }];
        let registry = HashMap::from([(
            "dsh-notification".into(),
            PluginError {
                message: "PREINSTALL_SILENT_FAIL: missing artifact".into(),
                action: "install".into(),
                at: "1700000000".into(),
            },
        )]);

        merge_current_errors(&mut plugins, &registry);

        assert!(plugins[0].error.is_none());
    }

    #[test]
    fn unresolved_install_and_runtime_errors_remain_visible() {
        let plugin = DshPlugin {
            id: "dsh-notification".into(),
            name: "DSH Notification".into(),
            version: String::new(),
            description: String::new(),
            repo_url: String::new(),
            bundled: true,
            disabled: false,
            recommended: false,
            fix: false,
            internal: false,
            update_available: false,
            latest_version: None,
            has_snapshot: false,
            error: None,
        };
        let install_error = PluginError {
            message: "missing artifact".into(),
            action: "install".into(),
            at: "1700000000".into(),
        };
        let mut plugins = vec![plugin.clone()];
        merge_current_errors(
            &mut plugins,
            &HashMap::from([("dsh-notification".into(), install_error.clone())]),
        );
        assert_eq!(plugins[0].error, Some(install_error));

        let runtime_error = PluginError {
            message: "runtime failure".into(),
            action: "runtime".into(),
            at: "1700000001".into(),
        };
        let mut healthy_plugins = vec![DshPlugin {
            version: "0.1.3".into(),
            ..plugin
        }];
        merge_current_errors(
            &mut healthy_plugins,
            &HashMap::from([("dsh-notification".into(), runtime_error.clone())]),
        );
        assert_eq!(healthy_plugins[0].error, Some(runtime_error));
    }

    #[test]
    fn parse_plugins_returns_empty_without_manifest() {
        let dir = std::env::temp_dir().join(format!("dsh-watch-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(parse_plugins(&dir, &[]).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_plugins_distinguishes_disabled_from_unloaded() {
        let dir = std::env::temp_dir().join(format!("dsh-watch-disabled-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        let manifest = serde_json::json!({
            "name": "dsh-profile-web",
            "private": true,
            "dependencies": {
                "dsh-loaded": "1.0.0",
                "dsh-disabled": "1.0.0",
                "dsh-unloaded": "1.0.0"
            },
            "dsh": { "profile": { "bundles": ["dsh-loaded"] } }
        });
        std::fs::write(
            dir.join("package.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        for (id, declares_bundle) in [
            ("dsh-loaded", true),
            ("dsh-disabled", true),
            ("dsh-unloaded", false),
        ] {
            let pkg_dir = dir.join("node_modules").join(id);
            std::fs::create_dir_all(&pkg_dir).unwrap();
            let meta = if declares_bundle {
                serde_json::json!({ "name": id, "version": "1.0.0", "dsh": { "bundle": {} } })
            } else {
                serde_json::json!({ "name": id, "version": "1.0.0" })
            };
            std::fs::write(pkg_dir.join("package.json"), meta.to_string()).unwrap();
        }

        let plugins = parse_plugins(&dir, &[]);
        let loaded = plugins.iter().find(|p| p.id == "dsh-loaded").unwrap();
        assert!(loaded.bundled);
        assert!(!loaded.disabled);
        let disabled_plugin = plugins.iter().find(|p| p.id == "dsh-disabled").unwrap();
        assert!(!disabled_plugin.bundled);
        assert!(disabled_plugin.disabled);
        let unloaded = plugins.iter().find(|p| p.id == "dsh-unloaded").unwrap();
        assert!(!unloaded.bundled);
        assert!(!unloaded.disabled);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_plugins_marks_config_override_disabled() {
        let dir = build_profile(
            "patch",
            &[
                (
                    "dshmarket",
                    r#"{"name":"dshmarket","version":"1.13.1","dsh":{"bundle":{}}}"#,
                ),
                (
                    "dsh-tauri-pet",
                    r#"{"name":"dsh-tauri-pet","version":"0.1.0"}"#,
                ),
                ("dsh-other", r#"{"name":"dsh-other","version":"0.1.0"}"#),
            ],
        );
        std::fs::write(
            dir.join("cordis.patch.yml"),
            "- id: dshmarket\n  disabled: true\n- id: dsh-tauri-pet\n  disabled: false\n",
        )
        .unwrap();
        let plugins = parse_plugins(&dir, &presets_for_test());
        let market = plugins.iter().find(|p| p.id == "dshmarket").unwrap();
        assert!(market.disabled);
        let pet = plugins.iter().find(|p| p.id == "dsh-tauri-pet").unwrap();
        assert!(!pet.disabled);
        let other = plugins.iter().find(|p| p.id == "dsh-other").unwrap();
        assert!(!other.disabled);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_plugins_marks_patch_disabled_by_package_name_alias() {
        let dir = build_profile(
            "patch-alias",
            &[(
                "dsh-better-sidebar",
                r#"{"name":"better-sidebar","version":"1.0.0"}"#,
            )],
        );
        std::fs::write(
            dir.join("cordis.patch.yml"),
            "- id: better-sidebar\n  disabled: true\n",
        )
        .unwrap();
        let plugins = parse_plugins(&dir, &[]);
        let sidebar = plugins
            .iter()
            .find(|p| p.id == "dsh-better-sidebar")
            .unwrap();
        assert!(sidebar.disabled);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_plugins_marks_bundle_deselected_as_disabled() {
        let dir = build_profile(
            "bundle-deselected",
            &[(
                "dshmarket",
                r#"{"name":"dshmarket","version":"1.13.1","dsh":{"bundle":{}}}"#,
            )],
        );
        let manifest_path = dir.join("package.json");
        let mut manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        manifest["dsh"]["profile"]["bundles"] = serde_json::json!([]);
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let plugins = parse_plugins(&dir, &[]);
        let market = plugins.iter().find(|p| p.id == "dshmarket").unwrap();
        assert!(!market.bundled);
        assert!(market.disabled);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_plugins_marks_entry_id_patch_disable() {
        let dir = build_profile(
            "entry-id",
            &[(
                "dshmarket",
                r#"{"name":"dshmarket","version":"1.13.1","dsh":{"bundle":{"patch":"./cordis.patch.yml"}}}"#,
            )],
        );
        std::fs::write(
            dir.join("node_modules/dshmarket/cordis.patch.yml"),
            "- insert:\n    - id: dsh-market\n      name: dshmarket\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("cordis.patch.yml"),
            "- id: dsh-market\n  disabled: true\n",
        )
        .unwrap();

        let plugins = parse_plugins(&dir, &[]);
        assert!(
            plugins
                .iter()
                .find(|p| p.id == "dshmarket")
                .unwrap()
                .disabled
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
