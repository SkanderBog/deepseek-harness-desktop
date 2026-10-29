
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use tauri::{Manager};

use super::installed::{profile_dir, ProfilePackageJson};
use super::patch_guard::now_stamp;
use crate::config;
use crate::{AppHandle};

pub(crate) const HMR_PATCH_FILENAME: &str = "cordis.hmr.patch.yml";

const HMR_ENTRY_ID: &str = "hmr";
const HMR_ENTRY_NAME: &str = "@deepseek-ai/dsh-hmr";

const HMR_DEBOUNCE_MS: u64 = 100;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalHmrStatus {
    pub enabled: bool,
    pub watching: bool,
    pub patch_path: Option<String>,
    pub roots: Vec<String>,
}

pub(crate) fn patch_path(app_handle: &AppHandle) -> PathBuf {
    config::get_base_dir(app_handle).join(HMR_PATCH_FILENAME)
}

pub(crate) fn sync_layer(app_handle: &AppHandle) -> Option<PathBuf> {
    sync_layer_at(
        &config::get_base_dir(app_handle),
        &profile_dir(app_handle),
        app_handle.path().resource_dir().ok().as_deref(),
        hmr_enabled(app_handle),
    )
}

pub(crate) fn sync_layer_at(
    base: &Path,
    profile: &Path,
    resource_root: Option<&Path>,
    enabled: bool,
) -> Option<PathBuf> {
    let path = base.join(HMR_PATCH_FILENAME);
    let roots = local_watch_roots_at(profile, resource_root);
    let rendered = if enabled { render_layer(&roots) } else { None };
    let Some(rendered) = rendered else {
        remove_layer(&path);
        return None;
    };
    if std::fs::read_to_string(&path).is_ok_and(|current| current == rendered) {
        return Some(path);
    }
    match write_layer(&path, &rendered) {
        Ok(()) => {
            log::info!("HMR layer written: {}", path.display());
            Some(path)
        }
        Err(error) => {
            log::warn!("HMR_LAYER_WRITE_FAILED: {error}");
            None
        }
    }
}

pub(crate) fn status(app_handle: &AppHandle) -> LocalHmrStatus {
    let path = patch_path(app_handle);
    let roots = local_watch_roots(app_handle);
    let enabled = hmr_enabled(app_handle);
    LocalHmrStatus {
        enabled,
        watching: enabled && !roots.is_empty(),
        patch_path: path.exists().then(|| path.display().to_string()),
        roots: roots.iter().map(|dir| dir.display().to_string()).collect(),
    }
}

fn hmr_enabled(app_handle: &AppHandle) -> bool {
    config::read_store_dat_setting(app_handle).local_plugin_hmr
}

pub(crate) fn local_watch_roots(app_handle: &AppHandle) -> Vec<PathBuf> {
    local_watch_roots_at(
        &profile_dir(app_handle),
        app_handle.path().resource_dir().ok().as_deref(),
    )
}

pub(crate) fn local_watch_roots_at(profile: &Path, resource_root: Option<&Path>) -> Vec<PathBuf> {
    let Some(manifest) = read_manifest(profile) else {
        return Vec::new();
    };
    watch_roots_in(profile, &manifest, |_name, dir| {
        resource_root.is_some_and(|root| is_bundled(root, dir))
    })
}

