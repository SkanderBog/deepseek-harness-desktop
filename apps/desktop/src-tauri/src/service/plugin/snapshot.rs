//! 单插件快照：创建 / 读取 / 删除 / 还原插件的原子快照（issue #303）。

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::config;
use crate::service::fs_guard;

use super::installed::{is_installed, profile_dir};
use super::process::acquire_operation_lock;
use super::recovery::is_actionable_plugin_ref;
use crate::{AppHandle};

const SNAPSHOT_DIR_NAME: &str = ".plugin-backups";
const MANIFEST_NAME: &str = "manifest.json";
const PACKAGE_PREFIX: &str = "package";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotInfo {
    pub id: String,
    pub created: String,
    pub size: u64,
    pub include_config: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotResult {
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginBackupInfo {
    pub exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    pub size: u64,
    pub include_config: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotManifest {
    plugin_id: String,
    created: String,
    include_config: bool,
    spec: String,
    patches: Vec<String>,
    entry_count: usize,
    archive_size: u64,
}

fn snapshot_dir(app_handle: &AppHandle) -> PathBuf {
    let dir = config::get_dsh_data_path(app_handle).join(SNAPSHOT_DIR_NAME);
    fs::create_dir_all(&dir).ok();
    dir
}

fn snapshot_filename(id: &str) -> Result<String, String> {
    if id.trim().is_empty() {
        return Err("SNAPSHOT_INVALID_ID: 插件 id 为空".to_string());
    }
    if id.len() > 128 {
        return Err("SNAPSHOT_INVALID_ID: 插件 id 过长".to_string());
    }
    let sanitized: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    fs_guard::validate_id(&sanitized).map_err(|e| format!("SNAPSHOT_INVALID_ID: {e}"))?;
    if sanitized.is_empty() {
        return Err("SNAPSHOT_INVALID_ID: 插件 id 为空".to_string());
    }
    Ok(format!("{sanitized}.tgz"))
}

fn now_timestamp() -> String {
    use time::OffsetDateTime;
    let now = OffsetDateTime::now_utc();
    let date = now.date();
    let time = now.time();
    format!(
        "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}",
        date.year(),
        date.month() as u8,
        date.day(),
        time.hour(),
        time.minute(),
        time.second()
    )
}

fn resolve_real_target(node_modules: &Path, id: &str) -> Result<PathBuf, String> {
    let entry = node_modules.join(id);
    if !entry.exists() {
        return Err(format!(
            "SNAPSHOT_NOT_INSTALLED: {id} 未安装（{} 不存在）",
            entry.display()
        ));
    }
    let real = dunce::canonicalize(&entry).map_err(|e| format!("SNAPSHOT_RESOLVE_TARGET: {e}"))?;
    if !real.is_dir() {
        return Err(format!(
            "SNAPSHOT_NOT_DIR: {id} 的安装目标 {} 不是目录",
            real.display()
        ));
    }
    Ok(real)
}

fn count_tree(dir: &Path) -> (usize, u64) {
    let mut files = 0usize;
    let mut size = 0u64;
    let Ok(entries) = fs::read_dir(dir) else {
        return (files, size);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            let (f, s) = count_tree(&path);
            files += f;
            size += s;
        } else {
            files += 1;
            size += meta.len();
        }
    }
    (files, size)
}

fn append_package_tree(
    builder: &mut tar::Builder<impl Write>,
    dir: &Path,
    prefix: &Path,
) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("SNAPSHOT_READDIR: {e}"))? {
        let entry = entry.map_err(|e| format!("SNAPSHOT_ENTRY: {e}"))?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| format!("SNAPSHOT_METADATA: {e}"))?;
        if meta.file_type().is_symlink() {
            continue;
        }
        let rel = prefix.join(entry.file_name());
        if meta.is_dir() {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Directory);
            header.set_mode(0o755);
            header.set_size(0);
            header.set_cksum();
            builder
                .append_data(&mut header, &rel, std::io::empty())
                .map_err(|e| format!("SNAPSHOT_APPEND_DIR: {e}"))?;
            append_package_tree(builder, &path, &rel)?;
        } else {
            let file = fs::File::open(&path).map_err(|e| format!("SNAPSHOT_OPEN: {e}"))?;
            let mut header = tar::Header::new_gnu();
            header.set_size(meta.len());
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, &rel, file)
                .map_err(|e| format!("SNAPSHOT_APPEND_FILE: {e}"))?;
        }
    }
    Ok(())
}

