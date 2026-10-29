
use serde::Deserialize;
use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use tauri::{Manager, Runtime};
use crate::{AppHandle};

pub const MANIFEST_FILE: &str = "manifest.jsonc";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub engines: Engines,
    pub plugins: Plugins,
    pub pets: Pets,
    pub dependencies: BTreeMap<String, DependencySpec>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Engines {
    pub dsh: DshEngine,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DshEngine {
    pub recommend: String,
    pub minimum: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, Deserialize)]
#[serde(default)]
pub struct Plugins {
    #[serde(rename = "depercated")]
    pub deprecated: Vec<String>,
    #[serde(rename = "built-in")]
    pub built_in: Vec<PluginEntry>,
    pub preset: Vec<PluginEntry>,
}

#[derive(Debug, Clone, Default, serde::Serialize, Deserialize)]
#[serde(default)]
pub struct PluginEntry {
    pub id: String,
    pub spec: String,
    pub name: String,
    pub description: String,
    pub repo: String,
    pub package: Option<String>,
    pub recommended: bool,
    pub fix: bool,
    pub checked: bool,
    #[serde(rename = "winOnly")]
    pub win_only: bool,
    #[serde(default, deserialize_with = "deserialize_version")]
    pub version: Option<PluginVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(untagged)]
pub enum PluginVersion {
    Declared(String),
    Matrix(Vec<VersionPair>),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct VersionPair {
    pub version: String,
    pub dsh: String,
}

fn matches_including_prerelease(req: &semver::VersionReq, version: &semver::Version) -> bool {
    let matched = req.matches(version);
    if matched || version.pre.is_empty() {
        return matched;
    }
    let mut bridged = req.clone();
    bridged.comparators.push(semver::Comparator {
        op: semver::Op::Exact,
        major: version.major,
        minor: Some(version.minor),
        patch: Some(version.patch),
        pre: version.pre.clone(),
    });
    bridged.matches(version)
}

fn req_matches(req: &str, version: &semver::Version) -> bool {
    parse_req(req).is_some_and(|req| matches_including_prerelease(&req, version))
}

fn parse_req(raw: &str) -> Option<semver::VersionReq> {
    let mut parts: Vec<String> = Vec::new();
    let mut pending: Option<String> = None;
    for token in raw.split_whitespace() {
        let token = token.trim_matches(',');
        if token.is_empty() {
            continue;
        }
        if matches!(token, ">" | ">=" | "<" | "<=" | "=" | "^" | "~") {
            pending = Some(token.to_string());
            continue;
        }
        match pending.take() {
            Some(op) => parts.push(format!("{op}{token}")),
            None => parts.push(token.to_string()),
        }
    }
    if pending.is_some() || parts.is_empty() {
        return None;
    }
    semver::VersionReq::parse(&parts.join(",")).ok()
}

impl PluginVersion {
    pub fn plugin_req_for_core(&self, core: Option<&str>) -> Option<&str> {
        let PluginVersion::Matrix(pairs) = self else {
            return None;
        };
        let core = semver::Version::parse(core?).ok()?;
        pairs
            .iter()
            .rev()
            .find(|pair| req_matches(&pair.dsh, &core))
            .map(|pair| pair.version.as_str())
    }

