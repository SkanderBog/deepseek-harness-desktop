use std::collections::HashSet;
#[cfg(windows)]
use std::os::windows::fs::FileTypeExt;
use std::path::Path;

pub(super) fn internal_plugin_entry_is_ready(entry: &Path) -> bool {
    let Ok(raw) = std::fs::read(entry.join("package.json")) else {
        return false;
    };
    serde_json::from_slice::<serde_json::Value>(&raw).is_ok_and(|manifest| manifest.is_object())
}

pub(super) fn dedupe_profile_bundles(manifest: &mut serde_json::Value) -> bool {
    let Some(bundles) = manifest
        .get_mut("dsh")
        .and_then(|dsh| dsh.get_mut("profile"))
        .and_then(|profile| profile.get_mut("bundles"))
        .and_then(serde_json::Value::as_array_mut)
    else {
        return false;
    };

    let before = bundles.len();
    let mut seen = HashSet::new();
    bundles.retain(|bundle| {
        bundle
            .as_str()
            .is_none_or(|name| seen.insert(name.to_string()))
    });
    bundles.len() != before
}

pub(super) fn remove_duplicate_bundle_entries_from_patch(
    patch: &mut serde_yaml::Value,
    bundle_ids: &HashSet<&str>,
) -> bool {
    let mut modified = false;
    remove_duplicate_inserted_entries(patch, bundle_ids, &mut modified);
    modified
}

fn remove_duplicate_inserted_entries(
    value: &mut serde_yaml::Value,
    bundle_ids: &HashSet<&str>,
    modified: &mut bool,
) {
    match value {
        serde_yaml::Value::Sequence(items) => {
            for item in items {
                remove_duplicate_inserted_entries(item, bundle_ids, modified);
            }
        }
        serde_yaml::Value::Mapping(mapping) => {
            if let Some(insert) = mapping
                .get_mut(serde_yaml::Value::String("insert".to_string()))
                .and_then(serde_yaml::Value::as_sequence_mut)
            {
                let before = insert.len();
                insert.retain(|item| {
                    item.get("id")
                        .and_then(serde_yaml::Value::as_str)
                        .is_none_or(|id| !bundle_ids.contains(id))
                });
                *modified |= insert.len() != before;
            }
            for child in mapping.values_mut() {
                remove_duplicate_inserted_entries(child, bundle_ids, modified);
            }
        }
        _ => {}
    }
}

pub(super) fn remove_internal_plugins_from_manifest(
    manifest: &mut serde_json::Value,
    names: &HashSet<&str>,
) -> bool {
    let mut modified = false;
    if let Some(dependencies) = manifest
        .get_mut("dependencies")
        .and_then(serde_json::Value::as_object_mut)
    {
        for name in names {
            modified |= dependencies.remove(*name).is_some();
        }
    }
    if let Some(bundles) = manifest
        .get_mut("dsh")
        .and_then(|dsh| dsh.get_mut("profile"))
        .and_then(|profile| profile.get_mut("bundles"))
        .and_then(serde_json::Value::as_array_mut)
    {
        let before = bundles.len();
        bundles.retain(|bundle| bundle.as_str().is_none_or(|name| !names.contains(name)));
        modified |= bundles.len() != before;
    }
    modified
}