fn write_archive_atomic(
    dest: &Path,
    write_fn: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let tmp = dest.with_extension("tmp");
    let _ = fs::remove_file(&tmp);
    write_fn(&tmp)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&tmp)
        .map_err(|e| format!("SNAPSHOT_OPEN_TMP: {e}"))?;
    file.sync_all()
        .map_err(|e| format!("SNAPSHOT_FSYNC: {e}"))?;
    fs::rename(&tmp, dest).map_err(|e| format!("SNAPSHOT_RENAME: {e}"))?;
    Ok(())
}

fn read_manifest(path: &Path) -> Result<SnapshotManifest, String> {
    let file = fs::File::open(path).map_err(|e| format!("SNAPSHOT_OPEN: {e}"))?;
    let dec = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(dec);
    for entry in archive
        .entries()
        .map_err(|e| format!("SNAPSHOT_LIST: {e}"))?
    {
        let mut entry = entry.map_err(|e| format!("SNAPSHOT_ENTRY: {e}"))?;
        let entry_path = entry
            .path()
            .map_err(|e| format!("SNAPSHOT_ENTRY_PATH: {e}"))?;
        if entry_path == Path::new(MANIFEST_NAME) {
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .map_err(|e| format!("SNAPSHOT_READ_MANIFEST: {e}"))?;
            return serde_json::from_slice(&buf)
                .map_err(|e| format!("SNAPSHOT_MANIFEST_PARSE: {e}"));
        }
    }
    Err("SNAPSHOT_MANIFEST_MISSING: 归档缺少 manifest.json".to_string())
}

fn count_archive_package(path: &Path) -> Result<(usize, u64), String> {
    let file = fs::File::open(path).map_err(|e| format!("SNAPSHOT_OPEN: {e}"))?;
    let dec = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(dec);
    let mut files = 0usize;
    let mut size = 0u64;
    for entry in archive
        .entries()
        .map_err(|e| format!("SNAPSHOT_LIST: {e}"))?
    {
        let entry = entry.map_err(|e| format!("SNAPSHOT_ENTRY: {e}"))?;
        let entry_path = entry
            .path()
            .map_err(|e| format!("SNAPSHOT_ENTRY_PATH: {e}"))?;
        if entry_path == Path::new(MANIFEST_NAME) {
            continue;
        }
        if entry.header().entry_type().is_file() {
            files += 1;
            size += entry.header().size().unwrap_or(0);
        }
    }
    Ok((files, size))
}

fn verify_archive_integrity(path: &Path, manifest: &SnapshotManifest) -> Result<(), String> {
    let (files, size) = count_archive_package(path)?;
    if files != manifest.entry_count || size != manifest.archive_size {
        return Err(format!(
            "SNAPSHOT_INTEGRITY: 归档包体 {files} 项 {size}B 与清单记录 {} 项 {}B 不一致",
            manifest.entry_count, manifest.archive_size
        ));
    }
    Ok(())
}

fn resolve_spec(app_handle: &AppHandle, id: &str) -> String {
    if let Some(preset) = super::preset::load_presets(app_handle)
        .iter()
        .find(|p| p.id == id)
    {
        return preset.spec.clone();
    }
    let manifest_path = profile_dir(app_handle).join("package.json");
    if let Ok(text) = fs::read_to_string(&manifest_path) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(spec) = value
                .get("dependencies")
                .and_then(|d| d.get(id))
                .and_then(|v| v.as_str())
            {
                return spec.to_string();
            }
        }
    }
    id.to_string()
}

