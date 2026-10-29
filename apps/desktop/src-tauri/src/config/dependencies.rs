
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Runtime};

use super::manifest;
use crate::{AppHandle};

pub const MAPPING_FILE: &str = "dependencies.json";

pub const DEP_PNPM: &str = "pnpm";
pub const DEP_DSH: &str = "dsh";

static MAPPING_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DependencyMapping(pub BTreeMap<String, Option<String>>);

impl DependencyMapping {
    pub fn get(&self, key: &str) -> Option<Option<PathBuf>> {
        self.0
            .get(key)
            .map(|value| value.as_ref().map(PathBuf::from))
    }

    pub fn set(&mut self, key: &str, value: Option<PathBuf>) {
        self.0.insert(
            key.to_string(),
            value.map(|path| path.to_string_lossy().into_owned()),
        );
    }
}

pub fn mapping_path<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    super::runtime::get_base_dir(app).join(MAPPING_FILE)
}

static CACHE: Mutex<Option<(PathBuf, u64, DependencyMapping)>> = Mutex::new(None);

fn cached(path: &Path, stamp: u64) -> Option<DependencyMapping> {
    let guard = CACHE.lock().unwrap_or_else(|error| error.into_inner());
    guard
        .as_ref()
        .filter(|(cached_path, cached_stamp, _)| cached_path == path && *cached_stamp == stamp)
        .map(|(_, _, mapping)| mapping.clone())
}

fn store_cache(path: &Path, stamp: u64, mapping: &DependencyMapping) {
    let mut guard = CACHE.lock().unwrap_or_else(|error| error.into_inner());
    *guard = Some((path.to_path_buf(), stamp, mapping.clone()));
}

pub fn read_mapping<R: Runtime>(app: &AppHandle<R>) -> DependencyMapping {
    read_mapping_cached(&mapping_path(app))
}

fn read_mapping_cached(path: &Path) -> DependencyMapping {
    let stamp = super::file_stamp(path);
    if let Some(mapping) = cached(path, stamp) {
        return mapping;
    }
    let mapping = read_mapping_at(path);
    store_cache(path, stamp, &mapping);
    mapping
}

pub(crate) fn read_mapping_at(path: &Path) -> DependencyMapping {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return DependencyMapping::default();
    };
    match serde_json::from_str(&raw) {
        Ok(mapping) => mapping,
        Err(error) => {
            log::warn!(
                "DEPENDENCY_MAPPING_INVALID: {}: {error}; falling back to manifest defaults",
                path.display()
            );
            DependencyMapping::default()
        }
    }
}

pub fn write_mapping<R: Runtime>(
    app: &AppHandle<R>,
    mapping: &DependencyMapping,
) -> Result<(), String> {
    let path = mapping_path(app);
    let body = serde_json::to_string_pretty(mapping).map_err(|error| error.to_string())?;
    if std::fs::read_to_string(&path).is_ok_and(|current| current == body) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("DEPENDENCY_MAPPING_WRITE_FAILED: {error}"))?;
    }
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &body)
        .map_err(|error| format!("DEPENDENCY_MAPPING_WRITE_FAILED: {error}"))?;
    std::fs::rename(&temp, &path).map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        format!("DEPENDENCY_MAPPING_WRITE_FAILED: {error}")
    })?;
    store_cache(&path, super::file_stamp(&path), mapping);
    Ok(())
}

pub fn record<R: Runtime>(app: &AppHandle<R>, key: &str, value: Option<PathBuf>) {
    let _guard = MAPPING_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let current = read_mapping(app);
    let mut next = current.clone();
    next.set(key, value);
    if next.0 == current.0 {
        return;
    }
    if let Err(error) = write_mapping(app, &next) {
        log::warn!("DEPENDENCY_MAPPING_RECORD_FAILED: {key}: {error}");
    } else {
        log::info!(
            "dependency mapping updated: {key} = {}",
            next.0
                .get(key)
                .and_then(|value| value.clone())
                .unwrap_or_else(|| "<system>".to_string())
        );
    }
}

pub fn mapped<R: Runtime>(app: &AppHandle<R>, key: &str) -> Option<Option<PathBuf>> {
    read_mapping(app).get(key)
}

