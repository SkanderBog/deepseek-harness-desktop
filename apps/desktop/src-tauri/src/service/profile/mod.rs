//! 核心 bundle 层自愈（issue #452）：`dsh.profile.bundles` 必须始终带

use crate::config;
use crate::service::fs_guard;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub mod migrate;
use crate::AppHandle;

pub const DEFAULT_PROFILE: &str = "web";

pub const DESKTOP_PROFILE: &str = "tauri";

const LEGACY_DESKTOP_PROFILE: &str = "desktop";

pub const SAFE_PROFILE: &str = "safe";

const WEB_PROFILE_BUNDLES: [&str; 2] = ["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"];

const NON_WEB_SURFACE_BUNDLES: [&str; 4] = [
    "@deepseek-ai/dsh-headless",
    "@deepseek-ai/dsh-acp-app",
    "@deepseek-ai/dsh-sdk-app",
    "@deepseek-ai/dsh-sdk-minimal",
];

const PROFILE_PATCH_TEMPLATE: &str = "# Your patch layer for this dsh profile, applied after every bundle layer:\n# a top-level YAML array of loader patch entries (id-targeted config\n# overrides, disables, and insert lists; `!!js` expressions allowed).\n[]\n";

const PROFILE_PNPM_WORKSPACE: &str = "packages:\n  - .\n\nnodeLinker: hoisted\nautoInstallPeers: false\n\n# The desktop runtime intentionally reviews this fresh transitive release.\nminimumReleaseAgeExclude:\n  - zod@4.4.3\n";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub default: bool,
    pub active: bool,
}

pub fn profile_dir_of(app_handle: &AppHandle, id: &str) -> PathBuf {
    config::get_dsh_data_path(app_handle)
        .join("profiles")
        .join(id)
}

const PROFILE_MINIMUM_RELEASE_AGE_EXCLUDES: [&str; 1] = ["zod@4.4.3"];

/// （issue #526）。
pub(crate) fn parse_workspace_document(content: &str) -> Result<(Value, bool), String> {
    match serde_yaml::from_str::<Value>(content) {
        Ok(value) => Ok((value, false)),
        Err(single_err) => {
            let mut documents = Vec::new();
            for document in serde_yaml::Deserializer::from_str(content) {
                match Value::deserialize(document) {
                    Ok(value) => documents.push(value),
                    Err(_) => return Err(single_err.to_string()),
                }
            }
            if documents.len() < 2 {
                return Err(single_err.to_string());
            }
            let mut merged = Mapping::new();
            for document in documents {
                let Some(mapping) = document.as_mapping() else {
                    return Err(single_err.to_string());
                };
                for (key, value) in mapping {
                    merged.insert(key.clone(), value.clone());
                }
            }
            Ok((Value::Mapping(merged), true))
        }
    }
}

pub(crate) fn ensure_profile_pnpm_policy(app_handle: &AppHandle) -> Result<(), String> {
    let entries: Vec<String> = PROFILE_MINIMUM_RELEASE_AGE_EXCLUDES
        .iter()
        .map(|package| (*package).to_string())
        .collect();
    release_age_excludes_at(
        &profile_dir_of(app_handle, &active_profile(app_handle)),
        &entries,
    )
}

pub(crate) fn profile_release_age_excluded(app_handle: &AppHandle, entry: &str) -> bool {
    let path = profile_dir_of(app_handle, &active_profile(app_handle)).join("pnpm-workspace.yaml");
    let Ok(content) = fs::read_to_string(&path) else {
        return false;
    };
    let Ok((document, _)) = parse_workspace_document(&content) else {
        return false;
    };
    let Some(sequence) = document
        .as_mapping()
        .and_then(|mapping| mapping.get(Value::String("minimumReleaseAgeExclude".to_string())))
        .and_then(|value| value.as_sequence())
    else {
        return false;
    };
    release_age_excluded(sequence, entry)
}

fn release_age_package(entry: &str) -> Option<(&str, &str)> {
    static NAME: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^(?:@[a-zA-Z0-9._-]+/)?[a-zA-Z0-9._-]+$").unwrap()
    });
    let (name, versions) = entry.rsplit_once('@')?;
    (NAME.is_match(name) && versions.split("||").all(|part| !part.trim().is_empty()))
        .then_some((name, versions))
}

fn release_age_excluded(sequence: &[Value], entry: &str) -> bool {
    let Some((name, version)) = release_age_package(entry) else {
        return false;
    };
    sequence.iter().filter_map(Value::as_str).any(|item| {
        release_age_package(item).is_some_and(|(package, versions)| {
            package == name && versions.split("||").any(|part| part.trim() == version)
        })
    })
}

fn normalize_release_age_excludes(sequence: &[Value]) -> Vec<Value> {
    let mut result = Vec::with_capacity(sequence.len());
    let mut packages = std::collections::HashMap::<String, usize>::new();
    for item in sequence {
        let Some((name, versions)) = item.as_str().and_then(release_age_package) else {
            if !result.contains(item) {
                result.push(item.clone());
            }
            continue;
        };
        let index = *packages.entry(name.to_string()).or_insert_with(|| {
            result.push(Value::String(format!("{name}@")));
            result.len() - 1
        });
        let value = result[index].as_str().unwrap();
        let mut merged: Vec<&str> = value[name.len() + 1..]
            .split("||")
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();
        for version in versions.split("||").map(str::trim) {
            if !merged.contains(&version) {
                merged.push(version);
            }
        }
        result[index] = Value::String(format!("{name}@{}", merged.join(" || ")));
    }
    result
}

pub(crate) fn allow_profile_release_age(
    app_handle: &AppHandle,
    entries: &[String],
    expected_profile: Option<&str>,
) -> Result<(), String> {
    if entries.is_empty() {
        return Err("PROFILE_RELEASE_AGE_EMPTY: no release-age exemption to record".to_string());
    }
    let profile = active_profile(app_handle);
    if expected_profile.is_some_and(|expected| expected != profile) {
        return Err(
            "PROFILE_RELEASE_AGE_CHANGED: active profile changed before authorization".to_string(),
        );
    }
    release_age_excludes_at(&profile_dir_of(app_handle, &profile), entries)
}