pub fn create(app_handle: &AppHandle, id: &str) -> Result<SnapshotInfo, String> {
    let node_modules = profile_dir(app_handle).join("node_modules");
    let real = resolve_real_target(&node_modules, id)?;

    let (entry_count, archive_size) = count_tree(&real);
    let created = now_timestamp();
    let manifest = SnapshotManifest {
        plugin_id: id.to_string(),
        created: created.clone(),
        include_config: false,
        spec: resolve_spec(app_handle, id),
        patches: Vec::new(),
        entry_count,
        archive_size,
    };

    let dir = snapshot_dir(app_handle);
    let dest = dir.join(snapshot_filename(id)?);
    write_archive_atomic(&dest, |tmp| {
        let file = fs::File::create(tmp).map_err(|e| format!("SNAPSHOT_CREATE_FILE: {e}"))?;
        let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);
        let manifest_bytes = serde_json::to_vec(&manifest)
            .map_err(|e| format!("SNAPSHOT_MANIFEST_SERIALIZE: {e}"))?;
        let mut header = tar::Header::new_gnu();
        header.set_size(manifest_bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, MANIFEST_NAME, &manifest_bytes[..])
            .map_err(|e| format!("SNAPSHOT_APPEND_MANIFEST: {e}"))?;
        append_package_tree(&mut builder, &real, Path::new(PACKAGE_PREFIX))?;
        builder
            .finish()
            .map_err(|e| format!("SNAPSHOT_FINISH: {e}"))?;
        Ok(())
    })?;

    Ok(SnapshotInfo {
        id: id.to_string(),
        created,
        size: fs::metadata(&dest).map(|m| m.len()).unwrap_or(0),
        include_config: false,
    })
}

pub fn create_many(app_handle: &AppHandle, ids: &[String]) -> Vec<SnapshotResult> {
    ids.iter()
        .map(|id| match create(app_handle, id) {
            Ok(_) => SnapshotResult {
                id: id.clone(),
                ok: true,
                error: None,
            },
            Err(e) => SnapshotResult {
                id: id.clone(),
                ok: false,
                error: Some(e),
            },
        })
        .collect()
}

pub fn create_best_effort(app_handle: &AppHandle, id: &str) {
    if let Err(e) = create(app_handle, id) {
        log::warn!("SNAPSHOT_AUTO_FAILED: {id}: {e}");
    }
}

pub fn has_snapshot(app_handle: &AppHandle, id: &str) -> bool {
    let Ok(filename) = snapshot_filename(id) else {
        return false;
    };
    snapshot_dir(app_handle).join(filename).is_file()
}

pub fn get(app_handle: &AppHandle, id: &str) -> PluginBackupInfo {
    let Ok(filename) = snapshot_filename(id) else {
        return PluginBackupInfo {
            exists: false,
            created: None,
            size: 0,
            include_config: false,
        };
    };
    let path = snapshot_dir(app_handle).join(filename);
    if !path.is_file() {
        return PluginBackupInfo {
            exists: false,
            created: None,
            size: 0,
            include_config: false,
        };
    }
    let manifest = read_manifest(&path).ok();
    let created = manifest
        .as_ref()
        .map(|m| m.created.clone())
        .unwrap_or_else(now_timestamp);
    let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let include_config = manifest.is_some_and(|m| m.include_config);
    PluginBackupInfo {
        exists: true,
        created: Some(created),
        size,
        include_config,
    }
}

pub fn delete(app_handle: &AppHandle, id: &str) -> Result<(), String> {
    let filename = snapshot_filename(id)?;
    let path = snapshot_dir(app_handle).join(filename);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("SNAPSHOT_DELETE: {e}")),
    }
}

pub fn delete_best_effort(app_handle: &AppHandle, id: &str) {
    if let Err(e) = delete(app_handle, id) {
        log::warn!("SNAPSHOT_CASCADE_DELETE_FAILED: {id}: {e}");
    }
}