fn watch_roots_in(
    profile: &Path,
    manifest: &ProfilePackageJson,
    is_excluded: impl Fn(&str, &Path) -> bool,
) -> Vec<PathBuf> {
    let bundles: BTreeSet<&str> = manifest
        .dsh
        .as_ref()
        .and_then(|dsh| dsh.profile.as_ref())
        .map(|inner| inner.bundles.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let mut roots = BTreeSet::new();
    for (name, spec) in &manifest.dependencies {
        if !bundles.contains(name.as_str()) {
            continue;
        }
        let Some(raw) = local_dep_path(spec) else {
            continue;
        };
        let Some(dir) = resolve_dir(&raw, profile) else {
            continue;
        };
        if is_excluded(name, &dir) {
            continue;
        }
        roots.insert(dir);
    }
    prune_nested(roots.into_iter().collect())
}

fn read_manifest(profile: &Path) -> Option<ProfilePackageJson> {
    let content = std::fs::read_to_string(profile.join("package.json")).ok()?;
    serde_json::from_str(&content).ok()
}

fn local_dep_path(spec: &str) -> Option<String> {
    let raw = spec
        .strip_prefix("link:")
        .or_else(|| spec.strip_prefix("file:"))?;
    let trimmed = raw.trim().trim_end_matches(['/', '\\']);
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn resolve_dir(raw: &str, profile: &Path) -> Option<PathBuf> {
    let normalized = raw.replace('/', std::path::MAIN_SEPARATOR_STR);
    let path = Path::new(&normalized);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        profile.join(path)
    };
    let resolved = dunce::canonicalize(&joined).ok()?;
    resolved.is_dir().then_some(resolved)
}

fn is_bundled(resource_root: &Path, dir: &Path) -> bool {
    let flatten = |path: &Path| {
        let mut text = path
            .to_string_lossy()
            .replace('/', std::path::MAIN_SEPARATOR_STR);
        if !text.ends_with(std::path::MAIN_SEPARATOR) {
            text.push_str(std::path::MAIN_SEPARATOR_STR);
        }
        text.to_lowercase()
    };
    flatten(dir).starts_with(&flatten(dunce::simplified(resource_root)))
}

fn prune_nested(dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut kept: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        if kept.iter().any(|parent| dir.starts_with(parent)) {
            continue;
        }
        kept.retain(|child| !child.starts_with(&dir));
        kept.push(dir);
    }
    kept
}

fn pick_base(roots: &[PathBuf]) -> Option<PathBuf> {
    let mut groups: Vec<(std::ffi::OsString, Vec<PathBuf>)> = Vec::new();
    for root in roots {
        let key = root
            .components()
            .next()
            .map(|part| part.as_os_str().to_os_string())
            .unwrap_or_default();
        match groups.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, members)) => members.push(root.clone()),
            None => groups.push((key, vec![root.clone()])),
        }
    }
    let mut best: Option<Vec<PathBuf>> = None;
    for (_, members) in groups {
        if best
            .as_ref()
            .is_none_or(|current| members.len() > current.len())
        {
            best = Some(members);
        }
    }
    common_ancestor(&best?)
}

fn common_ancestor(paths: &[PathBuf]) -> Option<PathBuf> {
    let first = paths.first()?;
    let mut shared: Vec<Component> = first.components().collect();
    for path in &paths[1..] {
        let mut index = 0;
        for (left, right) in shared.iter().zip(path.components()) {
            if *left != right {
                break;
            }
            index += 1;
        }
        shared.truncate(index);
    }
    let ancestor: PathBuf = shared.into_iter().collect();
    ancestor.is_absolute().then_some(ancestor)
}

fn relative_root(base: &Path, root: &Path) -> String {
    let text = root
        .strip_prefix(base)
        .unwrap_or(root)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    let trimmed = text.trim_matches('/');
    if trimmed.is_empty() {
        ".".to_string()
    } else {
        trimmed.to_string()
    }
}

fn file_url(dir: &Path) -> Option<String> {
    if dir
        .components()
        .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return None;
    }
    tauri::Url::from_file_path(dir)
        .ok()
        .map(|url| url.as_str().to_string())
}

fn render_layer(roots: &[PathBuf]) -> Option<String> {
    let base = pick_base(roots)?;
    let base_url = file_url(&base)?;
    let mut yaml = String::from("- id: ");
    yaml.push_str(HMR_ENTRY_ID);
    yaml.push_str(
        "
  name: '",
    );
    yaml.push_str(HMR_ENTRY_NAME);
    yaml.push_str(
        "'
  config:
    base: ",
    );
    yaml.push_str(&base_url);
    yaml.push_str(
        "
    root:",
    );
    for root in roots {
        yaml.push_str(
            "
      - ",
        );
        yaml.push_str(&root_value(&base, root));
    }
    yaml.push_str(
        "
    ignored:",
    );
    for pattern in ignore_patterns() {
        yaml.push_str(
            "
      - '",
        );
        yaml.push_str(&pattern);
        yaml.push('\'');
    }
    yaml.push_str(
        "
    debounce: ",
    );
    yaml.push_str(&HMR_DEBOUNCE_MS.to_string());
    yaml.push('\n');
    Some(yaml)
}

fn root_value(base: &Path, root: &Path) -> String {
    if root.starts_with(base) {
        relative_root(base, root)
    } else {
        root.to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/")
    }
}

