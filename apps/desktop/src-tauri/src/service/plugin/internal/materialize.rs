//! '<profile>/node_modules/<name>/package.json'`）。issue #264 把它当作瞬时

use std::path::{Path, PathBuf};

use super::manifest::{
    internal_plugin_entry_is_ready, remove_stale_plugin_entry, write_profile_manifest,
};

pub(super) struct OfflineLink {
    pub id: String,
    pub name: String,
    pub spec: String,
    pub bundled: PathBuf,
    pub entry: PathBuf,
}

pub(super) fn materialize_links(profile: &Path, links: &[OfflineLink]) -> Result<(), String> {
    let node_modules = profile.join("node_modules");
    std::fs::create_dir_all(&node_modules).map_err(|e| {
        format!(
            "INTERNAL_PLUGIN_OFFLINE_MKDIR_FAILED: {}: {e}",
            node_modules.display()
        )
    })?;

    let manifest_path = profile.join("package.json");
    let raw = std::fs::read_to_string(&manifest_path).map_err(|e| {
        format!(
            "INTERNAL_PLUGIN_OFFLINE_MANIFEST_READ_FAILED: {}: {e}",
            manifest_path.display()
        )
    })?;
    let mut manifest: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("INTERNAL_PLUGIN_OFFLINE_MANIFEST_PARSE_FAILED: {e}"))?;

    let mut failures = Vec::new();
    for link in links {
        if let Err(e) = remove_stale_plugin_entry(&link.entry) {
            failures.push(format!("{}: 清理旧入口失败 {e}", link.id));
            continue;
        }
        if let Some(parent) = link.entry.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                failures.push(format!(
                    "{}: 建立入口父目录失败 {}: {e}",
                    link.id,
                    parent.display()
                ));
                continue;
            }
        }
        if let Err(e) = crate::service::core::create_directory_link(&link.bundled, &link.entry) {
            failures.push(format!(
                "{}: 建立链接失败 {} -> {}: {e}",
                link.id,
                link.entry.display(),
                link.bundled.display()
            ));
            continue;
        }
        if !internal_plugin_entry_is_ready(&link.entry) {
            failures.push(format!(
                "{}: 自建链接后仍不可读 {}",
                link.id,
                link.entry.display()
            ));
        }
    }
    if !failures.is_empty() {
        return Err(format!(
            "INTERNAL_PLUGIN_OFFLINE_LINK_FAILED: {}",
            failures.join("; ")
        ));
    }

    upsert_manifest(&mut manifest, links)?;
    write_profile_manifest(&manifest_path, &manifest)?;
    log::warn!(
        "INTERNAL_PLUGIN_OFFLINE_LINK_OK: materialized {} internal plugin links without pnpm",
        links.len()
    );
    Ok(())
}