pub(crate) fn release_age_excludes_at(dir: &Path, entries: &[String]) -> Result<(), String> {
    let path = dir.join("pnpm-workspace.yaml");
    let existing = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            PROFILE_PNPM_WORKSPACE.to_string()
        }
        Err(error) => return Err(format!("PROFILE_WORKSPACE_READ: {error}")),
    };
    // 错误里带上文件路径：issue #526 的报错只有解析错误文本，用户无从知道要修哪个文件。
    let (mut document, normalized) = parse_workspace_document(&existing)
        .map_err(|e| format!("PROFILE_WORKSPACE_INVALID_YAML: {}: {e}", path.display()))?;
    if normalized {
        log::warn!(
            "PROFILE_WORKSPACE_MULTI_DOCUMENT: normalized {} to a single YAML document",
            path.display()
        );
    }
    let mapping = document.as_mapping_mut().ok_or_else(|| {
        "PROFILE_WORKSPACE_NOT_MAP: pnpm-workspace.yaml must be a mapping".to_string()
    })?;
    let key = Value::String("minimumReleaseAgeExclude".to_string());
    let excludes = mapping
        .entry(key)
        .or_insert_with(|| Value::Sequence(Vec::new()));
    let sequence = excludes.as_sequence_mut().ok_or_else(|| {
        "PROFILE_WORKSPACE_POLICY_INVALID: minimumReleaseAgeExclude must be a sequence".to_string()
    })?;
    let mut combined = sequence.clone();
    combined.extend(entries.iter().cloned().map(Value::String));
    let merged = normalize_release_age_excludes(&combined);
    let changed = normalized || *sequence != merged;
    *sequence = merged;
    if changed {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("PROFILE_WORKSPACE_MKDIR: {e}"))?;
        }
        let rendered = serde_yaml::to_string(&document)
            .map_err(|e| format!("PROFILE_WORKSPACE_RENDER: {e}"))?;
        fs::write(&path, rendered).map_err(|e| format!("PROFILE_WORKSPACE_WRITE: {e}"))?;
        log::info!(
            "Ensured profile pnpm release-age policy: {}",
            path.display()
        );
    }
    Ok(())
}

pub fn active_profile(app_handle: &AppHandle) -> String {
    if let Some(profile) = crate::service::workflow::instances::current_profile() {
        return profile;
    }
    let stored = config::get_store_dat_setting(app_handle).active_profile;
    if !stored.is_empty()
        && stored != DEFAULT_PROFILE
        && profile_dir_of(app_handle, &stored).is_dir()
    {
        stored
    } else {
        DEFAULT_PROFILE.to_string()
    }
}

fn manifest_display_name(dir: &Path, id: &str) -> String {
    let raw = fs::read_to_string(dir.join("package.json"))
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(String::from))
        .unwrap_or_default();
    let stripped = raw
        .strip_prefix("dsh-profile-")
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| raw);
    let fallback = id.to_string();
    let name = if stripped.is_empty() {
        fallback
    } else {
        stripped
    };
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => name,
    }
}

pub fn list(app_handle: &AppHandle) -> Vec<Profile> {
    let active = active_profile(app_handle);
    let profiles_root = config::get_dsh_data_path(app_handle).join("profiles");
    let mut out: Vec<Profile> = Vec::new();
    if let Ok(entries) = fs::read_dir(&profiles_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(id) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if id.starts_with('.') || id == "node_modules" {
                continue;
            }
            out.push(Profile {
                id: id.to_string(),
                name: manifest_display_name(&path, id),
                default: id == DEFAULT_PROFILE,
                active: id == active,
            });
        }
    }
    if !out.iter().any(|p| p.id == DEFAULT_PROFILE) {
        out.push(Profile {
            id: DEFAULT_PROFILE.to_string(),
            name: "Web".to_string(),
            default: true,
            active: active == DEFAULT_PROFILE,
        });
    }
    out.sort_by_key(|p| (!p.default, p.id.clone()));
    out
}

fn normalize_profile_id(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending_sep = false;
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            if pending_sep && !out.is_empty() {
                out.push('-');
            }
            pending_sep = false;
            out.push(c);
        } else if c == ' ' || c == '-' || c == '_' {
            pending_sep = true;
        }
    }
    out.trim_matches('-').to_string()
}

pub fn create(app_handle: &AppHandle, name: &str) -> Result<Profile, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("PROFILE_EMPTY_NAME: profile name is empty".to_string());
    }
    let id = normalize_profile_id(trimmed);
    if id.is_empty() {
        return Err("PROFILE_INVALID_NAME: profile name has no usable characters".to_string());
    }
    if id.len() > 64 {
        return Err("PROFILE_NAME_TOO_LONG: profile id exceeds 64 characters".to_string());
    }
    if id == DEFAULT_PROFILE {
        return Err("PROFILE_RESERVED: this name is reserved".to_string());
    }
    let dir = profile_dir_of(app_handle, &id);
    if dir.is_dir() {
        return Err(format!("PROFILE_EXISTS: profile {id} already exists"));
    }
    if let Err(error) = init_profile_dir(&dir, &id) {
        if let Err(cleanup) = fs::remove_dir_all(&dir) {
            log::warn!("failed to roll back the half-initialized profile {id}: {cleanup}");
        }
        return Err(error);
    }
    Ok(Profile {
        id,
        name: trimmed.to_string(),
        default: false,
        active: false,
    })
}

pub fn select(app_handle: &AppHandle, id: &str) -> Result<Profile, String> {
    ensure_selectable(app_handle, id)?;
    list(app_handle)
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "PROFILE_NOT_FOUND: profile disappeared after switch".to_string())
}

pub fn set_active(app_handle: &AppHandle, id: &str) -> Result<Profile, String> {
    ensure_selectable(app_handle, id)?;
    let mut setting = config::get_store_dat_setting(app_handle);
    setting.active_profile = id.to_string();
    config::set_store_dat_setting(app_handle, setting);
    select(app_handle, id)
}

fn ensure_selectable(app_handle: &AppHandle, id: &str) -> Result<(), String> {
    let profiles_root = config::get_dsh_data_path(app_handle).join("profiles");
    let dir = fs_guard::join_safe(&profiles_root, id)?;
    if id != DEFAULT_PROFILE && !dir.is_dir() {
        return Err(format!("PROFILE_NOT_FOUND: profile {id} does not exist"));
    }
    Ok(())
}

/// `@deepseek-ai/dsh-web-app` 会让桌面端此后每次启动都失败（issue #452）。
fn ensure_desktop_profile_with_root(profiles_root: &Path) -> Result<(), String> {
    init_profile_dir(&profiles_root.join(DESKTOP_PROFILE), DESKTOP_PROFILE)
}

///    （含 issue #452 的半初始化补齐），保证改名失败时启动仍有可用档案；
pub fn migrate_desktop_profile_name(app_handle: &AppHandle) {
    let profiles_root = config::get_dsh_data_path(app_handle).join("profiles");
    migrate_desktop_profile_in_root(app_handle, &profiles_root);

    if config::is_first_install()
        && !config::get_store_dat_setting(app_handle).desktop_profile_ready
    {
        if let Err(e) = set_active(app_handle, DESKTOP_PROFILE) {
            log::warn!("first-run Desktop profile switch failed: {e}");
            return;
        }
        config::update_store_dat_setting(app_handle, |setting| {
            setting.desktop_profile_ready = true;
        });
        log::info!(
            "First-run bootstrap: Desktop profile created and activated ({DESKTOP_PROFILE})"
        );
    }
}

