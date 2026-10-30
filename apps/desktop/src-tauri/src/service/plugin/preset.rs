use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tauri::Manager;

use crate::AppHandle;
use crate::config;
use crate::config::manifest::{Manifest, PluginEntry, PluginVersion};

#[derive(Debug, Clone)]
pub struct PreinstallPluginInfo {
    pub id: String,
    pub spec: String,
    pub version: Option<PluginVersion>,
    pub internal: bool,
    pub package: Option<String>,
    pub name: String,
    pub description: String,
    pub repo_url: String,
    pub recommended: bool,
    pub fix: bool,
    pub default_checked: bool,
    pub default_unchecked: bool,
    pub win_only: bool,
}

impl PreinstallPluginInfo {
    pub(crate) fn unsupported_on(&self, core_version: Option<&str>) -> bool {
        self.version
            .as_ref()
            .is_some_and(|version| version.unsupported_on(core_version))
    }
}

fn plugin_info(entry: PluginEntry, internal: bool) -> PreinstallPluginInfo {
    let checked = entry.checked;
    PreinstallPluginInfo {
        id: entry.id,
        spec: entry.spec,
        version: entry.version,
        internal,
        package: entry.package,
        name: entry.name,
        description: entry.description,
        repo_url: entry.repo,
        recommended: entry.recommended,
        fix: entry.fix,
        default_checked: checked && !entry.recommended,
        default_unchecked: !checked,
        win_only: entry.win_only,
    }
}

#[cfg(debug_assertions)]
#[derive(Clone, Deserialize, Default)]
struct DevPluginPackageJson {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    private: bool,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    repository: Option<serde_json::Value>,
    #[serde(default)]
    dsh: Option<serde_json::Value>,
}

#[cfg(debug_assertions)]
struct DevPluginCandidate {
    info: PreinstallPluginInfo,
    directory: PathBuf,
}

#[cfg(debug_assertions)]
fn dev_plugins_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("packages")
}

#[cfg(debug_assertions)]
fn dev_repo_url(manifest: &DevPluginPackageJson) -> String {
    let repository = manifest.repository.as_ref().and_then(|repository| {
        repository
            .as_str()
            .or_else(|| repository.get("url").and_then(serde_json::Value::as_str))
    });
    repository
        .or(manifest.homepage.as_deref())
        .map(super::watch::normalize_repo_url)
        .unwrap_or_default()
}

#[cfg(debug_assertions)]
fn discover_dev_internal_plugins_at(root: &Path) -> Vec<DevPluginCandidate> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut entries: Vec<_> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|file_type| file_type.is_dir())
                .map(|_| entry)
        })
        .collect();
    entries.sort_by_key(|entry| entry.file_name());

    let mut seen = HashSet::new();
    entries
        .into_iter()
        .filter_map(|entry| {
            let directory = entry.path();
            let manifest_path = directory.join("package.json");
            let raw = std::fs::read_to_string(&manifest_path).ok()?;
            let manifest = match serde_json::from_str::<DevPluginPackageJson>(&raw) {
                Ok(manifest) => manifest,
                Err(error) => {
                    log::warn!(
                        "DEV_INTERNAL_PLUGIN_MANIFEST_INVALID: {}: {error}",
                        manifest_path.display()
                    );
                    return None;
                }
            };
            if manifest.private {
                return None;
            }
            if !manifest
                .dsh
                .as_ref()
                .is_some_and(serde_json::Value::is_object)
            {
                return None;
            }
            let repo_url = dev_repo_url(&manifest);
            let name = manifest.name.filter(|name| !name.trim().is_empty())?;
            if !seen.insert(name.clone()) {
                log::warn!("DEV_INTERNAL_PLUGIN_DUPLICATE: {name}");
                return None;
            }
            let info = PreinstallPluginInfo {
                id: name.clone(),
                spec: name.clone(),
                internal: true,
                package: Some(name.clone()),
                name: name.clone(),
                description: manifest.description.unwrap_or_default(),
                repo_url,
                recommended: false,
                fix: false,
                default_checked: false,
                default_unchecked: false,
                version: None,
                win_only: false,
            };
            Some(DevPluginCandidate { info, directory })
        })
        .collect()
}

#[cfg(debug_assertions)]
fn discover_dev_internal_plugins() -> Vec<DevPluginCandidate> {
    discover_dev_internal_plugins_at(&dev_plugins_root())
}

