
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

use super::is_actionable_plugin_ref;
use super::is_package_name;
use super::profile_dir;
use crate::{AppHandle};

#[derive(Default)]
struct PluginMeta {
    deps: Vec<String>,
    optional_deps: Vec<String>,
    patch_path: Option<String>,
}

fn read_plugin_meta(dir: &Path) -> Option<PluginMeta> {
    let content = fs::read_to_string(dir.join("package.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    let mut meta = PluginMeta::default();
    if let Some(deps) = v.get("dependencies").and_then(|d| d.as_object()) {
        meta.deps = deps.keys().cloned().collect();
    }
    if let Some(deps) = v.get("optionalDependencies").and_then(|d| d.as_object()) {
        meta.optional_deps = deps.keys().cloned().collect();
    }
    meta.patch_path = v
        .get("dsh")
        .and_then(|d| d.get("bundle"))
        .and_then(|b| b.get("patch"))
        .and_then(|p| p.as_str())
        .map(String::from);
    Some(meta)
}

fn configured_root_bundles(manifest: &serde_json::Value) -> Vec<String> {
    manifest
        .get("dsh")
        .and_then(|d| d.get("profile"))
        .and_then(|p| p.get("bundles"))
        .and_then(|b| b.as_array())
        .map(|bundles| {
            bundles
                .iter()
                .filter_map(|bundle| bundle.as_str())
                .filter(|bundle| is_actionable_plugin_ref(bundle))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn configured_roots(app_handle: &AppHandle) -> Vec<String> {
    let dir = profile_dir(app_handle);
    let content = match fs::read_to_string(dir.join("package.json")) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let v: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    configured_root_bundles(&v)
}

fn bundle_owns_package(profile: &Path, bundle: &str, package: &str) -> bool {
    let dir = profile.join("node_modules").join(bundle);
    let Some(meta) = read_plugin_meta(&dir) else {
        return false;
    };
    if meta.deps.iter().any(|d| d == package) || meta.optional_deps.iter().any(|d| d == package) {
        return true;
    }
    if let Some(patch) = &meta.patch_path {
        if let Ok(content) = fs::read_to_string(dir.join(patch)) {
            return content.contains(package);
        }
    }
    false
}

fn plugin_references_packages(profile: &Path, plugin: &str, packages: &HashSet<String>) -> bool {
    let dir = profile.join("node_modules").join(plugin);
    if let Some(meta) = read_plugin_meta(&dir) {
        if meta
            .deps
            .iter()
            .chain(meta.optional_deps.iter())
            .any(|d| packages.contains(d))
        {
            return true;
        }
    }
    for file in [
        "cordis.patch.yml",
        "index.js",
        "lib/index.js",
        "dist/index.js",
    ] {
        if let Ok(content) = fs::read_to_string(dir.join(file)) {
            if packages.iter().any(|p| content.contains(p)) {
                return true;
            }
        }
    }
    false
}

fn plugin_declares_loader_entry(profile: &Path, plugin: &str, entry_id: &str) -> bool {
    let dir = profile.join("node_modules").join(plugin);
    let Some(meta) = read_plugin_meta(&dir) else {
        return false;
    };
    let Some(patch) = &meta.patch_path else {
        return false;
    };
    let Ok(content) = fs::read_to_string(dir.join(patch)) else {
        return false;
    };
    let re = Regex::new(&format!(
        r#"^\s*-\s+id:\s*["']?{}["']?(?:\s*(?:#.*)?)?$"#,
        regex::escape(entry_id)
    ))
    .ok();
    re.map(|re| re.is_match(&content)).unwrap_or(false)
}

fn plugin_matches_slot(profile: &Path, plugin: &str, slot: &str) -> bool {
    let dir = profile.join("node_modules").join(plugin);
    if let Ok(content) = fs::read_to_string(dir.join("package.json")) {
        if let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(dsh) = manifest.get("dsh") {
                if value_contains_slot(dsh, slot) {
                    return true;
                }
            }
        }
    }
    for file in [
        "cordis.patch.yml",
        "client.js",
        "lib/client.js",
        "dist/client.js",
        "index.js",
        "lib/index.js",
        "dist/index.js",
    ] {
        if let Ok(content) = fs::read_to_string(dir.join(file)) {
            if content.contains(slot) {
                return true;
            }
        }
    }
    false
}

fn value_contains_slot(value: &serde_json::Value, slot: &str) -> bool {
    match value {
        serde_json::Value::String(s) => s.contains(slot),
        serde_json::Value::Array(items) => items.iter().any(|v| value_contains_slot(v, slot)),
        serde_json::Value::Object(map) => map.values().any(|v| value_contains_slot(v, slot)),
        _ => false,
    }
}

fn packages_providing_slot(profile: &Path, slot: &str) -> Vec<String> {
    let scope = profile.join("node_modules").join("@deepseek-ai");
    let Ok(entries) = fs::read_dir(&scope) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("dsh-client-ui-") {
            continue;
        }
        let package = format!("@deepseek-ai/{name}");
        let dir = entry.path();
        for file in ["client.js", "lib/client.js", "dist/client.js"] {
            if let Ok(content) = fs::read_to_string(dir.join(file)) {
                if content.contains(slot) {
                    out.push(package);
                    break;
                }
            }
        }
    }
    out
}

pub(super) fn resolve_recovery_plugins(
    app_handle: &AppHandle,
    detected_refs: &[String],
    duplicate_entry: Option<&str>,
    slot_conflict: Option<&str>,
) -> Vec<String> {
    let profile = profile_dir(app_handle);
    let roots = configured_roots(app_handle);
    if roots.is_empty() {
        return Vec::new();
    }
    let roots_set: HashSet<&String> = roots.iter().collect();

    let mut matched = HashSet::new();
    for detected in detected_refs {
        if !is_package_name(detected) {
            continue;
        }
        if roots_set.contains(detected) {
            matched.insert(detected.clone());
            continue;
        }
        let owners: Vec<&String> = roots
            .iter()
            .filter(|root| bundle_owns_package(&profile, root, detected))
            .collect();
        if owners.len() == 1 {
            matched.insert(owners[0].clone());
        }
    }
    if matched.len() == 1 {
        return matched.into_iter().collect();
    }

    let mut dynamic_owners = HashSet::new();
    for detected in detected_refs {
        if !is_package_name(detected) || roots_set.contains(detected) {
            continue;
        }
        let packages: HashSet<String> = [detected.to_string()].into();
        let owners: Vec<&String> = roots
            .iter()
            .filter(|root| plugin_references_packages(&profile, root, &packages))
            .collect();
        if owners.len() == 1 {
            dynamic_owners.insert(owners[0].clone());
        }
    }
    if dynamic_owners.len() == 1 {
        return dynamic_owners.into_iter().collect();
    }

    if let Some(entry) = duplicate_entry {
        let owners: Vec<&String> = roots
            .iter()
            .filter(|root| plugin_declares_loader_entry(&profile, root, entry))
            .collect();
        if owners.len() == 1 {
            return owners.into_iter().cloned().collect();
        }
    }

    if let Some(slot) = slot_conflict {
        let matched: Vec<&String> = roots
            .iter()
            .filter(|root| plugin_matches_slot(&profile, root, slot))
            .collect();
        if matched.len() == 1 {
            return matched.into_iter().cloned().collect();
        }
        let providers: HashSet<String> = packages_providing_slot(&profile, slot)
            .into_iter()
            .collect();
        if !providers.is_empty() {
            let owners: Vec<&String> = roots
                .iter()
                .filter(|root| plugin_references_packages(&profile, root, &providers))
                .collect();
            if owners.len() == 1 {
                return owners.into_iter().cloned().collect();
            }
        }
    }

    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_roots_include_bundle_left_after_dependency_removal() {
        let manifest = serde_json::json!({
            "name": "dsh-profile-web",
            "private": true,
            "dependencies": {},
            "dsh": {
                "profile": {
                    "bundles": [
                        "@deepseek-ai/dsh-base",
                        "@deepseek-ai/dsh-web-app",
                        "@linxin666/dsh-web-ui-all"
                    ]
                }
            }
        });

        assert_eq!(
            configured_root_bundles(&manifest),
            vec!["@linxin666/dsh-web-ui-all"]
        );
    }
}