    pub fn unsupported_on(&self, core: Option<&str>) -> bool {
        let PluginVersion::Matrix(pairs) = self else {
            return false;
        };
        let Some(Ok(core)) = core.map(semver::Version::parse) else {
            return false;
        };
        let mut declared = false;
        for pair in pairs {
            let Some(req) = parse_req(&pair.dsh) else {
                return false;
            };
            declared = true;
            if matches_including_prerelease(&req, &core) {
                return false;
            }
        }
        declared
    }
}

fn deserialize_version<'de, D>(deserializer: D) -> Result<Option<PluginVersion>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Declared(String),
        Matrix(Vec<VersionPair>),
    }
    Ok(match Option::<Raw>::deserialize(deserializer)? {
        None => None,
        Some(Raw::Declared(value)) => Some(PluginVersion::Declared(value)),
        Some(Raw::Matrix(pairs)) => Some(PluginVersion::Matrix(pairs)),
    })
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Pets {
    #[serde(rename = "built-in")]
    pub built_in: Vec<PresetPetSpec>,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresetPetSpec {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    pub config: String,
    pub uri: PresetPetUri,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<PresetPetExt>,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresetPetUri {
    pub default: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresetPetExt {
    pub default: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DependencySpec {
    pub entry: EntrySpec,
    pub managed_root: Option<String>,
    pub overridable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EntrySpec {
    Single(String),
    Platform(BTreeMap<String, String>),
    #[default]
    Empty,
}

impl<'de> Deserialize<'de> for EntrySpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Single(String),
            Platform(BTreeMap<String, String>),
        }
        Ok(match Option::<Raw>::deserialize(deserializer)? {
            None => EntrySpec::Empty,
            Some(Raw::Single(value)) => EntrySpec::Single(value),
            Some(Raw::Platform(map)) => EntrySpec::Platform(map),
        })
    }
}

impl EntrySpec {
    pub fn resolve(&self) -> Option<&str> {
        match self {
            EntrySpec::Single(value) => (!value.is_empty()).then_some(value.as_str()),
            EntrySpec::Platform(map) => map
                .get(env::consts::OS)
                .or_else(|| map.get("default"))
                .map(String::as_str),
            EntrySpec::Empty => None,
        }
    }
}

pub fn resource_root<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path()
        .resource_dir()
        .ok()
        .map(|dir| resource_root_from(&dir))
}

pub(crate) fn resource_root_from(resource_dir: &Path) -> PathBuf {
    let root = pick_resource_root(resource_dir).unwrap_or_else(|| resource_dir.to_path_buf());
    dunce::simplified(&root).to_path_buf()
}

fn pick_resource_root(dir: &Path) -> Option<PathBuf> {
    if dir.join(MANIFEST_FILE).is_file() {
        return Some(dir.to_path_buf());
    }
    let nested = dir.join("resources");
    nested.join(MANIFEST_FILE).is_file().then_some(nested)
}

fn source_manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join(MANIFEST_FILE)
}

pub fn manifest_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    manifest_path_from(resource_root(app).as_deref(), cfg!(debug_assertions))
}

pub(crate) fn manifest_path_from(
    resource_root: Option<&Path>,
    source_first: bool,
) -> Option<PathBuf> {
    let packaged = resource_root.map(|root| root.join(MANIFEST_FILE));
    let source = Some(source_manifest_path());
    let candidates = if source_first {
        [source, packaged]
    } else {
        [packaged, source]
    };
    candidates.into_iter().flatten().find(|path| path.is_file())
}

static CACHE: std::sync::Mutex<Option<(PathBuf, u64, Manifest)>> = std::sync::Mutex::new(None);

pub fn with_manifest<R: Runtime, T>(
    app: &AppHandle<R>,
    pick: impl FnOnce(&Manifest) -> T,
) -> Option<T> {
    let path = match manifest_path(app) {
        Some(path) => path,
        None => {
            log::warn!("RESOURCE_MANIFEST_MISSING: {MANIFEST_FILE} not found in resource dir or source resources dir");
            return None;
        }
    };
    let stamp = super::file_stamp(&path);
    let mut guard = CACHE.lock().unwrap_or_else(|error| error.into_inner());
    let fresh = matches!(
        guard.as_ref(),
        Some((cached_path, cached_stamp, _)) if cached_path == &path && *cached_stamp == stamp
    );
    if !fresh {
        match read_at(&path) {
            Ok(manifest) => *guard = Some((path.clone(), stamp, manifest)),
            Err(error) => {
                log::error!("RESOURCE_MANIFEST_INVALID: {}: {error}", path.display());
                return None;
            }
        }
    }
    guard.as_ref().map(|(_, _, manifest)| pick(manifest))
}

pub fn read<R: Runtime>(app: &AppHandle<R>) -> Option<Manifest> {
    with_manifest(app, Manifest::clone)
}

pub fn read_at(path: &Path) -> Result<Manifest, String> {
    let raw = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    parse(&raw)
}

pub fn parse(raw: &str) -> Result<Manifest, String> {
    let stripped = strip_jsonc(raw);
    serde_json::from_str(&stripped).map_err(|error| error.to_string())
}

pub fn recommended_dsh_version<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let version = with_manifest(app, |manifest| {
        manifest.engines.dsh.recommend.trim().to_string()
    })?;
    (!version.is_empty() && semver::Version::parse(&version).is_ok()).then_some(version)
}

pub fn minimum_dsh_version<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let version = with_manifest(app, |manifest| {
        manifest.engines.dsh.minimum.trim().to_string()
    })?;
    (!version.is_empty()).then_some(version)
}