fn migrate_desktop_profile_in_root(app_handle: &AppHandle, profiles_root: &Path) {
    if adopt_tauri_profile_dir(profiles_root) == ProfileDirAdoption::Failed {
        log::warn!(
            "Desktop profile rename failed; keeping {LEGACY_DESKTOP_PROFILE} and retrying next start"
        );
        return;
    }

    if let Err(e) = ensure_desktop_profile_with_root(profiles_root) {
        log::warn!("Desktop profile init failed: {e}");
    }

    let stored = config::get_store_dat_setting(app_handle).active_profile;
    if stored == LEGACY_DESKTOP_PROFILE {
        config::update_store_dat_setting(app_handle, |setting| {
            setting.active_profile = DESKTOP_PROFILE.to_string();
        });
        log::info!("Active profile switched: {LEGACY_DESKTOP_PROFILE} -> {DESKTOP_PROFILE}");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileDirAdoption {
    Adopted,
    Blocked,
    Failed,
}

fn adopt_tauri_profile_dir(profiles_root: &Path) -> ProfileDirAdoption {
    let old_dir = profiles_root.join(LEGACY_DESKTOP_PROFILE);
    let new_dir = profiles_root.join(DESKTOP_PROFILE);

    let mut adoption = ProfileDirAdoption::Adopted;
    if old_dir.is_dir() {
        if new_dir.exists() {
            adoption = ProfileDirAdoption::Blocked;
            log::warn!(
                "Both {} and {} exist; leaving the legacy profile untouched",
                old_dir.display(),
                new_dir.display()
            );
        } else {
            match fs::rename(&old_dir, &new_dir) {
                Ok(()) => log::info!(
                    "Desktop profile renamed: {} -> {}",
                    old_dir.display(),
                    new_dir.display()
                ),
                Err(e) => {
                    adoption = ProfileDirAdoption::Failed;
                    log::warn!(
                        "Desktop profile rename failed ({} -> {}): {e}; keeping the legacy profile for the next start",
                        old_dir.display(),
                        new_dir.display()
                    );
                }
            }
        }
    }

    let manifest = new_dir.join("package.json");
    if let Ok(content) = fs::read_to_string(&manifest) {
        let stale_name = serde_json::from_str::<serde_json::Value>(&content)
            .ok()
            .and_then(|value| {
                value
                    .get("name")
                    .and_then(|name| name.as_str())
                    .map(str::to_owned)
            })
            .is_some_and(|name| name == format!("dsh-profile-{LEGACY_DESKTOP_PROFILE}"));
        if stale_name {
            match rewrite_manifest_name(&new_dir, DESKTOP_PROFILE) {
                Ok(()) => log::info!("Desktop profile manifest renamed to {DESKTOP_PROFILE}"),
                Err(e) => log::warn!("Desktop profile manifest rename failed: {e}"),
            }
        }
    }

    adoption
}

pub fn ensure_safe_profile(app_handle: &AppHandle) -> Result<(), String> {
    let profiles_root = config::get_dsh_data_path(app_handle).join("profiles");
    init_profile_dir(&profiles_root.join(SAFE_PROFILE), SAFE_PROFILE)
}

pub fn remove(app_handle: &AppHandle, id: &str) -> Result<(), String> {
    if id == DEFAULT_PROFILE {
        return Err(
            "PROFILE_DEFAULT_NOT_REMOVABLE: the default profile cannot be removed".to_string(),
        );
    }
    if id == active_profile(app_handle) || crate::service::workflow::instances::is_attached(id) {
        return Err(
            "PROFILE_ACTIVE_NOT_REMOVABLE: the active profile cannot be removed".to_string(),
        );
    }
    let profiles_root = config::get_dsh_data_path(app_handle).join("profiles");
    let dir = fs_guard::safe_remove_target(&profiles_root, id)?;
    if !dir.is_dir() {
        return Err(format!("PROFILE_NOT_FOUND: profile {id} does not exist"));
    }
    fs::remove_dir_all(&dir).map_err(|e| format!("PROFILE_REMOVE_FAILED: {e}"))
}

pub fn reset(app_handle: &AppHandle, id: &str) -> Result<(), String> {
    let profiles_root = config::get_dsh_data_path(app_handle).join("profiles");
    reset_with_root(&profiles_root, id)
}

pub fn reset_with_root(profiles_root: &Path, id: &str) -> Result<(), String> {
    if fs_guard::join_safe(profiles_root, id)?.is_dir() {
        let dir = fs_guard::safe_remove_target(profiles_root, id)?;
        fs::remove_dir_all(&dir).map_err(|e| format!("PROFILE_RESET_FAILED: {e}"))?;
    }
    init_profile_dir(&profiles_root.join(id), id)
}

pub fn clone_with_root(
    profiles_root: &Path,
    source_id: &str,
    name: Option<&str>,
) -> Result<Profile, String> {
    fs_guard::validate_id(source_id)?;
    let src_dir = fs_guard::join_safe(profiles_root, source_id)?;
    if !src_dir.is_dir() {
        return Err(format!(
            "PROFILE_NOT_FOUND: profile {source_id} does not exist"
        ));
    }

    let new_id = match name {
        Some(n) => {
            let trimmed = n.trim();
            if trimmed.is_empty() {
                return Err("PROFILE_EMPTY_NAME: profile name is empty".to_string());
            }
            let id = normalize_profile_id(trimmed);
            if id.is_empty() {
                return Err(
                    "PROFILE_INVALID_NAME: profile name has no usable characters".to_string(),
                );
            }
            if id.len() > 64 {
                return Err("PROFILE_NAME_TOO_LONG: profile id exceeds 64 characters".to_string());
            }
            if id == DEFAULT_PROFILE {
                return Err("PROFILE_RESERVED: this name is reserved".to_string());
            }
            let target = profiles_root.join(&id);
            if target.is_dir() {
                return Err(format!("PROFILE_EXISTS: profile {id} already exists"));
            }
            id
        }
        None => next_profile_id(profiles_root, source_id)?,
    };

    let dst_dir = profiles_root.join(&new_id);
    copy_dir_tree(&src_dir, &dst_dir)?;
    crate::service::migrate::purge_carried_pnpm_metadata(&dst_dir);
    rewrite_manifest_name(&dst_dir, &new_id)?;

    Ok(Profile {
        id: new_id.clone(),
        name: manifest_display_name(&dst_dir, &new_id),
        default: false,
        active: false,
    })
}

fn next_profile_id(profiles_root: &Path, base: &str) -> Result<String, String> {
    let mut n = 1;
    loop {
        if n > 1000 {
            return Err("PROFILE_CLONE_EXHAUSTED: too many clones".to_string());
        }
        let candidate = format!("{base}-{n}");
        if !profiles_root.join(&candidate).is_dir() {
            return Ok(candidate);
        }
        n += 1;
    }
}

fn copy_dir_tree(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("COPY_MKDIR: {e}"))?;
    let read_dir = fs::read_dir(src).map_err(|e| format!("COPY_READ: {e}"))?;
    let entries: Vec<_> = read_dir
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("COPY_ENTRY: {e}"))?;
    entries
        .par_iter()
        .try_for_each(|entry| -> Result<(), String> {
            let name = entry.file_name();
            if let Some(s) = name.to_str() {
                if crate::config::HARNESS_PID_MARKER_NAMES.contains(&s) || s == ".backups" {
                    return Ok(());
                }
            }
            let src_path = entry.path();
            let dst_path = dst.join(&name);
            let ty = entry.file_type().map_err(|e| format!("COPY_TYPE: {e}"))?;
            if ty.is_symlink() {
                let target =
                    std::fs::read_link(&src_path).map_err(|e| format!("COPY_LINK_READ: {e}"))?;
                copy_symlink(&target, &dst_path)?;
            } else if ty.is_dir() {
                copy_dir_tree(&src_path, &dst_path)?;
            } else if ty.is_file() {
                fs::copy(&src_path, &dst_path).map_err(|e| format!("COPY_FILE: {e}"))?;
            }
            Ok(())
        })?;
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(target: &std::path::Path, dst: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(target, dst).map_err(|e| format!("COPY_LINK_CREATE: {e}"))
}

#[cfg(windows)]
fn copy_symlink(target: &std::path::Path, dst: &Path) -> Result<(), String> {
    if dst.parent().is_some() {
        let _ = std::os::windows::fs::symlink_dir(target, dst)
            .or_else(|_| std::os::windows::fs::symlink_file(target, dst))
            .map_err(|e| log::warn!("copy_symlink failed for {}: {e}", dst.display()));
    }
    Ok(())
}

fn rewrite_manifest_name(dir: &Path, new_id: &str) -> Result<(), String> {
    let path = dir.join("package.json");
    let content = fs::read_to_string(&path).map_err(|e| format!("MANIFEST_READ: {e}"))?;
    let mut manifest: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("MANIFEST_PARSE: {e}"))?;
    if let Some(obj) = manifest.as_object_mut() {
        obj.insert(
            "name".to_string(),
            serde_json::Value::String(format!("dsh-profile-{new_id}")),
        );
    }
    rewrite_manifest_name_atomic(&path, &manifest)
}