fn upsert_manifest(manifest: &mut serde_json::Value, links: &[OfflineLink]) -> Result<(), String> {
    let object = manifest.as_object_mut().ok_or_else(|| {
        "INTERNAL_PLUGIN_OFFLINE_MANIFEST_INVALID: profile package.json is not an object"
            .to_string()
    })?;

    let dependencies = object
        .entry("dependencies".to_string())
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| {
            "INTERNAL_PLUGIN_OFFLINE_MANIFEST_INVALID: dependencies is not an object".to_string()
        })?;
    for link in links {
        dependencies.insert(
            link.name.clone(),
            serde_json::Value::String(link.spec.clone()),
        );
    }

    let dsh = object
        .entry("dsh".to_string())
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| {
            "INTERNAL_PLUGIN_OFFLINE_MANIFEST_INVALID: dsh is not an object".to_string()
        })?;
    let profile = dsh
        .entry("profile".to_string())
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| {
            "INTERNAL_PLUGIN_OFFLINE_MANIFEST_INVALID: dsh.profile is not an object".to_string()
        })?;
    let bundles = profile
        .entry("bundles".to_string())
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| {
            "INTERNAL_PLUGIN_OFFLINE_MANIFEST_INVALID: dsh.profile.bundles is not an array"
                .to_string()
        })?;
    for link in links {
        if !bundles
            .iter()
            .any(|bundle| bundle.as_str() == Some(link.name.as_str()))
        {
            bundles.push(serde_json::Value::String(link.name.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::plugin::preset::bundled_dep_spec;

    fn tmp_root(label: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("dsh-offline-link-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn fixture(label: &str, names: &[&str]) -> (PathBuf, PathBuf, Vec<OfflineLink>) {
        let root = tmp_root(label);
        let profile = root.join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(
            profile.join("package.json"),
            r#"{
  "name": "dsh-profile-tauri",
  "private": true,
  "dependencies": { "dshmarket": "github:dsh-market/dshmarket" },
  "dsh": { "profile": { "bundles": ["@deepseek-ai/dsh-base", "dshmarket"] } }
}"#,
        )
        .unwrap();

        let mut links = Vec::new();
        for name in names {
            let bundled = root.join("resources").join("node_modules").join(name);
            std::fs::create_dir_all(&bundled).unwrap();
            std::fs::write(
                bundled.join("package.json"),
                format!(r#"{{"name":"{name}"}}"#),
            )
            .unwrap();
            links.push(OfflineLink {
                id: (*name).to_string(),
                name: (*name).to_string(),
                spec: bundled_dep_spec(&bundled),
                bundled,
                entry: profile.join("node_modules").join(name),
            });
        }
        (root, profile, links)
    }

    fn manifest(profile: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(profile.join("package.json")).unwrap())
            .unwrap()
    }

    #[test]
    fn materialize_links_creates_entry_and_manifest_entries() {
        let (root, profile, links) = fixture("create", &["dsh-tauri", "dsh-tauri-ui"]);

        materialize_links(&profile, &links).unwrap();

        for link in &links {
            assert!(
                internal_plugin_entry_is_ready(&link.entry),
                "{} entry must be readable",
                link.id
            );
        }
        let manifest = manifest(&profile);
        assert_eq!(manifest["dependencies"]["dsh-tauri"], links[0].spec);
        assert_eq!(manifest["dependencies"]["dsh-tauri-ui"], links[1].spec);
        assert_eq!(
            manifest["dependencies"]["dshmarket"],
            "github:dsh-market/dshmarket"
        );
        assert_eq!(
            manifest["dsh"]["profile"]["bundles"],
            serde_json::json!([
                "@deepseek-ai/dsh-base",
                "dshmarket",
                "dsh-tauri",
                "dsh-tauri-ui"
            ])
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn materialize_links_replaces_broken_entry_and_is_idempotent() {
        let (root, profile, links) = fixture("replace", &["dsh-tauri"]);
        let entry = links[0].entry.clone();
        std::fs::create_dir_all(&entry).unwrap();
        std::fs::write(entry.join("partial"), "broken").unwrap();

        materialize_links(&profile, &links).unwrap();
        materialize_links(&profile, &links).unwrap();

        assert!(internal_plugin_entry_is_ready(&entry));
        assert!(!entry.join("partial").exists());
        let manifest = manifest(&profile);
        assert_eq!(
            manifest["dsh"]["profile"]["bundles"],
            serde_json::json!(["@deepseek-ai/dsh-base", "dshmarket", "dsh-tauri"])
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn materialize_links_creates_scoped_entry_parent() {
        let scope = "@baihejiangnan/dsh-session-context-menu";
        let (root, profile, links) = fixture("scoped", &[scope]);

        materialize_links(&profile, &links).unwrap();

        assert!(internal_plugin_entry_is_ready(&links[0].entry));
        let manifest = manifest(&profile);
        assert_eq!(manifest["dependencies"][scope], links[0].spec);
        assert_eq!(
            manifest["dsh"]["profile"]["bundles"],
            serde_json::json!([
                "@deepseek-ai/dsh-base",
                "dshmarket",
                "@baihejiangnan/dsh-session-context-menu"
            ])
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn materialize_links_reports_unresolvable_source() {
        let (root, profile, links) = fixture("missing-source", &["dsh-tauri"]);
        std::fs::remove_dir_all(&links[0].bundled).unwrap();

        let error = materialize_links(&profile, &links).expect_err("missing source must fail");

        assert!(
            error.starts_with("INTERNAL_PLUGIN_OFFLINE_LINK_FAILED:"),
            "{error}"
        );
        assert!(error.contains("dsh-tauri"), "{error}");
        let manifest = manifest(&profile);
        assert!(manifest["dependencies"].get("dsh-tauri").is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn upsert_manifest_rejects_non_object_manifest() {
        let links = vec![OfflineLink {
            id: "dsh-tauri".to_string(),
            name: "dsh-tauri".to_string(),
            spec: "link:C:/x".to_string(),
            bundled: PathBuf::from("C:/x"),
            entry: PathBuf::from("C:/p/node_modules/dsh-tauri"),
        }];
        let mut manifest = serde_json::json!([]);

        let error = upsert_manifest(&mut manifest, &links).expect_err("array manifest must fail");

        assert!(
            error.starts_with("INTERNAL_PLUGIN_OFFLINE_MANIFEST_INVALID:"),
            "{error}"
        );
    }
}