pub fn managed_root<R: Runtime>(app: &AppHandle<R>, key: &str) -> PathBuf {
    let base = super::runtime::get_base_dir(app);
    let resources = manifest::resource_root(app);
    manifest::with_manifest(app, |manifest| {
        managed_root_from(&base, resources.as_deref(), Some(manifest), key)
    })
    .unwrap_or_else(|| managed_root_from(&base, resources.as_deref(), None, key))
}

pub(crate) fn managed_root_from(
    base: &Path,
    resources: Option<&Path>,
    manifest: Option<&manifest::Manifest>,
    key: &str,
) -> PathBuf {
    match manifest
        .and_then(|manifest| manifest.dependencies.get(key))
        .and_then(|spec| spec.managed_root.as_deref())
    {
        Some(raw) => manifest::resolve_location_from(base, resources, raw),
        None => base.join(default_managed_root(key)),
    }
}

pub fn active_root<R: Runtime>(app: &AppHandle<R>, key: &str) -> PathBuf {
    let base = super::runtime::get_base_dir(app);
    let resources = manifest::resource_root(app);
    let mapping = read_mapping(app);
    manifest::with_manifest(app, |manifest| {
        active_root_from(&base, resources.as_deref(), Some(manifest), &mapping, key)
    })
    .unwrap_or_else(|| active_root_from(&base, resources.as_deref(), None, &mapping, key))
}

pub(crate) fn active_root_from(
    base: &Path,
    resources: Option<&Path>,
    manifest: Option<&manifest::Manifest>,
    mapping: &DependencyMapping,
    key: &str,
) -> PathBuf {
    let overridable = manifest
        .and_then(|manifest| manifest.dependencies.get(key))
        .is_none_or(|spec| spec.overridable);
    if overridable {
        if let Some(Some(recorded)) = mapping.get(key) {
            let resolved =
                manifest::resolve_location_from(base, resources, &recorded.to_string_lossy());
            if resolved.exists() {
                return resolved;
            }
        }
    }
    managed_root_from(base, resources, manifest, key)
}

pub fn bundled_core_dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    let root = manifest::resource_root(app)?;
    let managed = managed_root(app, DEP_DSH);
    (managed != root && managed.starts_with(&root)).then_some(managed)
}

pub fn entry_relative<R: Runtime>(app: &AppHandle<R>, key: &str) -> PathBuf {
    manifest::with_manifest(app, |manifest| entry_relative_from(Some(manifest), key))
        .unwrap_or_else(|| entry_relative_from(None, key))
}

pub(crate) fn entry_relative_from(manifest: Option<&manifest::Manifest>, key: &str) -> PathBuf {
    let declared = manifest
        .and_then(|manifest| manifest.dependencies.get(key))
        .and_then(|spec| spec.entry.resolve());
    split_entry(declared.unwrap_or_else(|| default_entry(key)))
}

pub fn binary_path<R: Runtime>(app: &AppHandle<R>, key: &str) -> PathBuf {
    active_root(app, key).join(entry_relative(app, key))
}

fn split_entry(raw: &str) -> PathBuf {
    raw.split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn default_managed_root(key: &str) -> &'static str {
    match key {
        DEP_PNPM => "dependencies/pnpm",
        DEP_DSH => "dependencies/dsh",
        _ => "dependencies",
    }
}