fn rewrite_manifest_name_atomic(path: &Path, manifest: &serde_json::Value) -> Result<(), String> {
    let content =
        serde_json::to_string_pretty(manifest).map_err(|e| format!("MANIFEST_RENDER: {e}"))?;
    atomic_write(path, &format!("{content}\n"), "MANIFEST_WRITE")
}

fn web_profile_manifest(id: &str) -> serde_json::Value {
    serde_json::json!({
        "name": format!("dsh-profile-{id}"),
        "private": true,
        "dependencies": {},
        "dsh": { "profile": { "bundles": WEB_PROFILE_BUNDLES } }
    })
}

fn ensure_profile_core_bundles(dir: &Path, id: &str) -> Result<bool, String> {
    let manifest_path = dir.join("package.json");
    let mut manifest: serde_json::Value = match fs::read_to_string(&manifest_path) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|e| {
            format!(
                "PROFILE_MANIFEST_PARSE_FAILED: {}: {e}",
                manifest_path.display()
            )
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(dir).map_err(|e| format!("PROFILE_MKDIR: {e}"))?;
            let manifest = web_profile_manifest(id);
            write_profile_manifest_file(&manifest_path, &manifest)?;
            log::info!(
                "Profile manifest created with the official web template: {}",
                manifest_path.display()
            );
            return Ok(true);
        }
        Err(error) => {
            return Err(format!(
                "PROFILE_MANIFEST_READ_FAILED: {}: {error}",
                manifest_path.display()
            ));
        }
    };

    let existing = manifest
        .pointer("/dsh/profile/bundles")
        .and_then(serde_json::Value::as_array);
    let current: Vec<String> = existing
        .map(|bundles| {
            bundles
                .iter()
                .filter_map(|bundle| bundle.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if let Some(foreign) = current
        .iter()
        .find(|bundle| NON_WEB_SURFACE_BUNDLES.contains(&bundle.as_str()))
    {
        log::warn!(
            "PROFILE_CORE_BUNDLES_SKIPPED: profile {id} is a non-web dsh surface ({foreign}); \
             leaving {} untouched (the web layer would duplicate loader entries)",
            manifest_path.display()
        );
        return Ok(false);
    }

    let mut desired: Vec<String> = WEB_PROFILE_BUNDLES
        .iter()
        .map(|bundle| (*bundle).to_string())
        .collect();
    desired.extend(
        current
            .iter()
            .filter(|bundle| {
                !WEB_PROFILE_BUNDLES.contains(&bundle.as_str())
                    && bundle.as_str() != "@deepseek-ai/dsh-experimental-schedule-bundle"
            })
            .cloned(),
    );
    if existing.is_some() && current == desired {
        return Ok(false);
    }

    let root = manifest
        .as_object_mut()
        .ok_or_else(|| format!("PROFILE_MANIFEST_NOT_OBJECT: {}", manifest_path.display()))?;
    let dsh = root
        .entry("dsh".to_string())
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| {
            format!(
                "PROFILE_MANIFEST_DSH_NOT_OBJECT: {}",
                manifest_path.display()
            )
        })?;
    let profile = dsh
        .entry("profile".to_string())
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| {
            format!(
                "PROFILE_MANIFEST_PROFILE_NOT_OBJECT: {}",
                manifest_path.display()
            )
        })?;
    profile.insert("bundles".to_string(), serde_json::json!(desired));
    write_profile_manifest_file(&manifest_path, &manifest)?;
    log::warn!(
        "PROFILE_CORE_BUNDLES_RESTORED: profile {id} web bundles normalized from {:?} to {:?} in {}",
        current,
        desired,
        manifest_path.display()
    );
    Ok(true)
}

fn write_profile_manifest_file(path: &Path, manifest: &serde_json::Value) -> Result<(), String> {
    let content = serde_json::to_string_pretty(manifest)
        .map_err(|e| format!("PROFILE_MANIFEST_RENDER: {e}"))?;
    atomic_write(path, &format!("{content}\n"), "PROFILE_MANIFEST_WRITE")
}

fn atomic_write(path: &Path, content: &str, code: &str) -> Result<(), String> {
    let temp = path.with_extension(format!("json.profile.{}.tmp", std::process::id()));
    if let Err(e) = fs::write(&temp, content) {
        let _ = fs::remove_file(&temp);
        return Err(format!("{code}: {e}"));
    }
    if let Err(e) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(format!("{code}: {e}"));
    }
    Ok(())
}

/// 不抢先建目录）；档案已存在但缺核心层（issue #452 的「只有 dsh-base + 插件」
pub fn ensure_active_profile_core_bundles(app_handle: &AppHandle) -> Result<bool, String> {
    prepare_at(
        &config::get_dsh_data_path(app_handle),
        &active_profile(app_handle),
    )
}

pub(crate) fn prepare_at(data_home: &Path, id: &str) -> Result<bool, String> {
    let dir = fs_guard::join_safe(&data_home.join("profiles"), id)?;
    if !dir.is_dir() {
        return Ok(false);
    }
    ensure_profile_core_bundles(&dir, id)
}