pub fn is_above_recommended<R: Runtime>(app: &AppHandle<R>, version: &str) -> bool {
    let Some(recommended) = recommended_dsh_version(app) else {
        return false;
    };
    match (
        semver::Version::parse(version),
        semver::Version::parse(&recommended),
    ) {
        (Ok(actual), Ok(recommended)) => actual > recommended,
        _ => false,
    }
}

pub fn resolve_location_from(base: &Path, resource_root: Option<&Path>, raw: &str) -> PathBuf {
    let value = raw.trim();
    if PathBuf::from(value).is_absolute() {
        return PathBuf::from(value);
    }
    let normalized = value.replace('\\', "/");
    if let Some(rest) = normalized.strip_prefix("./") {
        return resolve_location_from(base, resource_root, rest);
    }
    if let Some(rest) = strip_prefix_ci(&normalized, "$appdata") {
        return join_token(base, rest);
    }
    if let Some(rest) = strip_prefix_ci(&normalized, "$resources") {
        return join_token(resource_root.unwrap_or(base), rest);
    }
    if let Some(rest) = normalized.strip_prefix("resources/") {
        if let Some(root) = resource_root {
            return join_token(root, rest);
        }
    }
    base.join(value)
}

fn strip_prefix_ci<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let head = value.get(..prefix.len())?;
    if !head.eq_ignore_ascii_case(prefix) {
        return None;
    }
    let rest = &value[prefix.len()..];
    if rest.is_empty() {
        return Some(rest);
    }
    rest.strip_prefix('/')
}

fn join_token(root: &Path, rest: &str) -> PathBuf {
    if rest.is_empty() || rest == "." {
        return root.to_path_buf();
    }
    root.join(rest.replace('/', std::path::MAIN_SEPARATOR_STR))
}

pub(crate) fn strip_jsonc(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(next) = chars.get(i + 1) {
                    out.push(*next);
                    i += 2;
                    continue;
                }
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(chars.len());
            continue;
        }
        out.push(c);
        i += 1;
    }
    strip_trailing_commas(&out)
}