fn default_entry(key: &str) -> &'static str {
    match key {
        DEP_PNPM => "bin/pnpm.cjs",
        DEP_DSH => "dsh.asar/node_modules/@deepseek-ai/dsh/lib/bin.js",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "dsh-deps-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }

    #[test]
    fn mapping_round_trips_system_and_path_roots() {
        let dir = temp_path("mapping");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(MAPPING_FILE);

        let mut mapping = DependencyMapping::default();
        mapping.set(DEP_DSH, Some(PathBuf::from("C:/app/dsh")));
        mapping.set(DEP_PNPM, None);
        std::fs::write(&file, serde_json::to_string_pretty(&mapping).unwrap()).unwrap();

        let read = read_mapping_at(&file);
        assert_eq!(read.get(DEP_DSH), Some(Some(PathBuf::from("C:/app/dsh"))));
        assert_eq!(read.get(DEP_PNPM), Some(None));
        assert!(read.get(DEP_DSH).is_some_and(|value| value.is_some()));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_mapping_falls_back_to_empty_table() {
        let dir = temp_path("invalid");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(MAPPING_FILE);
        std::fs::write(&file, "{ not json").unwrap();
        assert!(read_mapping_at(&file).0.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn root_resolution_preserves_mapping_and_manifest_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("data");
        let resources = dir.path().join("resources");
        std::fs::create_dir_all(resources.join("mapped")).unwrap();
        let mut manifest = manifest::parse(
            r#"{"dependencies":{"dsh":{"managedRoot":"$AppData/default-core","overridable":true,"entry":"lib\\bin.js"}}}"#,
        )
        .unwrap();
        let mut mapping = DependencyMapping::default();
        let resolve = |manifest: Option<&manifest::Manifest>, mapping: &DependencyMapping| {
            active_root_from(&base, Some(&resources), manifest, mapping, DEP_DSH)
        };
        assert_eq!(
            resolve(Some(&manifest), &mapping),
            base.join("default-core")
        );
        mapping.set(DEP_DSH, None);
        assert_eq!(
            resolve(Some(&manifest), &mapping),
            base.join("default-core")
        );
        mapping.set(DEP_DSH, Some("$Resources/missing".into()));
        assert_eq!(
            resolve(Some(&manifest), &mapping),
            base.join("default-core")
        );
        mapping.set(DEP_DSH, Some("$Resources/mapped".into()));
        assert_eq!(resolve(Some(&manifest), &mapping), resources.join("mapped"));
        manifest.dependencies.get_mut(DEP_DSH).unwrap().overridable = false;
        assert_eq!(
            resolve(Some(&manifest), &mapping),
            base.join("default-core")
        );
        assert_eq!(resolve(None, &mapping), resources.join("mapped"));
        mapping.set(DEP_DSH, Some("$AppData/missing".into()));
        assert_eq!(resolve(None, &mapping), base.join("dependencies/dsh"));
        assert_eq!(
            managed_root_from(&base, None, Some(&manifest), DEP_DSH),
            base.join("default-core")
        );
        assert_eq!(
            entry_relative_from(Some(&manifest), DEP_DSH),
            PathBuf::from("lib").join("bin.js")
        );
        assert_eq!(
            entry_relative_from(None, DEP_PNPM),
            PathBuf::from("bin").join("pnpm.cjs")
        );
    }

    #[test]
    fn entry_split_accepts_both_separators() {
        assert_eq!(
            split_entry("bin/pnpm.cjs"),
            PathBuf::from("bin").join("pnpm.cjs")
        );
        assert_eq!(
            split_entry(r"node_modules\@deepseek-ai\dsh\lib\bin.js"),
            PathBuf::from("node_modules")
                .join("@deepseek-ai")
                .join("dsh")
                .join("lib")
                .join("bin.js")
        );
        assert_eq!(split_entry("node.exe"), PathBuf::from("node.exe"));
    }

    #[test]
    fn default_roots_and_entries_match_the_manifest_spec() {
        assert_eq!(default_managed_root(DEP_PNPM), "dependencies/pnpm");
        assert_eq!(default_managed_root(DEP_DSH), "dependencies/dsh");
        assert_eq!(default_entry(DEP_PNPM), "bin/pnpm.cjs");
        assert_eq!(
            default_entry(DEP_DSH),
            "dsh.asar/node_modules/@deepseek-ai/dsh/lib/bin.js"
        );
    }

    #[test]
    fn shipped_manifest_matches_builtin_defaults() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(manifest::MANIFEST_FILE);
        let manifest = manifest::read_at(&path).expect("manifest should parse");
        let base = PathBuf::from("C:/app-data");
        let resources = PathBuf::from("C:/app-resources");
        for (key, root, entry) in [
            (
                DEP_PNPM,
                default_managed_root(DEP_PNPM),
                default_entry(DEP_PNPM),
            ),
            (
                DEP_DSH,
                default_managed_root(DEP_DSH),
                default_entry(DEP_DSH),
            ),
        ] {
            let spec = manifest.dependencies.get(key).expect("spec should exist");
            let declared = spec.managed_root.as_deref().expect("managedRoot");
            assert_eq!(
                manifest::resolve_location_from(&base, Some(&resources), declared),
                base.join(root),
                "{key}"
            );
            assert_eq!(spec.entry.resolve(), Some(entry), "{key}");
        }
    }
}