fn resolve_restore_target(node_modules: &Path, id: &str) -> Result<PathBuf, String> {
    if id != id.trim() || !is_actionable_plugin_ref(id) {
        return Err(format!("SNAPSHOT_INVALID_ID: {id}"));
    }
    let profile = node_modules
        .parent()
        .ok_or_else(|| "SNAPSHOT_NO_PARENT: node_modules 缺少父目录".to_string())?;
    let profile_real = dunce::canonicalize(profile)
        .map_err(|e| format!("SNAPSHOT_RESOLVE_PROFILE: {e}"))?;
    match fs::symlink_metadata(node_modules) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(node_modules).map_err(|e| format!("SNAPSHOT_MKDIR_MODULES: {e}"))?;
        }
        Err(e) => return Err(format!("SNAPSHOT_RESOLVE_MODULES: {e}")),
    }
    let root = fs_guard::ensure_within(node_modules, &profile_real)
        .map_err(|e| format!("SNAPSHOT_TARGET_ESCAPE: {e}"))?;
    if root == profile_real {
        return Err("SNAPSHOT_TARGET_ESCAPE: node_modules 指向档案根目录".to_string());
    }
    let entry = node_modules.join(id);
    let parent = entry.parent().ok_or_else(|| "SNAPSHOT_NO_PARENT: 插件缺少父目录".to_string())?;
    if fs::symlink_metadata(parent).is_ok() {
        fs_guard::ensure_within(parent, &root)
            .map_err(|e| format!("SNAPSHOT_TARGET_ESCAPE: {e}"))?;
    }
    match fs::symlink_metadata(&entry) {
        Ok(meta) => match dunce::canonicalize(&entry) {
            Ok(real) => {
                // issue #848：还原会删除旧目标，仅允许 node_modules 内的严格子目录。
                if real == root || !real.starts_with(&root) {
                    return Err(format!("SNAPSHOT_TARGET_ESCAPE: {}", real.display()));
                }
                if !real.is_dir() {
                    return Err(format!("SNAPSHOT_NOT_DIR: {}", real.display()));
                }
                if meta.file_type().is_symlink() {
                    Ok(real)
                } else {
                    Ok(entry)
                }
            }
            Err(e) if meta.file_type().is_symlink() && e.kind() == std::io::ErrorKind::NotFound => {
                #[cfg(windows)]
                let removal = {
                    use std::os::windows::fs::FileTypeExt;
                    if meta.file_type().is_symlink_dir() {
                        fs::remove_dir(&entry)
                    } else {
                        fs::remove_file(&entry)
                    }
                };
                #[cfg(not(windows))]
                let removal = fs::remove_file(&entry);
                removal.map_err(|e| format!("SNAPSHOT_REMOVE_BROKEN_LINK: {e}"))?;
                Ok(entry)
            }
            Err(e) => Err(format!("SNAPSHOT_RESOLVE_TARGET: {e}")),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(entry),
        Err(e) => Err(format!("SNAPSHOT_RESOLVE_TARGET: {e}")),
    }
}

fn write_back_manifest_refs_at(manifest_path: &Path, id: &str, spec: &str) {
    let Ok(text) = fs::read_to_string(manifest_path) else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return;
    };
    let version = if spec.is_empty() { "*" } else { spec };
    let mut modified = false;
    if value.get("dependencies").and_then(|d| d.get(id)).is_none() {
        if let Some(deps) = value
            .get_mut("dependencies")
            .and_then(|d| d.as_object_mut())
        {
            deps.insert(
                id.to_string(),
                serde_json::Value::String(version.to_string()),
            );
            modified = true;
        } else {
            value["dependencies"] = serde_json::json!({ id: version });
            modified = true;
        }
    }
    let bundled = value
        .get("dsh")
        .and_then(|d| d.get("profile"))
        .and_then(|p| p.get("bundles"))
        .and_then(|b| b.as_array())
        .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some(id)));
    if !bundled {
        let bundles = value
            .get_mut("dsh")
            .and_then(|d| d.get_mut("profile"))
            .and_then(|p| p.get_mut("bundles"))
            .and_then(|b| b.as_array_mut());
        match bundles {
            Some(arr) => {
                arr.push(serde_json::Value::String(id.to_string()));
                modified = true;
            }
            None => {
                if value.get("dsh").is_none() {
                    value["dsh"] = serde_json::json!({});
                }
                if value["dsh"].get("profile").is_none() {
                    value["dsh"]["profile"] = serde_json::json!({});
                }
                value["dsh"]["profile"]["bundles"] = serde_json::json!([id]);
                modified = true;
            }
        }
    }
    if !modified {
        return;
    }
    match serde_json::to_string_pretty(&value) {
        Ok(rendered) => {
            if let Err(e) = fs::write(manifest_path, format!("{rendered}\n")) {
                log::warn!("SNAPSHOT_WRITE_MANIFEST_FAILED: {id}: {e}");
            }
        }
        Err(e) => log::warn!("SNAPSHOT_RENDER_MANIFEST_FAILED: {id}: {e}"),
    }
}

fn write_back_manifest_refs(app_handle: &AppHandle, id: &str, spec: &str) {
    write_back_manifest_refs_at(&profile_dir(app_handle).join("package.json"), id, spec);
}