fn strip_trailing_commas(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(next) = chars.get(i + 1) {
                    out.push(*next);
                    i += 2;
                    continue;
                }
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c == ',' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if matches!(chars.get(j), Some('}') | Some(']')) {
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped_manifest() -> Manifest {
        parse(&std::fs::read_to_string(source_manifest_path()).expect("manifest should exist"))
            .expect("shipped manifest should be valid")
    }

    #[test]
    fn shipped_manifest_carries_engines_plugins_pets_and_dependencies() {
        let manifest = shipped_manifest();
        assert!(!manifest.plugins.preset.is_empty());
        assert!(!manifest.plugins.built_in.is_empty());
        assert!(!manifest.plugins.deprecated.is_empty());
        assert!(!manifest.pets.built_in.is_empty());
        for key in ["pnpm", "dsh"] {
            assert!(
                manifest.dependencies.contains_key(key),
                "依赖映射规范缺少 {key}"
            );
        }
    }

    #[test]
    fn dependency_entries_resolve_per_platform() {
        let manifest = shipped_manifest();
        let pnpm = &manifest.dependencies["pnpm"];
        assert!(pnpm.managed_root.is_some());
        assert!(pnpm.overridable);
        assert_eq!(pnpm.entry.resolve(), Some("bin/pnpm.cjs"));

        let dsh = &manifest.dependencies["dsh"];
        assert_eq!(
            dsh.entry.resolve(),
            Some("dsh.asar/node_modules/@deepseek-ai/dsh/lib/bin.js")
        );
    }

    #[test]
    fn shipped_manifest_keeps_recommendation_and_minimum_in_sync() {
        let manifest = shipped_manifest();
        assert_eq!(manifest.engines.dsh.recommend, "0.2.1-rc.4");
        assert_eq!(manifest.engines.dsh.minimum, "0.1.7-rc.1");
        let raw = std::fs::read_to_string(source_manifest_path()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&strip_jsonc(&raw)).unwrap();
        assert_eq!(
            value["dependencies"]["dsh"]["engine"]["recommend"],
            "0.2.1-rc.4"
        );
        assert_eq!(
            value["dependencies"]["dsh"]["engine"]["minimum"],
            "0.1.7-rc.1"
        );
        for (id, floor_req) in [
            ("dsh-better-sidebar", "^0.21.1"),
            ("dsh-rewind-plugin", "^0.14.0"),
            ("@xmanrui/dsh-im", "^4.29.0"),
        ] {
            let entry = manifest
                .plugins
                .preset
                .iter()
                .find(|entry| entry.id == id)
                .unwrap();
            let matrix = entry.version.as_ref().unwrap();
            assert_eq!(
                matrix.plugin_req_for_core(Some("0.1.7-rc.1")),
                Some(floor_req),
                "{id}"
            );
            assert_eq!(
                matrix.plugin_req_for_core(Some("0.2.1-rc.1")),
                Some("latest"),
                "{id}"
            );
            for core in ["0.1.5-rc.1", "0.1.6", "0.1.7-alpha.1", "0.1.7-rc.0"] {
                assert_eq!(matrix.plugin_req_for_core(Some(core)), None, "{id} {core}");
                assert!(matrix.unsupported_on(Some(core)), "{id} {core}");
            }
        }
    }

    #[test]
    fn plugin_version_matrix_matches_core_ranges() {
        let matrix = PluginVersion::Matrix(vec![
            VersionPair {
                version: "^0.21.1".to_string(),
                dsh: "^0.1.7-rc.1".to_string(),
            },
            VersionPair {
                version: "^0.22.1".to_string(),
                dsh: "^0.1.7-rc.5".to_string(),
            },
        ]);
        assert_eq!(
            matrix.plugin_req_for_core(Some("0.1.7-rc.1")),
            Some("^0.21.1")
        );
        assert_eq!(
            matrix.plugin_req_for_core(Some("0.1.7-rc.2")),
            Some("^0.21.1")
        );
        assert_eq!(
            matrix.plugin_req_for_core(Some("0.1.7-rc.5")),
            Some("^0.22.1")
        );
        assert_eq!(matrix.plugin_req_for_core(Some("0.1.8")), Some("^0.22.1"));
        assert_eq!(matrix.plugin_req_for_core(Some("0.2.1-rc.1")), None);
        assert!(!matrix.unsupported_on(Some("0.1.7-rc.1")));
        assert!(!matrix.unsupported_on(Some("0.1.8")));
        assert!(!matrix.unsupported_on(None));
        assert!(!matrix.unsupported_on(Some("not-a-version")));
        assert!(matrix.unsupported_on(Some("0.2.1-rc.1")));
    }

    #[test]
    fn prerelease_core_is_judged_by_numeric_range() {
        let matrix = PluginVersion::Matrix(vec![
            VersionPair {
                version: "^0.21.1".to_string(),
                dsh: "^0.1.7-rc.1".to_string(),
            },
            VersionPair {
                version: "^0.24.1".to_string(),
                dsh: "^0.2.0-rc.1".to_string(),
            },
        ]);
        for (core, expected) in [
            ("0.1.7-rc.1", Some("^0.21.1")),
            ("0.1.7-rc.3", Some("^0.21.1")),
            ("0.1.8", Some("^0.21.1")),
            ("0.2.0-rc.1", Some("^0.24.1")),
            ("0.2.0-rc.2", Some("^0.24.1")),
            ("0.2.0", Some("^0.24.1")),
            ("0.2.1-rc.1", Some("^0.24.1")),
            ("0.2.9-beta.3", Some("^0.24.1")),
        ] {
            assert_eq!(
                matrix.plugin_req_for_core(Some(core)),
                expected,
                "core={core}"
            );
            assert!(!matrix.unsupported_on(Some(core)), "core={core}");
        }
        for core in [
            "0.1.6-rc.1",
            "0.1.7-alpha.1",
            "0.1.7-beta.9",
            "0.1.7-rc.0",
            "0.2.0-alpha.1",
            "0.3.0-rc.1",
            "0.3.0",
        ] {
            assert_eq!(matrix.plugin_req_for_core(Some(core)), None, "core={core}");
            assert!(matrix.unsupported_on(Some(core)), "core={core}");
        }
    }

    #[test]
    fn declared_version_string_is_never_unsupported() {
        let declared = PluginVersion::Declared("latest".to_string());
        assert!(!declared.unsupported_on(Some("9.9.9")));
        assert_eq!(declared.plugin_req_for_core(Some("9.9.9")), None);
    }

    #[test]
    fn jsonc_tolerates_comments_and_trailing_commas() {
        let raw = r#"
        {
          // 行注释
          "engines": { "dsh": { "recommend": "1.2.3", "minimum": "1.0.0" } },
          /* 块注释
             跨行 */
          "dependencies": {
            "node": { "managedRoot": "runtime", "entry": { "windows": "node.exe", }, },
          },
        }
        "#;
        let manifest = parse(raw).expect("jsonc should parse");
        assert_eq!(manifest.engines.dsh.recommend, "1.2.3");
        assert_eq!(
            manifest.dependencies["node"].entry,
            EntrySpec::Platform(BTreeMap::from([(
                "windows".to_string(),
                "node.exe".to_string()
            )]))
        );
    }

    #[test]
    fn jsonc_keeps_comment_markers_inside_strings() {
        let raw = r#"{"plugins":{"preset":[{"id":"x","spec":"x","name":"a//b/*c*/","description":"d","repo":""}]}}"#;
        let manifest = parse(raw).expect("string content must survive comment stripping");
        assert_eq!(manifest.plugins.preset[0].name, "a//b/*c*/");
    }

    #[test]
    fn resource_root_probe_prefers_flat_then_nested() {
        let dir = std::env::temp_dir().join(format!(
            "dsh-manifest-probe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        assert_eq!(pick_resource_root(&dir), None);
        assert_eq!(resource_root_from(&dir), dir);

        let nested = dir.join("resources");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join(MANIFEST_FILE), "{}").unwrap();
        assert_eq!(pick_resource_root(&dir), Some(nested.clone()));
        assert_eq!(resource_root_from(&dir), nested);

        std::fs::write(dir.join(MANIFEST_FILE), "{}").unwrap();
        assert_eq!(pick_resource_root(&dir), Some(dir.clone()));
        assert_eq!(resource_root_from(&dir), dir);
        assert_eq!(
            manifest_path_from(Some(&dir), false),
            Some(dir.join(MANIFEST_FILE))
        );
        let source = source_manifest_path();
        assert!(source.is_file());
        assert_eq!(manifest_path_from(Some(&dir), true), Some(source));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resource_root_simplifies_windows_verbatim_prefix() {
        let verbatim = PathBuf::from(r"\\?\C:\app\resources\dsh");
        let simplified = resource_root_from(&verbatim);
        if cfg!(windows) {
            assert_eq!(simplified, PathBuf::from(r"C:\app\resources\dsh"));
            assert!(!simplified.to_string_lossy().starts_with(r"\\?\"));
        } else {
            assert_eq!(simplified, PathBuf::from(r"\\?\C:\app\resources\dsh"));
        }
    }

    #[test]
    fn location_token_resolves_arbitrary_bases() {
        let base = PathBuf::from("C:/app/data");
        let resources = PathBuf::from("C:/app/resources");
        let absolute = if cfg!(windows) {
            "D:/anywhere"
        } else {
            "/anywhere"
        };

        assert_eq!(
            resolve_location_from(&base, Some(&resources), "dependencies/dsh"),
            base.join("dependencies/dsh")
        );
        assert_eq!(
            resolve_location_from(&base, Some(&resources), "resources/dsh"),
            resources.join("dsh")
        );
        assert_eq!(
            resolve_location_from(&base, Some(&resources), r"resources\node"),
            resources.join("node")
        );
        assert_eq!(
            resolve_location_from(&base, Some(&resources), absolute),
            PathBuf::from(absolute)
        );
    }

    #[test]
    fn location_prefix_tokens_are_case_insensitive_and_typed() {
        let base = PathBuf::from("C:/app/data");
        let resources = PathBuf::from("C:/app/resources");

        for raw in [
            "$AppData/runtime",
            "$appdata/runtime",
            "$APPDATA\\runtime",
            "$AppData",
        ] {
            let expected = if raw.ends_with("runtime") {
                base.join("runtime")
            } else {
                base.clone()
            };
            assert_eq!(
                resolve_location_from(&base, Some(&resources), raw),
                expected,
                "{raw}"
            );
        }

        for raw in ["$Resources/dsh", "$resources/dsh", r"$RESOURCES\dsh"] {
            assert_eq!(
                resolve_location_from(&base, Some(&resources), raw),
                resources.join("dsh"),
                "{raw}"
            );
        }

        assert_eq!(
            resolve_location_from(&base, None, "$Resources/dsh"),
            base.join("dsh")
        );
        assert_eq!(
            resolve_location_from(&base, Some(&resources), "$resourcesfoo/dsh"),
            base.join("$resourcesfoo/dsh")
        );
    }
}