#[cfg(debug_assertions)]
fn dev_plugin_dir(id: &str) -> Option<PathBuf> {
    discover_dev_internal_plugins()
        .into_iter()
        .find(|candidate| candidate.info.id == id)
        .map(|candidate| candidate.directory)
}

#[cfg(debug_assertions)]
fn merge_dev_internal_plugins(internal: Vec<PreinstallPluginInfo>) -> Vec<PreinstallPluginInfo> {
    merge_dev_internal_plugins_at(&dev_plugins_root(), internal)
}

#[cfg(debug_assertions)]
fn merge_dev_internal_plugins_at(
    root: &Path,
    mut internal: Vec<PreinstallPluginInfo>,
) -> Vec<PreinstallPluginInfo> {
    let mut by_id: std::collections::HashMap<String, DevPluginCandidate> =
        discover_dev_internal_plugins_at(root)
            .into_iter()
            .map(|candidate| (candidate.info.id.clone(), candidate))
            .collect();
    for plugin in internal.iter_mut() {
        if let Some(candidate) = by_id.remove(&plugin.id) {
            let version = plugin.version.take();
            *plugin = candidate.info;
            plugin.version = version;
        }
    }
    internal.extend(by_id.into_values().map(|candidate| candidate.info));
    internal
}

const BUNDLED_PLUGINS_DIR: &str = "internal-plugins";
const LEGACY_BUNDLED_PLUGINS_DIR: &str = "preset-plugins";

fn find_bundled_in_root(root: &std::path::Path, id: &str) -> Option<PathBuf> {
    let probe = |base: &std::path::Path| {
        let resources = base.join("resources");
        let deployed = resources.join("node_modules").join(id);
        if deployed.join("package.json").exists() {
            return Some(deployed);
        }
        let legacy = resources.join(BUNDLED_PLUGINS_DIR).join(id);
        if legacy.join("package.json").exists() {
            return Some(legacy);
        }
        let flat_legacy = base.join(BUNDLED_PLUGINS_DIR).join(id);
        flat_legacy
            .join("package.json")
            .exists()
            .then_some(flat_legacy)
    };
    probe(root).or_else(|| {
        let deployed = root.join("node_modules").join(id);
        deployed.join("package.json").exists().then_some(deployed)
    })
}

pub(crate) fn bundled_plugin_dir(app_handle: &AppHandle, id: &str) -> Option<PathBuf> {
    bundled_plugin_dir_from(app_handle.path().resource_dir().ok().as_deref(), id)
}

pub(crate) fn bundled_plugin_dir_from(resources: Option<&Path>, id: &str) -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(dir) = dev_plugin_dir(id) {
        return Some(dir);
    }
    if let Some(candidate) = resources.and_then(|root| find_bundled_in_root(root, id)) {
        return Some(candidate);
    }
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
    let deployed = resources.join("node_modules").join(id);
    if deployed.join("package.json").exists() {
        return Some(deployed);
    }
    let legacy = resources.join(BUNDLED_PLUGINS_DIR).join(id);
    legacy.join("package.json").exists().then_some(legacy)
}

#[derive(Deserialize)]
struct PluginPackageJson {
    #[serde(default)]
    description: String,
}

fn bundled_plugin_description(resources: Option<&Path>, id: &str) -> Option<String> {
    let dir = bundled_plugin_dir_from(resources, id)?;
    let raw = std::fs::read_to_string(dir.join("package.json")).ok()?;
    package_json_description(&raw)
}

fn package_json_description(raw: &str) -> Option<String> {
    let manifest = serde_json::from_str::<PluginPackageJson>(raw).ok()?;
    let description = manifest.description.trim();
    (!description.is_empty()).then(|| description.to_string())
}

pub(crate) fn remove_legacy_bundled_plugins(app_handle: &AppHandle) -> Result<(), String> {
    let Ok(root) = app_handle.path().resource_dir() else {
        return Ok(());
    };
    let candidates = vec![
        root.join(LEGACY_BUNDLED_PLUGINS_DIR),
        root.join("resources").join(LEGACY_BUNDLED_PLUGINS_DIR),
        root.join(BUNDLED_PLUGINS_DIR),
        root.join("resources").join(BUNDLED_PLUGINS_DIR),
    ];
    remove_legacy_candidates(candidates, |path| std::fs::remove_dir_all(path))
}