pub async fn restore(app_handle: &AppHandle, id: &str) -> Result<(), String> {
    if !is_actionable_plugin_ref(id) {
        return Err(format!(
            "SNAPSHOT_RESTORE_REFUSED: 核心/官方包 {id} 不允许还原"
        ));
    }
    let filename = snapshot_filename(id)?;
    let archive_path = snapshot_dir(app_handle).join(filename);
    if !archive_path.is_file() {
        return Err(format!("SNAPSHOT_NOT_FOUND: {id} 无快照"));
    }
    let manifest = read_manifest(&archive_path)?;
    verify_archive_integrity(&archive_path, &manifest)?;

    let _guard = acquire_operation_lock().await;
    if crate::service::workflow::has_owned_process() {
        if let Err(e) = crate::service::workflow::stop(app_handle.clone()).await {
            log::warn!("failed to stop harness before plugin restore: {e}");
        }
    }

    let profile = profile_dir(app_handle);
    let node_modules = profile.join("node_modules");
    let dest = resolve_restore_target(&node_modules, id)?;
    let staging_parent = dest
        .parent()
        .ok_or_else(|| "SNAPSHOT_NO_PARENT: 还原目标缺少父目录".to_string())?;
    fs::create_dir_all(staging_parent)
        .map_err(|e| format!("SNAPSHOT_MKDIR_STAGING_PARENT: {e}"))?;

    let token = format!("{}-{}", id.replace(['/', '\\'], "_"), std::process::id());
    let staging = staging_parent.join(format!(".staging-{token}"));
    let backup = staging_parent.join(format!(".backup-{token}"));
    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_dir_all(&backup);

    if let Err(e) = crate::service::backup::archive::extract_archive_gzip(&archive_path, &staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    let staged_pkg = staging.join(PACKAGE_PREFIX);
    let validate = (|| {
        if !staged_pkg.join("package.json").is_file() {
            return Err("SNAPSHOT_STAGING_INVALID: 暂存包缺少 package.json".to_string());
        }
        let text = fs::read_to_string(staged_pkg.join("package.json"))
            .map_err(|e| format!("SNAPSHOT_STAGING_READ: {e}"))?;
        serde_json::from_str::<serde_json::Value>(&text)
            .map_err(|e| format!("SNAPSHOT_STAGING_PARSE: package.json 无法解析: {e}"))?;
        Ok(())
    })();
    if let Err(e) = validate {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }

    let dest_existed = fs::symlink_metadata(&dest).is_ok();
    if dest_existed {
        if let Err(e) = fs::rename(&dest, &backup) {
            let _ = fs::remove_dir_all(&staging);
            return Err(format!("SNAPSHOT_SWITCH_BACKUP: {e}"));
        }
    }
    if let Err(e) = fs::rename(&staged_pkg, &dest) {
        let _ = fs::remove_dir_all(&dest);
        if dest_existed {
            let _ = fs::rename(&backup, &dest);
        }
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("SNAPSHOT_SWITCH_STAGE: {e}"));
    }
    if !dest.join("package.json").is_file() {
        let _ = fs::remove_dir_all(&dest);
        if dest_existed {
            let _ = fs::rename(&backup, &dest);
        }
        let _ = fs::remove_dir_all(&staging);
        return Err("SNAPSHOT_VERIFY_FAILED: 还原后 package.json 缺失".to_string());
    }
    let _ = fs::remove_dir_all(&backup);
    let _ = fs::remove_dir_all(&staging);

    if !is_installed(app_handle, id) {
        write_back_manifest_refs(app_handle, id, &manifest.spec);
    }
    if let Err(e) = fs::remove_file(profile.join("pnpm-lock.yaml")) {
        if e.kind() != std::io::ErrorKind::NotFound {
            log::warn!("failed to remove pnpm-lock.yaml after plugin restore: {e}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_package(root: &Path, name: &str) {
        let pkg = root.join("node_modules").join(name);
        fs::create_dir_all(&pkg).unwrap();
        fs::write(
            pkg.join("package.json"),
            format!(r#"{{"name":"{name}","version":"1.0.0","main":"lib/index.js"}}"#),
        )
        .unwrap();
        fs::create_dir_all(pkg.join("lib")).unwrap();
        fs::write(pkg.join("lib").join("index.js"), "module.exports = {};").unwrap();
    }

    #[test]
    fn filename_sanitizes_scoped_and_rejects_traversal() {
        assert_eq!(snapshot_filename("dsh-market").unwrap(), "dsh-market.tgz");
        assert_eq!(snapshot_filename("@scope/pkg").unwrap(), "_scope_pkg.tgz");
        assert!(snapshot_filename("..").is_err());
        assert!(snapshot_filename("").is_err());
        assert!(snapshot_filename("../x").is_err());
        assert!(snapshot_filename("a/b").is_ok()); // 净化后合法
    }

    #[test]
    fn count_tree_skips_symlinks() {
        let dir = std::env::temp_dir().join(format!("dsh-snap-count-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("a.txt"), "hello").unwrap();
        fs::write(dir.join("sub").join("b.txt"), "world").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("a.txt"), dir.join("link.txt")).unwrap();
        }
        let (files, size) = count_tree(&dir);
        assert_eq!(files, 2);
        assert_eq!(size, 10);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_round_trip_creates_and_reads_manifest() {
        let dir = std::env::temp_dir().join(format!("dsh-snap-rt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        setup_package(&dir, "dsh-market");
        let real = resolve_real_target(&dir.join("node_modules"), "dsh-market").unwrap();
        let (entry_count, archive_size) = count_tree(&real);
        let manifest = SnapshotManifest {
            plugin_id: "dsh-market".to_string(),
            created: "2026-08-30T12-00-00".to_string(),
            include_config: false,
            spec: "dsh-market".to_string(),
            patches: Vec::new(),
            entry_count,
            archive_size,
        };
        let archive = dir.join("snap.tgz");
        write_archive_atomic(&archive, |tmp| {
            let file = fs::File::create(tmp).unwrap();
            let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut builder = tar::Builder::new(enc);
            let bytes = serde_json::to_vec(&manifest).unwrap();
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, MANIFEST_NAME, &bytes[..])
                .unwrap();
            append_package_tree(&mut builder, &real, Path::new(PACKAGE_PREFIX)).unwrap();
            builder.finish().unwrap();
            Ok(())
        })
        .unwrap();

        let read = read_manifest(&archive).unwrap();
        assert_eq!(read.plugin_id, "dsh-market");
        assert_eq!(read.entry_count, entry_count);
        assert_eq!(read.archive_size, archive_size);
        verify_archive_integrity(&archive, &read).unwrap();

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_round_trip_restores_package_content() {
        let dir = std::env::temp_dir().join(format!("dsh-snap-ext-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        setup_package(&dir, "dsh-market");
        let real = resolve_real_target(&dir.join("node_modules"), "dsh-market").unwrap();
        let (entry_count, archive_size) = count_tree(&real);
        let manifest = SnapshotManifest {
            plugin_id: "dsh-market".to_string(),
            created: "2026-08-30T12-00-00".to_string(),
            include_config: false,
            spec: "dsh-market".to_string(),
            patches: Vec::new(),
            entry_count,
            archive_size,
        };
        let archive = dir.join("snap.tgz");
        write_archive_atomic(&archive, |tmp| {
            let file = fs::File::create(tmp).unwrap();
            let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut builder = tar::Builder::new(enc);
            let bytes = serde_json::to_vec(&manifest).unwrap();
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, MANIFEST_NAME, &bytes[..])
                .unwrap();
            append_package_tree(&mut builder, &real, Path::new(PACKAGE_PREFIX)).unwrap();
            builder.finish().unwrap();
            Ok(())
        })
        .unwrap();

        let target = dir.join("restored");
        crate::service::backup::archive::extract_archive_gzip(&archive, &target).unwrap();
        let pkg = target.join(PACKAGE_PREFIX);
        assert!(pkg.join("package.json").is_file());
        assert!(pkg.join("lib").join("index.js").is_file());
        let text = fs::read_to_string(pkg.join("package.json")).unwrap();
        assert!(text.contains("dsh-market"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_restore_target_handles_real_dir_and_missing() {
        let dir = std::env::temp_dir().join(format!("dsh-snap-tgt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("node_modules")).unwrap();
        setup_package(&dir, "pkg-a");
        let t = resolve_restore_target(&dir.join("node_modules"), "pkg-a").unwrap();
        assert_eq!(t, dir.join("node_modules").join("pkg-a"));
        let t2 = resolve_restore_target(&dir.join("node_modules"), "pkg-b").unwrap();
        assert_eq!(t2, dir.join("node_modules").join("pkg-b"));
        let _ = fs::remove_dir_all(&dir);
    }

    fn link_directory(target: &Path, link: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let status = std::process::Command::new("cmd")
                .arg("/C")
                .raw_arg(format!("mklink /J \"{}\" \"{}\"", link.display(), target.display()))
                .creation_flags(0x08000000)
                .status()
                .unwrap();
            assert!(status.success());
        }
    }

    #[test]
    fn restore_target_rejects_links_to_home_and_node_modules_root() {
        let dir = std::env::temp_dir().join(format!("dsh-snap-escape-{}", std::process::id()));
        fs::create_dir_all(dir.join("profiles/web/node_modules")).unwrap();
        fs::create_dir_all(dir.join("sessions")).unwrap();
        fs::write(dir.join("sessions/session.jsonl"), "turn 484").unwrap();
        let node_modules = dir.join("profiles/web/node_modules");
        link_directory(&dir, &node_modules.join("home-alias"));
        link_directory(&node_modules, &node_modules.join("root-alias"));
        let home_result = resolve_restore_target(&node_modules, "home-alias");
        let root_result = resolve_restore_target(&node_modules, "root-alias");
        assert_eq!(fs::read_to_string(dir.join("sessions/session.jsonl")).unwrap(), "turn 484");
        fs::remove_dir_all(&dir).unwrap();
        assert!(home_result.unwrap_err().starts_with("SNAPSHOT_TARGET_ESCAPE:"));
        assert!(root_result.unwrap_err().starts_with("SNAPSHOT_TARGET_ESCAPE:"));
    }

    #[test]
    fn restore_target_rejects_scoped_parent_link_escape() {
        let dir = std::env::temp_dir().join(format!("dsh-snap-scope-{}", std::process::id()));
        let node_modules = dir.join("profile/node_modules");
        let outside = dir.join("outside");
        fs::create_dir_all(&node_modules).unwrap();
        fs::create_dir_all(outside.join("pkg")).unwrap();
        link_directory(&outside, &node_modules.join("@scope"));
        let existing = resolve_restore_target(&node_modules, "@scope/pkg");
        let missing = resolve_restore_target(&node_modules, "@scope/missing");
        fs::remove_dir_all(&dir).unwrap();
        assert!(existing.unwrap_err().starts_with("SNAPSHOT_TARGET_ESCAPE:"));
        assert!(missing.unwrap_err().starts_with("SNAPSHOT_TARGET_ESCAPE:"));
    }

    #[test]
    fn restore_target_accepts_pnpm_link_within_node_modules() {
        let dir = std::env::temp_dir().join(format!("dsh-snap-pnpm-{}", std::process::id()));
        let node_modules = dir.join("node_modules");
        let package = node_modules.join(".pnpm/pkg@1.0.0/node_modules/pkg");
        fs::create_dir_all(&package).unwrap();
        link_directory(&package, &node_modules.join("pkg"));
        let expected = dunce::canonicalize(&package).unwrap();
        let result = resolve_restore_target(&node_modules, "pkg");
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn write_back_manifest_refs_restores_deps_and_bundles() {
        let dir = std::env::temp_dir().join(format!("dsh-snap-wb-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let manifest_path = dir.join("package.json");
        fs::write(
            &manifest_path,
            r#"{"name":"dsh-profile-web","dependencies":{},"dsh":{"profile":{"bundles":[]}}}"#,
        )
        .unwrap();

        write_back_manifest_refs_at(&manifest_path, "dsh-market", "dsh-market");
        let content = fs::read_to_string(&manifest_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(value["dependencies"]["dsh-market"], "dsh-market");
        assert_eq!(value["dsh"]["profile"]["bundles"][0], "dsh-market");

        write_back_manifest_refs_at(&manifest_path, "dsh-market", "dsh-market");
        let content = fs::read_to_string(&manifest_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(value["dependencies"].as_object().unwrap().len(), 1);
        assert_eq!(
            value["dsh"]["profile"]["bundles"].as_array().unwrap().len(),
            1
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_back_manifest_refs_creates_sections_when_missing() {
        let dir = std::env::temp_dir().join(format!("dsh-snap-wb2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let manifest_path = dir.join("package.json");
        fs::write(&manifest_path, r#"{"name":"dsh-profile-web"}"#).unwrap();

        write_back_manifest_refs_at(&manifest_path, "@scope/pkg", "");
        let content = fs::read_to_string(&manifest_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(value["dependencies"]["@scope/pkg"], "*");
        assert_eq!(value["dsh"]["profile"]["bundles"][0], "@scope/pkg");

        let _ = fs::remove_dir_all(&dir);
    }
}