pub(super) fn write_profile_manifest(
    path: &Path,
    manifest: &serde_json::Value,
) -> Result<(), String> {
    use std::io::Write;

    let rendered = serde_json::to_string_pretty(manifest)
        .map_err(|e| format!("INTERNAL_PLUGIN_MANIFEST_RENDER_FAILED: {e}"))?;
    let temp = path.with_extension(format!("json.internal.{}.tmp", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(format!("{rendered}\n").as_bytes())?;
        file.sync_all()?;
        drop(file);
        replace_manifest_file(&temp, path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("INTERNAL_PLUGIN_MANIFEST_WRITE_FAILED: {e}"));
    }
    log::info!("Profile manifest rewritten: {}", path.display());
    Ok(())
}

#[cfg(not(windows))]
fn replace_manifest_file(temp: &Path, path: &Path) -> std::io::Result<()> {
    std::fs::rename(temp, path)
}

#[cfg(windows)]
fn replace_manifest_file(temp: &Path, path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let temp_wide: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
    let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let moved = unsafe {
        MoveFileExW(
            temp_wide.as_ptr(),
            path_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn remove_stale_plugin_entry(entry: &Path) -> std::io::Result<()> {
    let metadata = match std::fs::symlink_metadata(entry) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let file_type = metadata.file_type();
    #[cfg(windows)]
    if file_type.is_symlink_dir() {
        return std::fs::remove_dir(entry);
    }
    if file_type.is_symlink() {
        return std::fs::remove_file(entry);
    }
    if file_type.is_dir() {
        return std::fs::remove_dir_all(entry);
    }
    std::fs::remove_file(entry)
}

pub(super) fn is_local_link_dep(spec: &str) -> bool {
    spec.starts_with("link:") || spec.starts_with("file:")
}

pub(super) fn collect_dangling_local_link_deps(
    manifest: &serde_json::Value,
    keep: &HashSet<&str>,
    profile: &Path,
) -> Vec<String> {
    let Some(dependencies) = manifest
        .get("dependencies")
        .and_then(serde_json::Value::as_object)
    else {
        return Vec::new();
    };
    let mut dangling: Vec<String> = dependencies
        .iter()
        .filter(|(name, spec)| {
            !keep.contains(name.as_str())
                && spec.as_str().is_some_and(|spec| {
                    is_local_link_dep(spec) && !local_link_target_exists(spec, profile)
                })
        })
        .map(|(name, _)| name.clone())
        .collect();
    dangling.sort();
    dangling
}

fn local_link_target_exists(spec: &str, profile: &Path) -> bool {
    let Some(raw) = spec
        .strip_prefix("link:")
        .or_else(|| spec.strip_prefix("file:"))
    else {
        return true;
    };
    let trimmed = raw.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return false;
    }
    let normalized = trimmed.replace('/', std::path::MAIN_SEPARATOR_STR);
    let path = dunce::simplified(Path::new(&normalized)).to_path_buf();
    let resolved = if path.is_absolute() {
        path
    } else {
        profile.join(path)
    };
    resolved.is_dir()
}

pub(super) fn dep_matches_spec(actual: &str, expected: &str) -> bool {
    let norm = |spec: &str| {
        let stripped = spec
            .strip_prefix("link:")
            .or_else(|| spec.strip_prefix("file:"))
            .unwrap_or(spec);
        let normalized = stripped.replace(['/', '\\'], std::path::MAIN_SEPARATOR_STR);
        let path = Path::new(&normalized);
        let resolved = if path.is_absolute() {
            dunce::canonicalize(path).ok()
        } else {
            None
        };
        resolved
            .unwrap_or_else(|| dunce::simplified(path).to_path_buf())
            .to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_string()
    };
    let actual = norm(actual);
    let expected = norm(expected);
    if cfg!(windows) {
        actual.eq_ignore_ascii_case(&expected)
    } else {
        actual == expected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_plugin_entry_requires_readable_manifest_object() {
        let root = std::env::temp_dir().join(format!(
            "dsh-internal-entry-readiness-{}",
            std::process::id()
        ));
        let manifest = root.join("package.json");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        assert!(!internal_plugin_entry_is_ready(&root));

        std::fs::write(&manifest, br#"{"name":"dsh-tauri"}"#).unwrap();
        assert!(internal_plugin_entry_is_ready(&root));

        std::fs::write(&manifest, br#"{"name":"dsh-tauri""#).unwrap();
        assert!(!internal_plugin_entry_is_ready(&root));

        std::fs::write(&manifest, b"[]").unwrap();
        assert!(!internal_plugin_entry_is_ready(&root));

        std::fs::write(&manifest, [0xff, 0xfe, 0xfd]).unwrap();
        assert!(!internal_plugin_entry_is_ready(&root));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_profile_bundles_are_removed_without_reordering() {
        let mut manifest = serde_json::json!({
            "dsh": { "profile": { "bundles": ["dsh-tauri", "dshmarket", "dsh-tauri", "dshmarket"] } }
        });

        assert!(dedupe_profile_bundles(&mut manifest));
        assert_eq!(
            manifest["dsh"]["profile"]["bundles"],
            serde_json::json!(["dsh-tauri", "dshmarket"])
        );
        assert!(!dedupe_profile_bundles(&mut manifest));
    }

    #[test]
    fn stale_internal_manifest_entries_are_removed_without_touching_other_plugins() {
        let mut manifest = serde_json::json!({
            "private": true,
            "dependencies": {
                "dsh-tauri": "file:/Applications/Deepseek Harness Desktop.app/Contents/Resources/resources/preset-plugins/dsh-tauri",
                "dsh-tauri-ui": "link:/Applications/Deepseek Harness Desktop.app/Contents/Resources/resources/preset-plugins/dsh-tauri-ui",
                "dshmarket": "github:dsh-market/dshmarket"
            },
            "dsh": {
                "profile": {
                    "bundles": ["dsh-tauri", "dsh-tauri-ui", "dshmarket"]
                }
            }
        });
        let names = HashSet::from(["dsh-tauri", "dsh-tauri-ui"]);

        assert!(remove_internal_plugins_from_manifest(&mut manifest, &names));
        assert_eq!(
            manifest["dependencies"],
            serde_json::json!({ "dshmarket": "github:dsh-market/dshmarket" })
        );
        assert_eq!(
            manifest["dsh"]["profile"]["bundles"],
            serde_json::json!(["dshmarket"])
        );
        assert!(!remove_internal_plugins_from_manifest(
            &mut manifest,
            &names
        ));
    }

    #[test]
    fn manifest_replacement_preserves_original_when_temp_write_fails() {
        let root = std::env::temp_dir().join(format!(
            "dsh-internal-manifest-write-failure-{}",
            std::process::id()
        ));
        let path = root.join("package.json");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("sentinel"), "original").unwrap();

        let error = write_profile_manifest(&path, &serde_json::json!({ "private": true }))
            .expect_err("directory destination must reject replacement");

        assert!(error.starts_with("INTERNAL_PLUGIN_MANIFEST_WRITE_FAILED:"));
        assert_eq!(
            std::fs::read_to_string(path.join("sentinel")).unwrap(),
            "original"
        );
        assert!(
            !root
                .join(format!("package.json.internal.{}.tmp", std::process::id()))
                .exists()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn stale_plain_directory_is_removed() {
        let root = std::env::temp_dir().join(format!(
            "dsh-internal-stale-directory-{}",
            std::process::id()
        ));
        let entry = root.join("node_modules/dsh-tauri-ui");
        std::fs::create_dir_all(&entry).unwrap();
        std::fs::write(entry.join("partial"), "broken").unwrap();

        remove_stale_plugin_entry(&entry).unwrap();

        assert!(!entry.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn stale_symlink_is_removed_without_touching_target() {
        let root =
            std::env::temp_dir().join(format!("dsh-internal-stale-symlink-{}", std::process::id()));
        let target = root.join("old-app/dsh-tauri-ui");
        let entry = root.join("profile/node_modules/dsh-tauri-ui");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("package.json"), "{}").unwrap();
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&target, &entry).unwrap();

        remove_stale_plugin_entry(&entry).unwrap();

        assert!(target.join("package.json").is_file());
        assert!(std::fs::symlink_metadata(&entry).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_is_removed_idempotently() {
        let root = std::env::temp_dir().join(format!(
            "dsh-internal-dangling-symlink-{}",
            std::process::id()
        ));
        let entry = root.join("profile/node_modules/dsh-tauri-ui");
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(root.join("missing-app/dsh-tauri-ui"), &entry).unwrap();

        remove_stale_plugin_entry(&entry).unwrap();
        remove_stale_plugin_entry(&entry).unwrap();

        assert!(std::fs::symlink_metadata(&entry).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn local_link_dep_detects_only_directory_specs() {
        assert!(is_local_link_dep(
            "link:C:/Apps/dsh/resources/internal-plugins/dsh-tauri"
        ));
        assert!(is_local_link_dep("file:/Applications/.../dsh-tauri-ui"));
        assert!(!is_local_link_dep("dsh-tauri@0.2.0"));
        assert!(!is_local_link_dep("github:dsh-market/dshmarket"));
        assert!(!is_local_link_dep("workspace:*"));
        assert!(!is_local_link_dep(""));
    }

    #[test]
    fn dangling_local_link_deps_exclude_presets_and_registry_specs() {
        let root =
            std::env::temp_dir().join(format!("dsh-dangling-link-deps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let profile = root.join("profile");
        let alive = root.join("packages/dsh-tauri-ui");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::create_dir_all(&alive).unwrap();
        let alive_spec = format!("link:{}", alive.to_string_lossy());

        let manifest = serde_json::json!({
            "dependencies": {
                "dsh-tauri-panel": "link:D:/gone/packages/dsh-tauri-panel",
                "dsh-tauri-ui": alive_spec,
                "dsh-tauri": "link:D:/gone/packages/dsh-tauri",
                "dshmarket": "github:dsh-market/dshmarket",
                "dsh-better-sidebar": "1.0.0",
                "dsh-tauri-pet": "link:missing-sibling-package"
            }
        });
        let keep = HashSet::from(["dsh-tauri"]);

        let dangling = collect_dangling_local_link_deps(&manifest, &keep, &profile);

        assert_eq!(
            dangling,
            vec!["dsh-tauri-panel".to_string(), "dsh-tauri-pet".to_string()]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn dangling_local_link_deps_ignore_manifests_without_dependencies() {
        let profile = std::env::temp_dir();
        let manifest = serde_json::json!({ "name": "dsh-profile-web", "private": true });

        assert!(collect_dangling_local_link_deps(&manifest, &HashSet::new(), &profile).is_empty());
    }

    #[test]
    fn dep_spec_matches_itself() {
        let expected = "link:C:/Apps/dsh/resources/internal-plugins/dsh-tauri";
        assert!(dep_matches_spec(expected, expected));
        assert!(dep_matches_spec(
            "C:/Apps/dsh/resources/internal-plugins/dsh-tauri",
            expected
        ));
        assert!(dep_matches_spec(
            "link:C:/Apps/dsh/resources/internal-plugins/dsh-tauri/",
            expected
        ));
        assert!(dep_matches_spec(
            "link:C:\\Apps\\dsh\\resources\\internal-plugins\\dsh-tauri",
            expected
        ));
        assert!(dep_matches_spec(
            "file:C:/Apps/dsh/resources/internal-plugins/dsh-tauri",
            expected
        ));
    }

    #[test]
    fn dep_spec_does_not_reinstall_healthy_workspace_path_alias() {
        let root = std::env::temp_dir().join(format!(
            "dsh-dep-path-alias-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("src-tauri")).unwrap();
        std::fs::create_dir_all(root.join("packages/dsh-tauri")).unwrap();
        let actual =
            crate::service::plugin::preset::bundled_dep_spec(&root.join("packages/dsh-tauri"));
        let expected = crate::service::plugin::preset::bundled_dep_spec(
            &root.join("src-tauri/../packages/dsh-tauri"),
        );
        let matches = dep_matches_spec(&actual, &expected);
        std::fs::remove_dir_all(&root).unwrap();
        assert!(
            matches,
            "healthy plugin must not reinstall: actual={actual}, expected={expected}"
        );
    }

    #[test]
    fn dep_spec_rejects_wrong_path_or_source() {
        let expected = "link:C:/Apps/dsh/resources/internal-plugins/dsh-tauri";
        assert!(!dep_matches_spec("dsh-tauri@0.2.0", expected));
        assert!(!dep_matches_spec("^0.2.0", expected));
        let cwd_spec =
            crate::service::plugin::preset::bundled_dep_spec(&std::env::current_dir().unwrap());
        assert!(!dep_matches_spec(".", &cwd_spec));
        assert!(!dep_matches_spec("link:D:/elsewhere/dsh-tauri", expected));
        assert!(!dep_matches_spec(
            "link:D:/Apps/dsh/resources/internal-plugins/dsh-tauri",
            expected
        ));
    }

    #[cfg(windows)]
    #[test]
    fn dep_spec_case_insensitive_on_windows() {
        let expected = "link:C:/Apps/dsh/resources/internal-plugins/dsh-tauri";
        assert!(dep_matches_spec(
            "link:c:/apps/DSH/resources/internal-plugins/Dsh-Tauri",
            expected
        ));
        assert!(dep_matches_spec(
            "link://?/C:/Apps/dsh/resources/internal-plugins/dsh-tauri",
            expected
        ));
    }
}