fn remove_legacy_candidates(
    mut candidates: Vec<PathBuf>,
    mut remove: impl FnMut(&std::path::Path) -> std::io::Result<()>,
) -> Result<(), String> {
    candidates.sort();
    candidates.dedup();
    let mut failures = Vec::new();

    for path in candidates {
        if !path.is_dir() {
            continue;
        }
        match remove(&path) {
            Ok(()) => {
                log::info!(
                    "removed legacy bundled plugins directory: {}",
                    path.display()
                );
            }
            Err(e) => failures.push(format!(
                "LEGACY_PRESET_PLUGINS_REMOVE_FAILED: {}: {e}",
                path.display()
            )),
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

pub(crate) fn bundled_dep_spec(dir: &std::path::Path) -> String {
    let normalized = dunce::simplified(dir).to_string_lossy().replace('\\', "/");
    format!("link:{}", normalized.trim_end_matches('/'))
}

pub(crate) fn load_presets(app_handle: &AppHandle) -> Vec<PreinstallPluginInfo> {
    let Some(manifest) = config::manifest::read(app_handle) else {
        return Vec::new();
    };
    presets_from_manifest(&manifest, app_handle.path().resource_dir().ok().as_deref())
}

pub(crate) fn presets_from_manifest(
    manifest: &Manifest,
    resources: Option<&Path>,
) -> Vec<PreinstallPluginInfo> {
    let mut plugins: Vec<PreinstallPluginInfo> = manifest
        .plugins
        .preset
        .iter()
        .cloned()
        .map(|entry| plugin_info(entry, false))
        .collect();
    let internal: Vec<PreinstallPluginInfo> = manifest
        .plugins
        .built_in
        .iter()
        .cloned()
        .map(|entry| plugin_info(entry, true))
        .collect();
    #[cfg(debug_assertions)]
    let internal = merge_dev_internal_plugins(internal);
    plugins.extend(internal.into_iter().map(|mut plugin| {
        if let Some(description) = bundled_plugin_description(resources, &plugin.id) {
            plugin.description = description;
        }
        plugin
    }));
    plugins
}

pub(crate) fn load_deprecated_ids(app_handle: &AppHandle) -> HashSet<String> {
    config::manifest::read(app_handle)
        .map(|manifest| manifest.plugins.deprecated.into_iter().collect())
        .unwrap_or_default()
}

pub fn repo_url_of(app_handle: &AppHandle, id: &str) -> Option<String> {
    load_presets(app_handle)
        .into_iter()
        .find(|p| p.id == id)
        .map(|p| p.repo_url)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

pub(crate) fn current_preset_hash(app_handle: &AppHandle) -> Option<String> {
    let manifest = config::manifest::read(app_handle)?;
    let canonical = serde_json::to_string(&manifest.plugins).ok()?;
    Some(format!("{:016x}", fnv1a(canonical.as_bytes())))
}

pub(crate) fn preinstall_pending(app_handle: &AppHandle) -> bool {
    let setting = config::get_store_dat_setting(app_handle);
    if !setting.preinstall_done {
        return true;
    }
    match (
        setting.preset_hash.as_deref(),
        current_preset_hash(app_handle),
    ) {
        (None, Some(_)) => true,
        (Some(prev), Some(cur)) => prev != cur,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::manifest::{PluginVersion, VersionPair};

    fn manifest_path_for_test() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(crate::config::manifest::MANIFEST_FILE)
    }

    fn load_manifest_for_test() -> crate::config::manifest::Manifest {
        crate::config::manifest::read_at(&manifest_path_for_test())
            .expect("resource manifest should exist and parse")
    }

    fn plugin_infos_from(raw: &str, internal: bool) -> Vec<PreinstallPluginInfo> {
        let manifest = crate::config::manifest::parse(raw).expect("manifest should parse");
        let entries = if internal {
            manifest.plugins.built_in
        } else {
            manifest.plugins.preset
        };
        entries
            .into_iter()
            .map(|entry| plugin_info(entry, internal))
            .collect()
    }

    fn load_presets_for_test() -> Vec<PreinstallPluginInfo> {
        load_manifest_for_test()
            .plugins
            .preset
            .into_iter()
            .map(|entry| plugin_info(entry, false))
            .collect()
    }

    fn load_internal_for_test() -> Vec<PreinstallPluginInfo> {
        load_manifest_for_test()
            .plugins
            .built_in
            .into_iter()
            .map(|entry| plugin_info(entry, true))
            .collect()
    }

    fn load_all_plugins_for_test() -> Vec<PreinstallPluginInfo> {
        let mut plugins = load_presets_for_test();
        plugins.extend(load_internal_for_test());
        plugins
    }

    fn shipped_deprecated_ids() -> HashSet<String> {
        load_manifest_for_test()
            .plugins
            .deprecated
            .into_iter()
            .collect()
    }

    fn matrix(pairs: &[(&str, &str)]) -> PluginVersion {
        PluginVersion::Matrix(
            pairs
                .iter()
                .map(|(version, dsh)| VersionPair {
                    version: (*version).to_string(),
                    dsh: (*dsh).to_string(),
                })
                .collect(),
        )
    }

    #[test]
    fn preset_list_contains_dshmarket() {
        let presets = load_presets_for_test();
        assert!(presets.iter().any(|p| p.id == "dshmarket"));
        assert_eq!(
            presets
                .iter()
                .find(|p| p.id == "dshmarket")
                .map(|p| p.repo_url.as_str()),
            Some("https://github.com/dsh-market/dsh-market")
        );
        assert!(!presets.iter().any(|p| p.id == "unknown-package"));
    }

    #[test]
    fn preset_manifest_specs_match_ids() {
        let presets = load_presets_for_test();
        for preset in &presets {
            assert_eq!(preset.spec, preset.id);
        }
    }

    #[test]
    fn office_kit_requires_core_with_user_text_api() {
        let entry = load_presets_for_test()
            .into_iter()
            .find(|p| p.id == "dsh-office-kit")
            .expect("dsh-office-kit");
        assert!(entry.unsupported_on(Some("0.2.0-rc.2")));
        for core in ["0.2.1-rc.1", "0.2.1-rc.2", "0.2.1"] {
            assert!(!entry.unsupported_on(Some(core)), "core={core}");
        }
        assert_eq!(
            entry
                .version
                .as_ref()
                .and_then(|version| version.plugin_req_for_core(Some("0.2.1-rc.2"))),
            Some("latest")
        );
    }

    #[test]
    fn prerelease_core_in_same_generation_keeps_plugins() {
        let mut entry = load_presets_for_test()
            .into_iter()
            .find(|p| p.id == "dshmarket")
            .expect("dshmarket");
        entry.version = Some(matrix(&[("^0.24.1", "^0.2.0-rc.1")]));
        for core in ["0.2.0-rc.1", "0.2.0-rc.2", "0.2.0", "0.2.1-rc.1"] {
            assert!(!entry.unsupported_on(Some(core)), "core={core}");
        }
    }

    #[test]
    fn plugin_version_deserializes_as_optional_metadata_without_changing_spec() {
        for (field, expected) in [
            ("", None),
            (r#", "version": null"#, None),
            (
                r#", "version": "latest""#,
                Some(PluginVersion::Declared("latest".into())),
            ),
            (
                r#", "version": "1.2.3""#,
                Some(PluginVersion::Declared("1.2.3".into())),
            ),
            (
                r#", "version": [{"version": "^0.19.1", "dsh": "^0.1.5-rc.1"}]"#,
                Some(matrix(&[("^0.19.1", "^0.1.5-rc.1")])),
            ),
        ] {
            let raw = format!(
                r#"{{"plugins":{{"preset":[{{"id":"dshmarket","spec":"dshmarket","name":"Market","description":"","repo":"u"{field}}}]}}}}"#
            );
            let plugins = plugin_infos_from(&raw, false);
            assert_eq!(plugins.len(), 1);
            assert_eq!(plugins[0].version, expected);
            assert_eq!(plugins[0].spec, "dshmarket");
        }
    }

    #[test]
    fn internal_manifest_defaults_to_compatible_and_honours_declared_ceiling() {
        let internal = load_internal_for_test();
        assert!(!internal.is_empty());

        assert!(
            internal
                .iter()
                .all(|p| p.version.is_none() && !p.unsupported_on(Some("9.9.9"))),
            "no internal entry declares a core matrix, so every one stays compatible"
        );

        let mut capped = internal[0].clone();
        capped.version = Some(matrix(&[("^1.0.0", "^0.1.5-rc.1")]));
        assert!(!capped.unsupported_on(Some("0.1.5-rc.1")));
        assert!(!capped.unsupported_on(Some("0.1.6")));
        assert!(!capped.unsupported_on(Some("0.1.7-rc.1")));
        assert!(capped.unsupported_on(Some("0.1.4-rc.1")));
        assert!(capped.unsupported_on(Some("0.2.0")));
    }

    #[test]
    fn plugin_manifest_ids_are_unique_across_files() {
        let plugins = load_all_plugins_for_test();
        let ids: std::collections::HashSet<&str> = plugins.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids.len(),
            plugins.len(),
            "plugin ids must be unique across manifests"
        );
    }

    #[test]
    fn internal_manifest_marks_all_entries_internal() {
        let plugins = load_internal_for_test();
        assert!(!plugins.is_empty());
        assert!(plugins.iter().all(|plugin| plugin.internal));
        assert!(plugins.iter().any(|plugin| plugin.id == "dsh-tauri"));
        assert!(!load_presets_for_test().iter().any(|plugin| plugin.internal));
    }

    #[test]
    fn manifest_reads_in_both_flat_and_nested_resource_layouts() {
        let dir = std::env::temp_dir().join(format!("dsh-manifest-layout-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let nested = dir.join("resources");
        std::fs::create_dir_all(&nested).expect("create temp resources dir");
        let body = r#"{"plugins":{"preset":[{"id":"x","spec":"y","name":"X","description":"","repo":"u"}]}}"#;

        std::fs::write(nested.join(crate::config::manifest::MANIFEST_FILE), body)
            .expect("write nested manifest");
        assert_eq!(
            crate::config::manifest::read_at(&nested.join(crate::config::manifest::MANIFEST_FILE))
                .expect("nested layout manifest should parse")
                .plugins
                .preset
                .len(),
            1
        );

        std::fs::write(dir.join(crate::config::manifest::MANIFEST_FILE), body)
            .expect("write flat manifest");
        assert_eq!(
            crate::config::manifest::read_at(&dir.join(crate::config::manifest::MANIFEST_FILE))
                .expect("flat layout manifest should parse")
                .plugins
                .preset
                .len(),
            1
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn deprecated_ids_parse_into_set() {
        let ids: HashSet<String> = crate::config::manifest::parse(
            r#"{"plugins":{"depercated":["dsh-a","dsh-b","dsh-a"]}}"#,
        )
        .expect("deprecated manifest should parse")
        .plugins
        .deprecated
        .into_iter()
        .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("dsh-a"));
        assert!(ids.contains("dsh-b"));
    }

    #[test]
    fn deprecated_ids_reject_non_string_entries() {
        assert!(crate::config::manifest::parse(r#"{"plugins":{"depercated":[42]}}"#).is_err());
        assert!(
            crate::config::manifest::parse(r#"{"plugins":{"depercated":{"id":"dsh-x"}}}"#).is_err()
        );
    }

    #[test]
    fn deprecated_manifest_lists_session_context_menu() {
        assert!(shipped_deprecated_ids().contains("dsh-session-context-menu"));
    }

    #[test]
    fn deprecated_manifest_lists_absorbed_panel() {
        assert!(shipped_deprecated_ids().contains("dsh-tauri-panel"));
    }

    #[test]
    fn deprecated_manifest_lists_absorbed_connection() {
        assert!(shipped_deprecated_ids().contains("dsh-tauri-connection"));
    }

    #[test]
    fn deprecated_manifest_lists_the_renamed_bridge() {
        assert!(shipped_deprecated_ids().contains("dsh-tauri-bridge"));
    }

    #[test]
    fn deprecated_manifest_lists_the_retired_kernel() {
        assert!(shipped_deprecated_ids().contains("dsh-tauri-kernel"));
    }

    #[test]
    fn deprecated_manifest_lists_absorbed_running_changes() {
        assert!(shipped_deprecated_ids().contains("dsh-tauri-running-changes"));
    }

    #[test]
    fn manifest_source_overrides_internal_field() {
        let raw = r#"{"plugins":{
            "preset":[{"id":"x","spec":"y","internal":true,"name":"X","description":"","repo":"u"}],
            "built-in":[{"id":"x","spec":"y","internal":false,"name":"X","description":"","repo":"u"}]
        }}"#;
        let preset = plugin_infos_from(raw, false);
        assert!(!preset[0].internal);
        let internal = plugin_infos_from(raw, true);
        assert!(internal[0].internal);
    }

    #[test]
    fn legacy_cleanup_continues_after_failure_and_aggregates_errors() {
        let root = std::env::temp_dir().join(format!("dsh-legacy-cleanup-{}", std::process::id()));
        let first = root.join("a");
        let second = root.join("b");
        std::fs::create_dir_all(&first).expect("create first legacy dir");
        std::fs::create_dir_all(&second).expect("create second legacy dir");
        let mut attempted = Vec::new();

        let error = remove_legacy_candidates(vec![first.clone(), second.clone()], |path| {
            attempted.push(path.to_path_buf());
            if path == first {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "locked",
                ))
            } else {
                std::fs::remove_dir_all(path)
            }
        })
        .expect_err("one failed cleanup should be reported");

        assert_eq!(attempted, vec![first.clone(), second.clone()]);
        assert!(error.contains("LEGACY_PRESET_PLUGINS_REMOVE_FAILED"));
        assert!(error.contains(&first.display().to_string()));
        assert!(error.contains("locked"));
        assert!(first.is_dir());
        assert!(!second.exists());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bundled_dir_discovers_nested_layout() {
        let dir = std::env::temp_dir().join(format!("dsh-bundled-nested-{}", std::process::id()));
        let nested = dir
            .join("resources")
            .join(BUNDLED_PLUGINS_DIR)
            .join("dsh-tauri");
        std::fs::create_dir_all(&nested).expect("create temp nested bundled dir");
        std::fs::write(nested.join("package.json"), "{}").expect("write bundle manifest");

        let found =
            find_bundled_in_root(&dir, "dsh-tauri").expect("nested bundled dir should be found");
        assert_eq!(found, nested);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bundled_dir_discovers_flat_layout() {
        let dir = std::env::temp_dir().join(format!("dsh-bundled-flat-{}", std::process::id()));
        let flat = dir.join(BUNDLED_PLUGINS_DIR).join("dsh-tauri");
        std::fs::create_dir_all(&flat).expect("create temp flat bundled dir");
        std::fs::write(flat.join("package.json"), "{}").expect("write bundle manifest");

        let found =
            find_bundled_in_root(&dir, "dsh-tauri").expect("flat bundled dir should be found");
        assert_eq!(found, flat);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bundled_dir_requires_package_json() {
        let dir = std::env::temp_dir().join(format!("dsh-bundled-empty-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(BUNDLED_PLUGINS_DIR).join("dsh-tauri"))
            .expect("create empty dir");
        assert!(find_bundled_in_root(&dir, "dsh-tauri").is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bundled_dep_spec_normalizes_windows_separators() {
        assert_eq!(
            bundled_dep_spec(std::path::Path::new(
                "C:\\Apps\\dsh\\resources\\internal-plugins\\dsh-tauri"
            )),
            "link:C:/Apps/dsh/resources/internal-plugins/dsh-tauri"
        );
        assert_eq!(
            bundled_dep_spec(std::path::Path::new("/opt/dsh/plugins/x/")),
            "link:/opt/dsh/plugins/x"
        );
    }

    #[cfg(windows)]
    #[test]
    fn bundled_dep_spec_strips_verbatim_prefix() {
        let dir = std::path::Path::new(
            r"\\?\G:\Deepseek Harness Desktop\resources\internal-plugins\dsh-tauri",
        );
        assert_eq!(
            bundled_dep_spec(dir),
            "link:G:/Deepseek Harness Desktop/resources/internal-plugins/dsh-tauri"
        );
    }

    #[test]
    fn fnv1a_matches_known_vectors() {
        assert_eq!(fnv1a(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn same_content_same_hash_appended_comma_changes_hash() {
        let a = r#"[{"id":"x","spec":"y","name":"X","description":"","repoUrl":"u"}]"#;
        let b = r#"[{"id":"x","spec":"y","name":"X","description":"","repoUrl":"u"},]"#;
        assert_eq!(fnv1a(a.as_bytes()), fnv1a(a.as_bytes()));
        assert_ne!(fnv1a(a.as_bytes()), fnv1a(b.as_bytes()));
    }

    #[test]
    fn pending_decision_matrix() {
        assert!(preinstall_pending_for_test(false, None, Some("h1")));
        assert!(preinstall_pending_for_test(false, Some("h1"), Some("h1")));
        assert!(preinstall_pending_for_test(true, None, Some("h1")));
        assert!(!preinstall_pending_for_test(true, Some("h1"), Some("h1")));
        assert!(preinstall_pending_for_test(true, Some("h1"), Some("h2")));
        assert!(!preinstall_pending_for_test(true, Some("h1"), None));
        assert!(!preinstall_pending_for_test(true, None, None));
    }

    fn preinstall_pending_for_test(
        preinstall_done: bool,
        recorded: Option<&str>,
        current: Option<&str>,
    ) -> bool {
        if !preinstall_done {
            return true;
        }
        match (recorded, current) {
            (None, Some(_)) => true,
            (Some(prev), Some(cur)) => prev != cur,
            _ => false,
        }
    }

    #[cfg(debug_assertions)]
    fn write_dev_manifest(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).expect("create dev package dir");
        let manifest = serde_json::json!({
            "name": name,
            "description": "desc",
            "dsh": {"client": {"inject": ["x"]}},
        });
        std::fs::write(
            dir.join("package.json"),
            serde_json::to_string(&manifest).expect("serialize dev manifest"),
        )
        .expect("write dev manifest");
    }

    #[cfg(debug_assertions)]
    fn temp_dev_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dsh-dev-{label}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).expect("create temp dev root");
        root
    }

    #[cfg(debug_assertions)]
    #[test]
    fn dev_repo_url_keeps_repository_precedence_and_homepage_fallback() {
        for (json, expected) in [
            (
                r#"{"repository":" git+git://example/repo.git ","homepage":"https://fallback"}"#,
                "https://example/repo",
            ),
            (
                r#"{"repository":{"url":"git://example/object.git"}}"#,
                "https://example/object",
            ),
            (
                r#"{"repository":{"url":42},"homepage":"git+https://example/home.git"}"#,
                "https://example/home",
            ),
            (r#"{"repository":"","homepage":"https://fallback"}"#, ""),
            (r#"{}"#, ""),
        ] {
            let manifest = serde_json::from_str(json).unwrap();
            assert_eq!(dev_repo_url(&manifest), expected, "{json}");
        }
    }

    #[cfg(debug_assertions)]
    #[test]
    fn dev_discovery_empty_root_returns_none() {
        let root = temp_dev_root("empty");
        assert!(discover_dev_internal_plugins_at(&root).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn dev_discovery_skips_invalid_manifest() {
        let root = temp_dev_root("invalid");
        let dir = root.join("bad");
        std::fs::create_dir_all(&dir).expect("create dir");
        std::fs::write(dir.join("package.json"), "{ not json").expect("write bad manifest");
        assert!(discover_dev_internal_plugins_at(&root).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn dev_discovery_skips_when_missing_or_invalid_dsh_value() {
        let root = temp_dev_root("dsh");
        std::fs::create_dir_all(root.join("no-dsh")).expect("create dir");
        std::fs::write(
            root.join("no-dsh").join("package.json"),
            r#"{"name":"no-dsh","description":"x"}"#,
        )
        .expect("write manifest");
        for (name, dsh) in [
            ("str-dsh", r#""x""#),
            ("arr-dsh", "[]"),
            ("null-dsh", "null"),
        ] {
            std::fs::create_dir_all(root.join(name)).expect("create dir");
            std::fs::write(
                root.join(name).join("package.json"),
                format!(r#"{{"name":"{name}","dsh":{dsh}}}"#),
            )
            .expect("write manifest");
        }
        assert!(discover_dev_internal_plugins_at(&root).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn dev_discovery_skips_private_manifest() {
        let root = temp_dev_root("private");
        std::fs::create_dir_all(root.join("placeholder")).expect("create dir");
        std::fs::write(
            root.join("placeholder").join("package.json"),
            r#"{"name":"placeholder","private":true,"dsh":{"client":{"inject":["x"]}}}"#,
        )
        .expect("write manifest");
        assert!(discover_dev_internal_plugins_at(&root).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn dev_discovery_discovers_real_package_name_regardless_of_dirname() {
        let root = temp_dev_root("name");
        write_dev_manifest(&root.join("some-dir"), "@scope/dsh-plugin");
        let found = discover_dev_internal_plugins_at(&root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].info.id, "@scope/dsh-plugin");
        assert_eq!(found[0].info.spec, "@scope/dsh-plugin");
        assert_eq!(found[0].info.package.as_deref(), Some("@scope/dsh-plugin"));
        assert!(found[0].info.internal);
        assert!(found[0].directory.ends_with("some-dir"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn dev_discovery_dedups_by_package_name() {
        let root = temp_dev_root("dedup");
        write_dev_manifest(&root.join("a"), "dsh-tauri");
        write_dev_manifest(&root.join("b"), "dsh-tauri");
        let found = discover_dev_internal_plugins_at(&root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].info.id, "dsh-tauri");
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn dev_discovery_sorts_by_directory_name() {
        let root = temp_dev_root("sort");
        write_dev_manifest(&root.join("b-plugin"), "b");
        write_dev_manifest(&root.join("a-plugin"), "a");
        let found = discover_dev_internal_plugins_at(&root);
        let ids: Vec<&str> = found.iter().map(|c| c.info.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn dev_discovery_sets_repo_url_from_repository_object() {
        let root = temp_dev_root("repo");
        let dir = root.join("repo-plugin");
        std::fs::create_dir_all(&dir).expect("create dir");
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"repo-plugin","repository":{"url":"git+https://github.com/x/repo.git"},"dsh":{"client":{"inject":["x"]}}}"#,
        )
        .expect("write manifest");
        let found = discover_dev_internal_plugins_at(&root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].info.repo_url, "https://github.com/x/repo");
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn dev_discovery_merge_overrides_static_entry_and_appends_new() {
        let root = temp_dev_root("merge");
        write_dev_manifest(&root.join("dsh-tauri"), "dsh-tauri");
        write_dev_manifest(&root.join("brand-new"), "brand-new");
        let static_internal = vec![
            PreinstallPluginInfo {
                id: "dsh-tauri".into(),
                spec: "dsh-tauri".into(),
                internal: true,
                package: Some("dsh-tauri".into()),
                name: "dsh-tauri".into(),
                description: "static desc".into(),
                repo_url: "static".into(),
                recommended: false,
                fix: false,
                default_checked: false,
                default_unchecked: false,
                version: None,
                win_only: false,
            },
            PreinstallPluginInfo {
                id: "keep-static".into(),
                spec: "keep-static".into(),
                internal: true,
                package: Some("keep-static".into()),
                name: "keep-static".into(),
                description: String::new(),
                repo_url: String::new(),
                recommended: false,
                fix: false,
                default_checked: false,
                default_unchecked: false,
                version: None,
                win_only: false,
            },
        ];
        let merged = merge_dev_internal_plugins_at(&root, static_internal);
        let ids: std::collections::HashSet<&str> = merged.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids.len(), 3);
        let dsh = merged.iter().find(|p| p.id == "dsh-tauri").unwrap();
        assert_eq!(dsh.description, "desc"); // dev 覆盖静态
        assert!(merged.iter().any(|p| p.id == "brand-new")); // 追加上去
        assert!(merged.iter().any(|p| p.id == "keep-static")); // 静态未覆盖项保留
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn dev_merge_carries_static_version_declaration() {
        let root = temp_dev_root("merge-cap");
        write_dev_manifest(
            &root.join("dsh-tauri-experimental"),
            "dsh-tauri-experimental",
        );
        let declared = matrix(&[("^1.2.3", "^0.1.7-alpha.0")]);
        let static_internal = vec![PreinstallPluginInfo {
            id: "dsh-tauri-experimental".into(),
            spec: "dsh-tauri-experimental".into(),
            internal: true,
            package: Some("dsh-tauri-experimental".into()),
            name: "DSH Tauri Experimental".into(),
            description: String::new(),
            repo_url: String::new(),
            recommended: false,
            fix: false,
            default_checked: false,
            default_unchecked: false,
            version: Some(declared.clone()),
            win_only: false,
        }];

        let merged = merge_dev_internal_plugins_at(&root, static_internal);
        let renamed = merged
            .iter()
            .find(|p| p.id == "dsh-tauri-experimental")
            .expect("merged entry must exist");
        assert_eq!(renamed.version.as_ref(), Some(&declared));
        assert_eq!(renamed.spec, "dsh-tauri-experimental");
        assert_eq!(renamed.description, "desc");
        assert!(!renamed.unsupported_on(Some("0.1.7-alpha.0")));
        assert!(renamed.unsupported_on(Some("0.2.0")));
        assert!(renamed.internal);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn package_json_description_is_the_description_source() {
        assert_eq!(
            package_json_description(r#"{"description":"  hello  "}"#).as_deref(),
            Some("hello")
        );
        assert_eq!(package_json_description(r#"{"description":"   "}"#), None);
        assert_eq!(package_json_description(r#"{"name":"x"}"#), None);
        assert_eq!(package_json_description("not json"), None);
    }

    #[test]
    fn internal_manifest_entries_carry_no_description() {
        let declared: Vec<String> = load_internal_for_test()
            .into_iter()
            .filter(|plugin| !plugin.description.is_empty())
            .map(|plugin| plugin.id)
            .collect();
        assert!(
            declared.is_empty(),
            "内置插件描述的唯一真值是 package.json，清单不该再登记：{declared:?}"
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn every_builtin_plugin_package_declares_a_description() {
        let missing: Vec<String> = load_internal_for_test()
            .into_iter()
            .filter(|plugin| {
                dev_plugin_dir(&plugin.id)
                    .and_then(|dir| std::fs::read_to_string(dir.join("package.json")).ok())
                    .and_then(|raw| package_json_description(&raw))
                    .is_none()
            })
            .map(|plugin| plugin.id)
            .collect();
        assert!(
            missing.is_empty(),
            "内置插件必须在 package.json 声明 description：{missing:?}"
        );
    }
}