fn init_profile_dir(dir: &Path, id: &str) -> Result<(), String> {
    // 建目录 + 真实写入探测：目录已存在但属主不是当前用户时（issue #466，典型：
    crate::service::perm::ensure_dir_writable(dir, "PROFILE_MKDIR")?;

    ensure_profile_core_bundles(dir, id)?;

    let patch_path = dir.join("cordis.patch.yml");
    if !patch_path.exists() {
        fs::write(&patch_path, PROFILE_PATCH_TEMPLATE)
            .map_err(|e| format!("PROFILE_PATCH_WRITE: {e}"))?;
    }

    let workspace_path = dir.join("pnpm-workspace.yaml");
    if !workspace_path.exists() {
        fs::write(&workspace_path, PROFILE_PNPM_WORKSPACE)
            .map_err(|e| format!("PROFILE_WORKSPACE_WRITE: {e}"))?;
    }

    let npmrc_path = dir.join(".npmrc");
    let npmrc_existing = fs::read_to_string(&npmrc_path).unwrap_or_default();
    if !npmrc_existing
        .lines()
        .any(|l| l.trim() == "confirmModulesPurge=false")
    {
        let mut content = npmrc_existing;
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str("confirmModulesPurge=false\n");
        fs::write(&npmrc_path, content).map_err(|e| format!("PROFILE_NPMRC_WRITE: {e}"))?;
    }

    Ok(())
}