fn ignore_patterns() -> Vec<String> {
    vec![
        "**/node_modules".to_string(),
        "**/node_modules/**".to_string(),
        "**/.*".to_string(),
        "**/.*/**".to_string(),
        "cache".to_string(),
        "data".to_string(),
    ]
}

fn write_layer(path: &Path, rendered: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("HMR_LAYER_MKDIR_FAILED: {}: {e}", parent.display()))?;
    }
    let temp = path.with_file_name(format!("{HMR_PATCH_FILENAME}.tmp-{}", now_stamp()));
    std::fs::write(&temp, rendered).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("HMR_LAYER_WRITE_FAILED: {}: {e}", temp.display())
    })?;
    std::fs::rename(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("HMR_LAYER_WRITE_FAILED: {}: {e}", path.display())
    })
}

fn remove_layer(path: &Path) {
    if !path.exists() {
        return;
    }
    match std::fs::remove_file(path) {
        Ok(()) => log::info!("HMR layer removed: {}", path.display()),
        Err(error) => log::warn!("HMR_LAYER_REMOVE_FAILED: {}: {error}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dsh-hmr-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dunce::canonicalize(&dir).unwrap()
    }

    fn manifest(dependencies: &[(&str, String)], bundles: &[&str]) -> ProfilePackageJson {
        let deps: HashMap<String, String> = dependencies
            .iter()
            .map(|(name, spec)| (name.to_string(), spec.clone()))
            .collect();
        serde_json::from_value(serde_json::json!({
            "name": "dsh-profile-test",
            "private": true,
            "dependencies": deps,
            "dsh": { "profile": { "bundles": bundles } },
        }))
        .unwrap()
    }

    fn plugin_dir(profile: &Path, name: &str) -> PathBuf {
        let dir = profile.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dunce::canonicalize(&dir).unwrap()
    }

    fn dump_layer(name: &str, yaml: &str) {
        let Some(dir) = std::env::var_os("DSH_HMR_LAYER_DUMP").filter(|dir| !dir.is_empty()) else {
            return;
        };
        std::fs::write(Path::new(&dir).join(name), yaml).unwrap();
    }

    #[test]
    fn ignore_patterns_keep_globstar_semantics() {
        let patterns = ignore_patterns();
        assert_eq!(
            patterns,
            vec![
                "**/node_modules".to_string(),
                "**/node_modules/**".to_string(),
                "**/.*".to_string(),
                "**/.*/**".to_string(),
                "cache".to_string(),
                "data".to_string(),
            ]
        );
        for pattern in &patterns {
            assert!(!pattern.contains('\\'), "{pattern}");
            assert!(!pattern.contains('['), "{pattern}");
        }
    }

    #[test]
    fn sync_layer_at_rereads_the_profile_and_excludes_only_the_explicit_resource_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        let profile = plugin_dir(&root, "profile");
        let resources = plugin_dir(&root, "resources");
        let bundled = plugin_dir(&resources, "builtin");
        let local = plugin_dir(&root, "resources-local");
        let base = root.join("app-data");
        let path = base.join(HMR_PATCH_FILENAME);
        let manifest_path = profile.join("package.json");
        std::fs::write(
            &manifest_path,
            serde_json::json!({
                "dependencies": {
                    "local": format!("link:{}", local.display()),
                    "bundled": format!("file:{}", bundled.display())
                },
                "dsh": { "profile": { "bundles": ["local", "bundled"] } }
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            local_watch_roots_at(&profile, Some(&resources)),
            vec![local.clone()]
        );
        assert_eq!(
            sync_layer_at(&base, &profile, Some(&resources), true),
            Some(path.clone())
        );
        let layer = std::fs::read_to_string(&path).unwrap();
        let entries: Vec<serde_yaml::Value> = serde_yaml::from_str(&layer).unwrap();
        let url = tauri::Url::parse(entries[0]["config"]["base"].as_str().unwrap()).unwrap();
        assert_eq!(url.to_file_path().unwrap(), local);
        assert_eq!(entries[0]["config"]["root"].as_sequence().unwrap().len(), 1);
        assert_eq!(entries[0]["config"]["root"][0].as_str(), Some("."));

        assert_eq!(
            sync_layer_at(&base, &profile, Some(&resources), false),
            None
        );
        assert!(!path.exists());
        assert_eq!(
            sync_layer_at(&base, &profile, Some(&resources), true),
            Some(path.clone())
        );
        std::fs::write(
            &manifest_path,
            r#"{"dependencies":{},"dsh":{"profile":{"bundles":[]}}}"#,
        )
        .unwrap();
        assert_eq!(sync_layer_at(&base, &profile, Some(&resources), true), None);
        assert!(!path.exists());
    }

    #[test]
    fn local_dep_path_accepts_both_prefixes_and_trims_separators() {
        assert_eq!(
            local_dep_path("link:D:/plugins/mine/"),
            Some("D:/plugins/mine".to_string())
        );
        assert_eq!(
            local_dep_path("file:D:\\plugins\\mine"),
            Some("D:\\plugins\\mine".to_string())
        );
        assert_eq!(local_dep_path("^1.2.3"), None);
        assert_eq!(local_dep_path("github:user/repo"), None);
        assert_eq!(local_dep_path("link:"), None);
    }

    #[test]
    fn resolve_dir_needs_a_real_directory() {
        let profile = tmp_dir("resolve");
        let dir = plugin_dir(&profile, "mine");
        assert_eq!(resolve_dir("mine", &profile), Some(dir.clone()));
        assert_eq!(resolve_dir("./mine", &profile), Some(dir));
        assert_eq!(resolve_dir("missing", &profile), None);
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn watch_roots_only_cover_mounted_local_plugins() {
        let profile = tmp_dir("watch-roots");
        let local = plugin_dir(&profile, "local");
        let sibling = plugin_dir(&profile, "plain");
        let excluded = plugin_dir(&profile, "excluded");
        let unbundled = plugin_dir(&profile, "unbundled");
        let manifest = manifest(
            &[
                ("local", format!("link:{}", local.display())),
                ("sibling", "link:plain".to_string()),
                ("unbundled", format!("link:{}", unbundled.display())),
                ("excluded", format!("link:{}", excluded.display())),
                ("registry", "^1.0.0".to_string()),
                ("ghost", "link:no-such-dir".to_string()),
            ],
            &[
                "local",
                "sibling",
                "unbundled",
                "excluded",
                "registry",
                "ghost",
            ],
        );

        let roots = watch_roots_in(&profile, &manifest, |name, _| name == "excluded");
        assert_eq!(roots, vec![local, sibling, unbundled]);
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[test]
    fn prune_nested_keeps_the_topmost_directory() {
        let root = PathBuf::from("X:\\hmr-tmp").join("prune");
        let outside = PathBuf::from("X:\\hmr-tmp").join("outside");
        let roots = prune_nested(vec![
            root.join("a\\b"),
            root.clone(),
            root.join("a\\b\\c"),
            root.join("ab"),
            outside.clone(),
        ]);
        assert_eq!(roots, vec![root.clone(), outside]);

        let sibling = prune_nested(vec![root.join("a\\b"), root.join("a\\bc")]);
        assert_eq!(sibling, vec![root.join("a\\b"), root.join("a\\bc")]);
    }

    #[test]
    fn common_ancestor_stops_at_the_shared_prefix() {
        let root = tmp_dir("common");
        assert_eq!(
            common_ancestor(&[root.join("a").join("b"), root.join("a").join("c")]),
            Some(root.join("a"))
        );
        assert_eq!(
            common_ancestor(std::slice::from_ref(&root)),
            Some(root.clone())
        );
        assert_eq!(common_ancestor(&[]), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn pick_base_prefers_the_volume_with_more_roots() {
        let root = tmp_dir("pick-base");
        let single = vec![root.join("only")];
        assert_eq!(pick_base(&single), Some(single[0].clone()));

        let roots = vec![root.join("a").join("p1"), root.join("a").join("p2")];
        assert_eq!(pick_base(&roots), Some(root.join("a")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn render_layer_writes_a_complete_hmr_config() {
        let root = tmp_dir("render").join("plugins");
        std::fs::create_dir_all(&root).unwrap();
        let roots = vec![root.join("mine"), root.join("other").join("sub")];
        let yaml = render_layer(&roots).expect("render");

        let entries: Vec<serde_yaml::Value> = serde_yaml::from_str(&yaml).expect("parse");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry["id"].as_str(), Some(HMR_ENTRY_ID));
        assert_eq!(entry["name"].as_str(), Some(HMR_ENTRY_NAME));
        let config = &entry["config"];
        let base = config["base"].as_str().expect("base");
        assert!(base.starts_with("file:///"), "{base}");
        assert_eq!(config["root"].as_sequence().unwrap().len(), 2, "{yaml}");
        assert_eq!(config["root"][0].as_str(), Some("mine"));
        assert_eq!(config["root"][1].as_str(), Some("other/sub"));
        assert_eq!(config["ignored"].as_sequence().unwrap().len(), 6);
        assert_eq!(config["debounce"].as_u64(), Some(HMR_DEBOUNCE_MS));

        let base_line = format!("    base: {}", file_url(&root).expect("file url"));
        let golden = [
            "- id: hmr",
            "  name: '@deepseek-ai/dsh-hmr'",
            "  config:",
            base_line.as_str(),
            "    root:",
            "      - mine",
            "      - other/sub",
            "    ignored:",
            "      - '**/node_modules'",
            "      - '**/node_modules/**'",
            "      - '**/.*'",
            "      - '**/.*/**'",
            "      - 'cache'",
            "      - 'data'",
            "    debounce: 100",
        ]
        .join("\n")
            + "\n";
        assert_eq!(yaml, golden);
    }

    fn reserved_dir(tag: &str) -> PathBuf {
        let dir = tmp_dir(tag).join("my#plugin");
        std::fs::create_dir_all(&dir).unwrap();
        dunce::canonicalize(&dir).unwrap()
    }

    #[test]
    fn render_layer_percent_encodes_reserved_characters() {
        let source = reserved_dir("reserved");
        let yaml = render_layer(std::slice::from_ref(&source)).expect("render");
        dump_layer("dsh-hmr-layer-hash-out.yml", &yaml);
        let entries: Vec<serde_yaml::Value> = serde_yaml::from_str(&yaml).expect("parse");
        let config = &entries[0]["config"];
        let base = config["base"].as_str().expect("base");
        assert!(base.contains("my%23plugin"), "{base}");
        assert!(!base.contains('#'), "{base}");

        let restored = tauri::Url::parse(base).expect("parse url").to_file_path();
        assert_eq!(restored.ok(), Some(source));
    }

    #[test]
    fn render_layer_round_trips_a_real_link_dependency() {
        let home = tmp_dir("roundtrip");
        let profile = home.join("profiles").join("p3");
        std::fs::create_dir_all(&profile).unwrap();
        let profile = dunce::canonicalize(&profile).unwrap();
        let source = plugin_dir(&profile, "mine");
        let spec = super::super::install::local_spec_from_path(&source);
        let manifest = manifest(&[("probe-hmr-plugin", spec)], &["probe-hmr-plugin"]);

        let roots = watch_roots_in(&profile, &manifest, |_name, _dir| false);
        assert_eq!(roots, vec![source.clone()]);

        let yaml = render_layer(&roots).expect("render");
        dump_layer("dsh-hmr-layer-out.yml", &yaml);

        let entries: Vec<serde_yaml::Value> = serde_yaml::from_str(&yaml).expect("parse");
        let config = &entries[0]["config"];
        let raw = config["base"]
            .as_str()
            .expect("base")
            .trim_start_matches("file://");
        #[cfg(windows)]
        let raw = raw.trim_start_matches('/');
        let base = PathBuf::from(raw);
        let resolved = config["root"]
            .as_sequence()
            .expect("root")
            .iter()
            .map(|item| base.join(item.as_str().expect("root item")))
            .collect::<Vec<_>>();
        assert_eq!(resolved, vec![source]);
    }

    #[test]
    fn render_layer_needs_at_least_one_root() {
        assert_eq!(render_layer(&[]), None);
    }

    #[cfg(windows)]
    #[test]
    fn render_layer_keeps_foreign_volumes_absolute() {
        let roots = vec![
            PathBuf::from("D:\\plugins\\mine"),
            PathBuf::from("C:\\other\\plugin"),
        ];
        let yaml = render_layer(&roots).expect("render");
        let entries: Vec<serde_yaml::Value> = serde_yaml::from_str(&yaml).expect("parse");
        let config = &entries[0]["config"];
        assert_eq!(config["base"].as_str(), Some("file:///D:/plugins/mine"));
        assert_eq!(config["root"][0].as_str(), Some("."));
        assert_eq!(config["root"][1].as_str(), Some("C:/other/plugin"));
    }
}