#[cfg(test)]
mod clone_tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn scaffold_source(root: &Path, id: &str) -> PathBuf {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("package.json"),
            format!(r#"{{"name":"dsh-profile-{id}","private":true}}"#),
        )
        .unwrap();
        std::fs::write(dir.join("cordis.patch.yml"), "# patch\n[]\n").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/deep.txt"), "nested content").unwrap();
        dir
    }

    #[test]
    fn clone_produces_independent_copy_with_incremented_name() {
        let tmp = std::env::temp_dir().join(format!("dsh-clone-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("profiles");
        let source = scaffold_source(&root, "web");
        std::fs::write(source.join(".harness.pid"), "12345").unwrap();
        std::fs::write(source.join(".harness-nightly.pid"), "23456").unwrap();

        let profile = clone_with_root(&root, "web", None).unwrap();
        assert_eq!(profile.id, "web-1");
        assert!(!profile.default);
        assert!(!profile.active);

        let dst = root.join("web-1");
        assert!(dst.is_dir(), "cloned dir must exist");
        assert!(dst.join("package.json").is_file());
        assert!(dst.join("cordis.patch.yml").is_file());
        assert!(dst.join("sub/deep.txt").is_file());
        assert!(!dst.join(".harness.pid").exists());
        assert!(!dst.join(".harness-nightly.pid").exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn clone_skips_taken_names() {
        let tmp = std::env::temp_dir().join(format!("dsh-clone-skip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("profiles");
        scaffold_source(&root, "web");
        std::fs::create_dir_all(root.join("web-1")).unwrap();
        std::fs::write(
            root.join("web-1/package.json"),
            r#"{"name":"dsh-profile-web-1"}"#,
        )
        .unwrap();

        let profile = clone_with_root(&root, "web", None).unwrap();
        assert_eq!(profile.id, "web-2");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn clone_rewrites_manifest_name() {
        let tmp = std::env::temp_dir().join(format!("dsh-clone-manifest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("profiles");
        scaffold_source(&root, "web");

        let profile = clone_with_root(&root, "web", None).unwrap();
        let dst = root.join(&profile.id);
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dst.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["name"], format!("dsh-profile-{}", profile.id));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn clone_rejects_traversal_id() {
        let tmp = std::env::temp_dir().join(format!("dsh-clone-traversal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("profiles");
        scaffold_source(&root, "web");

        let err = clone_with_root(&root, "..", None).unwrap_err();
        assert!(
            err.contains("INVALID_ID") || err.contains("INVALID"),
            "expected traversal rejection, got: {err}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn clone_of_missing_source_returns_not_found() {
        let tmp = std::env::temp_dir().join(format!("dsh-clone-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("profiles");

        let err = clone_with_root(&root, "nonexistent", None).unwrap_err();
        assert!(
            err.contains("PROFILE_NOT_FOUND"),
            "expected PROFILE_NOT_FOUND, got: {err}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn clone_purges_carried_pnpm_metadata() {
        let tmp = std::env::temp_dir().join(format!("dsh-clone-pnpm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("profiles");
        scaffold_source(&root, "web");
        let nm = root.join("web/node_modules");
        std::fs::create_dir_all(&nm).unwrap();
        std::fs::write(
            nm.join(".modules.yaml"),
            "lockfileVersion: '9.0'\nstoreDir: /old/store\n",
        )
        .unwrap();

        let profile = clone_with_root(&root, "web", None).unwrap();
        let dst_nm = root.join(&profile.id).join("node_modules");
        assert!(dst_nm.is_dir(), "node_modules should be copied");
        assert!(
            !dst_nm.join(".modules.yaml").exists(),
            "carried .modules.yaml must be purged"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_at_rereads_the_explicit_profile_without_creating_missing_directories() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("profiles/beta");
        assert!(!prepare_at(home.path(), "beta").unwrap());
        assert!(!dir.exists());
        assert!(
            prepare_at(home.path(), "../beta")
                .unwrap_err()
                .starts_with("INVALID_ID:")
        );

        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("package.json");
        fs::write(
            &path,
            r#"{"dependencies":{"user-plugin":"1.0.0"},"dsh":{"profile":{"bundles":["user-plugin"]}}}"#,
        )
        .unwrap();
        let patch = dir.join("cordis.patch.yml");
        fs::write(&patch, "# user patch\n[]\n").unwrap();
        assert!(prepare_at(home.path(), "beta").unwrap());
        let repaired = fs::read_to_string(&path).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&repaired).unwrap();
        assert_eq!(
            manifest["dsh"]["profile"]["bundles"],
            serde_json::json!([
                "@deepseek-ai/dsh-base",
                "@deepseek-ai/dsh-web-app",
                "user-plugin"
            ])
        );
        assert_eq!(manifest["dependencies"]["user-plugin"], "1.0.0");
        assert!(!prepare_at(home.path(), "beta").unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), repaired);

        fs::write(&path, "{ broken").unwrap();
        assert!(
            prepare_at(home.path(), "beta")
                .unwrap_err()
                .starts_with("PROFILE_MANIFEST_PARSE_FAILED:")
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ broken");
        assert_eq!(fs::read_to_string(patch).unwrap(), "# user patch\n[]\n");
    }

    #[test]
    fn normalize_id_lowercases_and_joins() {
        assert_eq!(normalize_profile_id("My Work Space"), "my-work-space");
        assert_eq!(normalize_profile_id("  dev--stage  "), "dev-stage");
        assert_eq!(normalize_profile_id("中文档案"), "");
        assert_eq!(normalize_profile_id("a_b-c"), "a-b-c");
    }

    #[test]
    fn parse_workspace_document_accepts_a_single_document() {
        let (value, normalized) =
            parse_workspace_document("packages:\n  - .\nnodeLinker: hoisted\n").unwrap();
        assert!(!normalized);
        assert_eq!(
            value.get("nodeLinker").and_then(Value::as_str),
            Some("hoisted")
        );
    }

    /// issue #526：手工/工具拼接出的 `---` 多文档文件必须被归一化成单文档，
    #[test]
    fn parse_workspace_document_merges_multiple_documents() {
        let multi = "packages:\n  - .\nnodeLinker: hoisted\n---\nautoInstallPeers: false\n";
        let (value, normalized) = parse_workspace_document(multi).unwrap();

        assert!(normalized, "多文档必须标记为需要回写，否则 pnpm 仍读不了");
        let mapping = value.as_mapping().unwrap();
        assert_eq!(mapping.len(), 3, "{mapping:?}");
        assert_eq!(
            value
                .get("packages")
                .and_then(Value::as_sequence)
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(
            value.get("autoInstallPeers").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn parse_workspace_document_later_document_wins() {
        let (value, normalized) =
            parse_workspace_document("nodeLinker: hoisted\n---\nnodeLinker: isolated\n").unwrap();
        assert!(normalized);
        assert_eq!(
            value.get("nodeLinker").and_then(Value::as_str),
            Some("isolated")
        );
    }

    #[test]
    fn parse_workspace_document_rejects_non_mapping_documents() {
        assert!(parse_workspace_document("a: 1\n---\n- not\n- a\n- map\n").is_err());
    }

    /// 真正的语法错误（issue #49 的重复映射键）不能被当成「多文档」而带病归一化。
    #[test]
    fn parse_workspace_document_keeps_real_syntax_errors() {
        let duplicate = "allowBuilds:\n  node-pty: true\n  node-pty: true\n";
        assert!(parse_workspace_document(duplicate).is_err());
    }

    #[test]
    fn release_age_excludes_merge_every_exact_package_without_expanding_ranges() {
        let dir = std::env::temp_dir().join(format!("dsh-policy-merge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pnpm-workspace.yaml");
        std::fs::write(&path, "packages:\n  - .\nnodeLinker: hoisted\nminimumReleaseAgeExclude:\n  - foo@^1.0.0\n  - '@scope/pkg@2.0.0'\n  - foo@1.2.3\n  - foo@1.2.3 || 1.3.0\n  - '@scope/pkg@2.1.0'\n  - '@scope/*@3.0.0'\n  - '@scope/other@3.1.0'\n  - zod@4.4.3\n").unwrap();
        release_age_excludes_at(&dir, &["zod@4.4.3".to_string()]).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        let document: Value = serde_yaml::from_str(&first).unwrap();
        let entries: Vec<&str> = document["minimumReleaseAgeExclude"]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|entry| entry.as_str().unwrap())
            .collect();
        assert_eq!(
            entries,
            vec![
                "foo@^1.0.0 || 1.2.3 || 1.3.0",
                "@scope/pkg@2.0.0 || 2.1.0",
                "@scope/*@3.0.0",
                "@scope/other@3.1.0",
                "zod@4.4.3"
            ]
        );
        assert_eq!(document["nodeLinker"].as_str(), Some("hoisted"));
        release_age_excludes_at(
            &dir,
            &["foo@1.4.0".to_string(), "@scope/pkg@2.1.0".to_string()],
        )
        .unwrap();
        let merged = std::fs::read_to_string(&path).unwrap();
        let document: Value = serde_yaml::from_str(&merged).unwrap();
        assert_eq!(
            document["minimumReleaseAgeExclude"][0].as_str(),
            Some("foo@^1.0.0 || 1.2.3 || 1.3.0 || 1.4.0")
        );
        release_age_excludes_at(&dir, &["zod@4.4.3".to_string(), "foo@1.4.0".to_string()]).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), merged);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn release_age_excluded_reads_exact_or_branches_not_ranges_or_other_packages() {
        let sequence: Value = serde_yaml::from_str(
            "- foo@^1.0.0 || 1.2.3 || 1.3.0\n- '@scope/pkg@2.0.0 || 2.1.0'\n- '@scope/*@3.0.0'\n",
        )
        .unwrap();
        let sequence = sequence.as_sequence().unwrap();
        for entry in [
            "foo@1.2.3",
            "foo@1.3.0",
            "@scope/pkg@2.0.0",
            "@scope/pkg@2.1.0",
        ] {
            assert!(release_age_excluded(sequence, entry), "{entry}");
        }
        for entry in [
            "foo@1.1.0",
            "foo@1.2.30",
            "foo@1.4.0",
            "@scope/other@2.1.0",
            "@scope/other@3.0.0",
        ] {
            assert!(!release_age_excluded(sequence, entry), "{entry}");
        }
    }

    #[test]
    fn display_name_strips_manifest_prefix() {
        let dir = std::env::temp_dir().join(format!("dsh-profile-name-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(manifest_display_name(&dir, "beta"), "Beta");
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"dsh-profile-beta","private":true}"#,
        )
        .unwrap();
        assert_eq!(manifest_display_name(&dir, "beta"), "Beta");
        std::fs::write(dir.join("package.json"), r#"{"name":"my-profile"}"#).unwrap();
        assert_eq!(manifest_display_name(&dir, "beta"), "My-profile");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn init_profile_dir_scaffolds_official_shape() {
        let dir = std::env::temp_dir().join(format!("dsh-profile-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        init_profile_dir(&dir, "beta").unwrap();

        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["name"], "dsh-profile-beta");
        assert_eq!(manifest["dependencies"], serde_json::json!({}));
        assert_eq!(
            manifest["dsh"]["profile"]["bundles"],
            serde_json::json!(["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"])
        );
        assert!(dir.join("cordis.patch.yml").is_file());
        assert!(dir.join("pnpm-workspace.yaml").is_file());
        let npmrc = std::fs::read_to_string(dir.join(".npmrc")).unwrap();
        assert!(npmrc.contains("confirmModulesPurge=false"));

        init_profile_dir(&dir, "beta").unwrap();
        let npmrc2 = std::fs::read_to_string(dir.join(".npmrc")).unwrap();
        assert_eq!(npmrc, npmrc2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn desktop_bootstrap_creates_official_shape_once() {
        let tmp = std::env::temp_dir().join(format!("dsh-profile-desktop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("profiles");

        ensure_desktop_profile_with_root(&root).unwrap();
        assert_eq!(DESKTOP_PROFILE, "tauri");
        let dir = root.join(DESKTOP_PROFILE);
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["name"], "dsh-profile-tauri");
        assert_eq!(
            manifest["dsh"]["profile"]["bundles"],
            serde_json::json!(["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"])
        );
        assert!(dir.join("cordis.patch.yml").is_file());
        assert!(dir.join("pnpm-workspace.yaml").is_file());
        let npmrc = std::fs::read_to_string(dir.join(".npmrc")).unwrap();
        assert!(npmrc.contains("confirmModulesPurge=false"));

        std::fs::write(dir.join("cordis.patch.yml"), "# user edit\n[]\n").unwrap();
        ensure_desktop_profile_with_root(&root).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("cordis.patch.yml")).unwrap(),
            "# user edit\n[]\n"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn temp_profiles_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dsh-profile-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    fn run_migration(root: &Path) {
        let _ = adopt_tauri_profile_dir(root);
        ensure_desktop_profile_with_root(root).unwrap();
    }

    #[test]
    fn legacy_desktop_profile_dir_renamed_to_tauri() {
        let tmp = temp_profiles_root("rename");
        let root = tmp.join("profiles");
        let legacy = root.join(LEGACY_DESKTOP_PROFILE);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(
            legacy.join("package.json"),
            r#"{"name":"dsh-profile-desktop","private":true,"dependencies":{"dsh-tauri":"link:/a"},"dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base","@deepseek-ai/dsh-web-app"]}}}"#,
        )
        .unwrap();
        std::fs::write(legacy.join("cordis.patch.yml"), "# user edit\n[]\n").unwrap();

        run_migration(&root);

        assert!(!legacy.exists(), "legacy desktop dir must be gone");
        let renamed = root.join(DESKTOP_PROFILE);
        assert!(
            renamed.is_dir(),
            "profile dir must move to {DESKTOP_PROFILE}"
        );
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(renamed.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["name"], "dsh-profile-tauri");
        assert_eq!(manifest["dependencies"]["dsh-tauri"], "link:/a");
        assert_eq!(
            std::fs::read_to_string(renamed.join("cordis.patch.yml")).unwrap(),
            "# user edit\n[]\n"
        );

        run_migration(&root);
        let again: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(renamed.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(again["name"], "dsh-profile-tauri");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn interrupted_desktop_profile_rename_finishes_on_next_start() {
        let tmp = temp_profiles_root("rename-resume");
        let root = tmp.join("profiles");
        let renamed = root.join(DESKTOP_PROFILE);
        std::fs::create_dir_all(&renamed).unwrap();
        std::fs::write(
            renamed.join("package.json"),
            r#"{"name":"dsh-profile-desktop","private":true,"dependencies":{},"dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base"]}}}"#,
        )
        .unwrap();

        run_migration(&root);

        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(renamed.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["name"], "dsh-profile-tauri");
        // 缺 web 层的老档案同时被补齐（issue #452 形态）
        assert_eq!(
            manifest["dsh"]["profile"]["bundles"],
            serde_json::json!(["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"])
        );
        assert!(renamed.join("cordis.patch.yml").is_file());
        assert!(renamed.join("pnpm-workspace.yaml").is_file());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn legacy_desktop_profile_never_overwrites_existing_tauri_profile() {
        let tmp = temp_profiles_root("conflict");
        let root = tmp.join("profiles");
        let legacy = root.join(LEGACY_DESKTOP_PROFILE);
        let target = root.join(DESKTOP_PROFILE);
        for dir in [&legacy, &target] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(
            legacy.join("package.json"),
            r#"{"name":"legacy","private":true}"#,
        )
        .unwrap();
        std::fs::write(
            target.join("package.json"),
            r#"{"name":"current","private":true,"dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base","@deepseek-ai/dsh-web-app"]}}}"#,
        )
        .unwrap();

        let adoption = adopt_tauri_profile_dir(&root);
        assert_eq!(adoption, ProfileDirAdoption::Blocked);

        assert!(legacy.join("package.json").is_file(), "old dir must stay");
        assert_eq!(
            std::fs::read_to_string(target.join("package.json")).unwrap(),
            r#"{"name":"current","private":true,"dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base","@deepseek-ai/dsh-web-app"]}}}"#,
            "existing target must stay untouched"
        );
        assert_eq!(
            std::fs::read_to_string(legacy.join("package.json")).unwrap(),
            r#"{"name":"legacy","private":true}"#
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn adoption_reports_success_and_is_idempotent() {
        let tmp = temp_profiles_root("adopt-idempotent");
        let root = tmp.join("profiles");

        assert_eq!(adopt_tauri_profile_dir(&root), ProfileDirAdoption::Adopted);

        let legacy = root.join(LEGACY_DESKTOP_PROFILE);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(
            legacy.join("package.json"),
            r#"{"name":"dsh-profile-desktop","private":true}"#,
        )
        .unwrap();
        assert_eq!(adopt_tauri_profile_dir(&root), ProfileDirAdoption::Adopted);
        assert_eq!(adopt_tauri_profile_dir(&root), ProfileDirAdoption::Adopted);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn failed_rename_is_reported_for_retry() {
        let tmp = temp_profiles_root("rename-fail");
        let root = tmp.join("profiles");
        let legacy = root.join(LEGACY_DESKTOP_PROFILE);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(
            legacy.join("package.json"),
            r#"{"name":"dsh-profile-desktop","private":true}"#,
        )
        .unwrap();
        std::fs::write(root.join(DESKTOP_PROFILE), "not a dir").unwrap();

        let adoption = adopt_tauri_profile_dir(&root);
        assert!(
            adoption == ProfileDirAdoption::Failed || adoption == ProfileDirAdoption::Blocked,
            "rename failure must be reported, got {adoption:?}"
        );
        assert!(
            legacy.join("package.json").is_file(),
            "legacy profile must stay"
        );
        assert_eq!(
            std::fs::read_to_string(legacy.join("package.json")).unwrap(),
            r#"{"name":"dsh-profile-desktop","private":true}"#,
            "legacy profile manifest must stay untouched"
        );

        std::fs::remove_file(root.join(DESKTOP_PROFILE)).unwrap();
        assert_eq!(adopt_tauri_profile_dir(&root), ProfileDirAdoption::Adopted);
        assert!(!legacy.exists(), "retry must finish the rename");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn manifest_name_rewrite_is_atomic_and_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("dsh-profile-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"dsh-profile-desktop","private":true,"dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base","@deepseek-ai/dsh-web-app"]}}}"#,
        )
        .unwrap();

        rewrite_manifest_name(&dir, DESKTOP_PROFILE).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["name"], "dsh-profile-tauri");
        assert_eq!(
            manifest["dsh"]["profile"]["bundles"],
            serde_json::json!(["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"])
        );
        let temps: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp"))
            .collect();
        assert!(temps.is_empty(), "temp files must be cleaned: {temps:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// issue #452 回归：`dsh plugin add` 会把缺 `package.json` 的目录按
    #[test]
    fn core_bundles_restored_before_plugin_entries_without_touching_other_fields() {
        let dir = std::env::temp_dir().join(format!("dsh-profile-core-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = serde_json::json!({
            "name": "dsh-profile-tauri",
            "private": true,
            "dependencies": { "dsh-tauri-pet": "link:C:/app/resources/node_modules/dsh-tauri-pet" },
            "dsh": { "profile": { "bundles": ["@deepseek-ai/dsh-base", "dsh-tauri-pet", "dsh-better-sidebar"] } }
        });
        std::fs::write(
            dir.join("package.json"),
            format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
        )
        .unwrap();

        assert!(ensure_profile_core_bundles(&dir, "tauri").unwrap());

        let repaired: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(
            repaired["dsh"]["profile"]["bundles"],
            serde_json::json!([
                "@deepseek-ai/dsh-base",
                "@deepseek-ai/dsh-web-app",
                "dsh-tauri-pet",
                "dsh-better-sidebar"
            ])
        );
        assert_eq!(repaired["name"], "dsh-profile-tauri");
        assert_eq!(
            repaired["dependencies"]["dsh-tauri-pet"],
            "link:C:/app/resources/node_modules/dsh-tauri-pet"
        );

        let before = std::fs::read_to_string(dir.join("package.json")).unwrap();
        assert!(!ensure_profile_core_bundles(&dir, "tauri").unwrap());
        assert_eq!(
            std::fs::read_to_string(dir.join("package.json")).unwrap(),
            before
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn core_bundles_disable_legacy_official_schedule_without_removing_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("package.json");
        for bundles in [
            serde_json::json!([
                "@deepseek-ai/dsh-base",
                "@deepseek-ai/dsh-web-app",
                "dsh-tauri-scheduler",
                "user-plugin",
                "@deepseek-ai/dsh-experimental-schedule-bundle"
            ]),
            serde_json::json!([
                "@deepseek-ai/dsh-base",
                "@deepseek-ai/dsh-web-app",
                "@deepseek-ai/dsh-experimental-schedule-bundle",
                "dsh-tauri-scheduler",
                "user-plugin",
                "@deepseek-ai/dsh-experimental-schedule-bundle"
            ]),
        ] {
            let mut expected = serde_json::json!({
                "name": "dsh-profile-tauri",
                "private": true,
                "dependencies": {
                    "@deepseek-ai/dsh-experimental-schedule-bundle": "0.2.0-rc.2",
                    "user-plugin": "1.0.0"
                },
                "dsh": { "profile": { "bundles": bundles, "custom": "keep" } }
            });
            std::fs::write(&path, serde_json::to_string(&expected).unwrap()).unwrap();
            expected["dsh"]["profile"]["bundles"] = serde_json::json!([
                "@deepseek-ai/dsh-base",
                "@deepseek-ai/dsh-web-app",
                "dsh-tauri-scheduler",
                "user-plugin"
            ]);

            assert!(ensure_profile_core_bundles(dir.path(), "tauri").unwrap());
            let repaired = std::fs::read_to_string(&path).unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&repaired).unwrap(),
                expected
            );
            assert!(!ensure_profile_core_bundles(dir.path(), "tauri").unwrap());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), repaired);
        }
    }

    #[test]
    fn core_bundles_repair_skips_non_web_surfaces() {
        let dir = std::env::temp_dir().join(format!("dsh-profile-surface-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let raw = r#"{"name":"dsh-profile-headless","private":true,"dependencies":{},"dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base","@deepseek-ai/dsh-headless","@deepseek-ai/dsh-experimental-schedule-bundle"]}}}"#;
        std::fs::write(dir.join("package.json"), raw).unwrap();

        assert!(!ensure_profile_core_bundles(&dir, "headless").unwrap());
        assert_eq!(
            std::fs::read_to_string(dir.join("package.json")).unwrap(),
            raw,
            "non-web surface profile must stay untouched"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn core_bundles_seeded_when_manifest_is_missing() {
        let dir = std::env::temp_dir().join(format!("dsh-profile-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!dir.join("package.json").exists());

        assert!(ensure_profile_core_bundles(&dir, "tauri").unwrap());

        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["name"], "dsh-profile-tauri");
        assert_eq!(manifest["private"], true);
        assert_eq!(manifest["dependencies"], serde_json::json!({}));
        assert_eq!(
            manifest["dsh"]["profile"]["bundles"],
            serde_json::json!(["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"])
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn core_bundles_repair_refuses_to_clobber_corrupt_manifest() {
        let dir = std::env::temp_dir().join(format!("dsh-profile-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for raw in [
            "{ not json",
            "[]",
            r#"{"dsh":"x"}"#,
            r#"{"dsh":{"profile":"y"}}"#,
        ] {
            std::fs::write(dir.join("package.json"), raw).unwrap();
            assert!(
                ensure_profile_core_bundles(&dir, "tauri").is_err(),
                "corrupt manifest {raw:?} must be reported, not rewritten"
            );
            assert_eq!(
                std::fs::read_to_string(dir.join("package.json")).unwrap(),
                raw,
                "corrupt manifest {raw:?} must stay untouched"
            );
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 首装引导必须修复「目录已存在但缺 web 层」的半初始化档案（issue #452 形态）。
    #[test]
    fn desktop_bootstrap_repairs_partially_initialized_profile() {
        let tmp = std::env::temp_dir().join(format!("dsh-profile-partial-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("profiles");
        let dir = root.join(DESKTOP_PROFILE);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"dsh-profile-tauri","private":true,"dependencies":{"dsh-tauri":"link:C:/app/resources/node_modules/dsh-tauri"},"dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base","dsh-tauri"]}}}"#,
        )
        .unwrap();

        ensure_desktop_profile_with_root(&root).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(
            manifest["dsh"]["profile"]["bundles"],
            serde_json::json!([
                "@deepseek-ai/dsh-base",
                "@deepseek-ai/dsh-web-app",
                "dsh-tauri"
            ])
        );
        assert!(dir.join("cordis.patch.yml").is_file());
        assert!(dir.join("pnpm-workspace.yaml").is_file());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn reset_wipes_profile_data_but_keeps_sessions() {
        let tmp = std::env::temp_dir().join(format!("dsh-profile-reset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let profiles_root = tmp.join("profiles");
        let sessions = tmp.join("sessions");

        init_profile_dir(&profiles_root.join("web"), "web").unwrap();
        std::fs::write(
            profiles_root.join("web").join("package.json"),
            r#"{"name":"dsh-profile-web","dependencies":{"some-plugin":"1.2.3"}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(sessions.join("_no-cwd")).unwrap();
        std::fs::write(sessions.join("_no-cwd").join("s1.json"), "{}").unwrap();

        reset_with_root(&profiles_root, "web").unwrap();

        let manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(profiles_root.join("web").join("package.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["name"], "dsh-profile-web");
        assert_eq!(manifest["dependencies"], serde_json::json!({}));
        assert_eq!(
            manifest["dsh"]["profile"]["bundles"],
            serde_json::json!(["@deepseek-ai/dsh-base", "@deepseek-ai/dsh-web-app"])
        );
        assert!(sessions.join("_no-cwd").join("s1.json").is_file());

        reset_with_root(&profiles_root, "fresh").unwrap();
        assert!(profiles_root.join("fresh").join("package.json").is_file());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn remove_rejects_path_traversal_ids() {
        for bad in ["..", ".", "../x", "/etc", "a/b", "..\\x", "a\\b"] {
            assert!(
                fs_guard::validate_id(bad).is_err(),
                "id {bad:?} 必须被字符集白名单拦截"
            );
        }
        for good in ["web", "my-profile", "dsh-1.2.3"] {
            assert!(fs_guard::validate_id(good).is_ok(), "id {good:?} 应合法");
        }
        let tmp = std::env::temp_dir().join(format!("dsh-profile-guard-{}", std::process::id()));
        let root = tmp.join("profiles");
        std::fs::create_dir_all(&root).unwrap();
        let res = std::panic::catch_unwind(|| {
            std::fs::create_dir_all(root.join("web")).unwrap();
            let ok = crate::service::fs_guard::safe_remove_target(&root, "web");
            assert!(ok.is_ok(), "存在的合法目录应通过守卫: {ok:?}");
            let bad = crate::service::fs_guard::safe_remove_target(&root, "..");
            assert!(bad.is_err(), "`..` 必须被拒绝");
        });
        let _ = std::fs::remove_dir_all(&tmp);
        assert!(res.is_ok(), "test panicked: {res:?}");
    }
}
