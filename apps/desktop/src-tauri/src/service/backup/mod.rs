pub mod archive;
pub mod recovery;
pub mod retention;

use std::fs;
use std::path::{Path, PathBuf};

use crate::AppHandle;
use crate::config;

#[derive(Debug, Clone)]
pub struct BackupOptions {
    pub include_credentials: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub timestamp: String,
    pub path: String,
    pub size: u64,
    pub include_credentials: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum RestoreMode {
    Overwrite,
    AsNew,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct BackupManifest {
    backups: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestEntry {
    timestamp: String,
    profile: String,
    path: String,
    size: u64,
    include_credentials: bool,
}

pub fn get_backup_dir(app_handle: &AppHandle) -> PathBuf {
    let dir = config::get_dsh_data_path(app_handle).join(".backups");
    fs::create_dir_all(&dir).ok();
    dir
}

fn archive_filename(profile: &str, timestamp: &str) -> String {
    format!("{profile}-{timestamp}.tar.zst")
}

#[derive(Debug)]
enum ManifestError {
    ParseError(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::ParseError(e) => write!(f, "manifest parse error: {e}"),
        }
    }
}

impl std::error::Error for ManifestError {}

fn read_manifest(backup_dir: &Path) -> Result<BackupManifest, ManifestError> {
    let path = backup_dir.join(".manifest.json");
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Ok(BackupManifest { backups: vec![] }),
    };
    if content.trim().is_empty() {
        return Err(ManifestError::ParseError("empty manifest".into()));
    }
    match serde_json::from_str(&content) {
        Ok(manifest) => Ok(manifest),
        Err(e) => {
            log::error!("BACKUP_MANIFEST_PARSE: {} {e}", path.display());
            let _ = fs::rename(&path, path.with_extension("corrupt"));
            Err(ManifestError::ParseError(e.to_string()))
        }
    }
}

fn write_manifest(backup_dir: &Path, manifest: &BackupManifest) -> Result<(), String> {
    let path = backup_dir.join(".manifest.json");
    let tmp_name = format!(".manifest.{}.{}.tmp", std::process::id(), timestamp_nanos());
    let tmp = backup_dir.join(tmp_name);
    let content = serde_json::to_string_pretty(manifest)
        .map_err(|e| format!("BACKUP_MANIFEST_SERIALIZE: {e}"))?;
    fs::write(&tmp, content).map_err(|e| format!("BACKUP_MANIFEST_WRITE: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("BACKUP_MANIFEST_RENAME: {e}"))?;
    Ok(())
}

fn timestamp_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn now_timestamp() -> String {
    use time::OffsetDateTime;
    let now = OffsetDateTime::now_utc();
    let date = now.date();
    let time = now.time();
    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}",
        date.year(),
        date.month() as u8,
        date.day(),
        time.hour(),
        time.minute(),
        time.second()
    )
}

fn timestamp_order(timestamp: &str) -> (&str, u64) {
    match timestamp.rsplit_once('-') {
        Some((base, suffix)) if base.len() == 14 && base.bytes().all(|b| b.is_ascii_digit()) => {
            match suffix.parse() {
                Ok(sequence) => (base, sequence),
                Err(_) => (timestamp, 1),
            }
        }
        _ => (timestamp, 1),
    }
}

fn collision_safe_timestamp(backup_dir: &Path, timestamp: &str) -> Result<String, String> {
    let marker = format!("-{timestamp}");
    let mut latest = 0_u64;
    for entry in fs::read_dir(backup_dir).map_err(|e| format!("BACKUP_READDIR: {e}"))? {
        let name = entry.map_err(|e| format!("BACKUP_ENTRY: {e}"))?.file_name();
        let Some(stem) = name.to_str().and_then(|name| name.strip_suffix(".tar.zst")) else {
            continue;
        };
        let Some((_, suffix)) = stem.rsplit_once(&marker) else {
            continue;
        };
        let sequence = if suffix.is_empty() {
            1
        } else if let Some(sequence) = suffix
            .strip_prefix('-')
            .and_then(|value| value.parse::<u64>().ok())
        {
            sequence
        } else {
            continue;
        };
        latest = latest.max(sequence);
    }
    if latest == 0 {
        return Ok(timestamp.to_string());
    }
    let next = latest.checked_add(1).ok_or("BACKUP_SEQUENCE_EXHAUSTED")?;
    Ok(format!("{timestamp}-{next}"))
}

static BACKUP_OPERATION: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn create_backup(app_handle: &AppHandle, options: BackupOptions) -> Result<BackupInfo, String> {
    let _operation = BACKUP_OPERATION
        .lock()
        .map_err(|e| format!("BACKUP_LOCK: {e}"))?;
    let backup_dir = get_backup_dir(app_handle);
    let active = crate::service::profile::active_profile(app_handle);
    let source = crate::service::profile::profile_dir_of(app_handle, &active);
    let timestamp = collision_safe_timestamp(&backup_dir, &now_timestamp())?;
    let dest = backup_dir.join(archive_filename(&active, &timestamp));

    if let Err(e) = archive::create_archive(&source, &dest, options.include_credentials) {
        let _ = fs::remove_file(&dest);
        return Err(e);
    }

    let size = fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);

    let info = BackupInfo {
        timestamp: timestamp.clone(),
        path: dest.to_string_lossy().into_owned(),
        size,
        include_credentials: options.include_credentials,
    };

    let mut manifest =
        read_manifest(&backup_dir).map_err(|e| format!("BACKUP_MANIFEST_CORRUPT: {e}"))?;
    manifest.backups.push(ManifestEntry {
        timestamp: info.timestamp.clone(),
        profile: active.clone(),
        path: info.path.clone(),
        size: info.size,
        include_credentials: info.include_credentials,
    });
    write_manifest(&backup_dir, &manifest)?;

    prune_if_needed(app_handle)?;

    Ok(info)
}

pub fn list_backups(app_handle: &AppHandle) -> Vec<BackupInfo> {
    let backup_dir = get_backup_dir(app_handle);
    let manifest = match read_manifest(&backup_dir) {
        Ok(m) => m,
        Err(e) => {
            log::error!("[backup] list_backups: manifest unreadable: {e}");
            return vec![];
        }
    };
    let active = crate::service::profile::active_profile(app_handle);
    manifest
        .backups
        .into_iter()
        .filter(|e| e.profile == active)
        .map(|e| BackupInfo {
            timestamp: e.timestamp,
            path: e.path,
            size: e.size,
            include_credentials: e.include_credentials,
        })
        .collect()
}

pub fn delete_backup(app_handle: &AppHandle, timestamp: &str) -> Result<(), String> {
    crate::service::fs_guard::validate_id(timestamp)?;
    let _operation = BACKUP_OPERATION
        .lock()
        .map_err(|e| format!("BACKUP_LOCK: {e}"))?;
    let backup_dir = get_backup_dir(app_handle);
    let active = crate::service::profile::active_profile(app_handle);
    let filename = archive_filename(&active, timestamp);
    let file = backup_dir.join(&filename);
    if file.exists() {
        fs::remove_file(&file).map_err(|e| format!("BACKUP_DELETE_FILE: {e}"))?;
    }
    let mut manifest = match read_manifest(&backup_dir) {
        Ok(m) => m,
        Err(e) => return Err(format!("BACKUP_MANIFEST_CORRUPT: {e}")),
    };
    manifest
        .backups
        .retain(|e| !(e.profile == active && e.timestamp == timestamp));
    write_manifest(&backup_dir, &manifest)?;
    Ok(())
}

pub fn restore_backup(
    app_handle: &AppHandle,
    timestamp: &str,
    mode: RestoreMode,
) -> Result<(), String> {
    crate::service::fs_guard::validate_id(timestamp)?;
    let _operation = BACKUP_OPERATION
        .lock()
        .map_err(|e| format!("BACKUP_LOCK: {e}"))?;
    let backup_dir = get_backup_dir(app_handle);
    let active = crate::service::profile::active_profile(app_handle);
    let filename = archive_filename(&active, timestamp);
    let archive_path = backup_dir.join(&filename);
    if !archive_path.exists() {
        return Err(format!(
            "BACKUP_NOT_FOUND: backup {filename} does not exist"
        ));
    }

    match mode {
        RestoreMode::Overwrite => {
            let dest = crate::service::profile::profile_dir_of(app_handle, &active);
            let backup_old =
                std::path::PathBuf::from(format!("{}-{}.restore-bak", dest.display(), timestamp));
            let _ = fs::remove_dir_all(&backup_old);
            fs::rename(&dest, &backup_old).map_err(|e| {
                format!(
                    "BACKUP_RESTORE_RENAME_OLD: {e} (old={}, new={})",
                    dest.display(),
                    backup_old.display()
                )
            })?;
            fs::create_dir_all(&dest)
                .map_err(|e| format!("BACKUP_RESTORE_MKDIR_NEW: {e} (dest={})", dest.display()))?;
            let extract_result = archive::extract_archive(&archive_path, &dest);
            match extract_result {
                Ok(()) => {
                    let _ = fs::remove_dir_all(&backup_old);
                }
                Err(e) => {
                    let _ = fs::remove_dir_all(&dest);
                    let _ = fs::rename(&backup_old, &dest);
                    return Err(format!(
                        "BACKUP_RESTORE_EXTRACT_FAILED: {e}。已自动回滚到原状态。"
                    ));
                }
            }
        }
        RestoreMode::AsNew => {
            let profiles_root = config::get_dsh_data_path(app_handle).join("profiles");
            fs::create_dir_all(&profiles_root)
                .map_err(|e| format!("BACKUP_RESTORE_MKDIR_PROFILES: {e}"))?;
            let new_dir = profiles_root.join(format!("{active}-{timestamp}"));
            match fs::create_dir(&new_dir) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(format!(
                        "BACKUP_RESTORE_TARGET_EXISTS: {}",
                        new_dir.display()
                    ));
                }
                Err(e) => return Err(format!("BACKUP_RESTORE_MKDIR_NEW: {e}")),
            }
            if let Err(e) = archive::extract_archive(&archive_path, &new_dir) {
                let _ = fs::remove_dir_all(&new_dir);
                return Err(format!("BACKUP_RESTORE_EXTRACT_FAILED: {e}"));
            }
        }
    }
    Ok(())
}

fn prune_if_needed(app_handle: &AppHandle) -> Result<(), String> {
    let setting = config::get_store_dat_setting(app_handle);
    retention::prune_old_backups(app_handle, setting.backup_retention_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn timestamp_format_is_valid() {
        let ts = now_timestamp();
        assert_eq!(ts.len(), 14, "时间戳长度应为 14: {ts}");
        assert!(
            ts.chars().all(|c| c.is_ascii_digit()),
            "时间戳应全为数字: {ts}"
        );
    }

    #[test]
    fn collision_safe_timestamp_matches_archive_filename() {
        let dir = std::env::temp_dir().join(format!("dsh-backup-collision-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let base = "2026-10-02T15-24-08";
        let first = collision_safe_timestamp(&dir, base).unwrap();
        assert_eq!(first, base, "无同名文件时应保持原时间戳");
        fs::write(dir.join(archive_filename("web", &first)), b"first").unwrap();

        let second = collision_safe_timestamp(&dir, base).unwrap();
        assert_ne!(second, first, "同名归档已存在时必须换一个时间戳");
        assert_eq!(second, format!("{base}-2"));
        assert!(
            !dir.join(archive_filename("web", &second)).exists(),
            "换出的时间戳必须是一个空闲文件名"
        );
        fs::write(dir.join(archive_filename("web", &second)), b"second").unwrap();
        assert_eq!(
            collision_safe_timestamp(&dir, base).unwrap(),
            format!("{base}-3"),
            "再次碰撞应继续递增"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn collision_timestamp_does_not_reuse_pruned_lower_suffixes() {
        let dir = std::env::temp_dir().join(format!(
            "dsh-backup-pruned-collision-{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let base = "20261011093345";
        for suffix in ["-9", "-10", "-12"] {
            fs::write(
                dir.join(archive_filename("web", &format!("{base}{suffix}"))),
                b"retained",
            )
            .unwrap();
        }
        fs::write(
            dir.join(archive_filename("other", &format!("{base}-99"))),
            b"other",
        )
        .unwrap();
        assert_eq!(
            collision_safe_timestamp(&dir, base).unwrap(),
            format!("{base}-100")
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn repeated_creation_and_retention_preserve_the_last_two_archives() {
        let dir =
            std::env::temp_dir().join(format!("dsh-backup-retention-loop-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        for index in 1..=20 {
            let timestamp = collision_safe_timestamp(&dir, "20261011093345").unwrap();
            let profile = if index % 2 == 0 { "web" } else { "tauri" };
            let path = dir.join(archive_filename(profile, &timestamp));
            fs::write(&path, index.to_string()).unwrap();
            let mut manifest = read_manifest(&dir).unwrap();
            manifest.backups.push(ManifestEntry {
                timestamp,
                profile: profile.into(),
                path: path.to_string_lossy().into_owned(),
                size: 0,
                include_credentials: false,
            });
            write_manifest(&dir, &manifest).unwrap();
            retention::prune_backups_in_dir(&dir, 2).unwrap();
            assert!(path.exists(), "latest archive {index} was pruned");
        }
        let manifest = read_manifest(&dir).unwrap();
        assert_eq!(
            manifest
                .backups
                .iter()
                .map(|entry| entry.timestamp.as_str())
                .collect::<Vec<_>>(),
            ["20261011093345-19", "20261011093345-20"]
        );
        assert_eq!(fs::read_to_string(&manifest.backups[0].path).unwrap(), "19");
        assert_eq!(fs::read_to_string(&manifest.backups[1].path).unwrap(), "20");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn collision_timestamp_rejects_directory_errors_and_sequence_overflow() {
        let dir = std::env::temp_dir().join(format!(
            "dsh-backup-collision-errors-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        assert!(
            collision_safe_timestamp(&dir, "20261011093345")
                .unwrap_err()
                .starts_with("BACKUP_READDIR:")
        );
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("web-20261011093345-18446744073709551615.tar.zst"),
            b"keep",
        )
        .unwrap();
        assert_eq!(
            collision_safe_timestamp(&dir, "20261011093345").unwrap_err(),
            "BACKUP_SEQUENCE_EXHAUSTED"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn create_archive_real_filesystem_roundtrip() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let suffix = COUNTER.fetch_add(1, Ordering::SeqCst);

        let tmp_root =
            std::env::temp_dir().join(format!("dsh-backup-int-{}-{}", std::process::id(), suffix));
        let _ = fs::remove_dir_all(&tmp_root);
        fs::create_dir_all(&tmp_root).unwrap();

        let source = tmp_root.join("source");
        fs::create_dir_all(&source).unwrap();
        for i in 0..20 {
            let mut f = fs::File::create(source.join(format!("file_{i}.txt"))).unwrap();
            f.write_all(format!("hello {i}").as_bytes()).unwrap();
        }

        let backup_dir = tmp_root.join(".backups");
        fs::create_dir_all(&backup_dir).unwrap();
        let dest = backup_dir.join(format!("test_{suffix}.tar.zst"));

        archive::create_archive(&source, &dest, false).unwrap();
        assert!(dest.exists(), "备份文件应已创建");
        let size = fs::metadata(&dest).unwrap().len();
        assert!(size > 0, "备份文件应非空");

        let restore_dir = tmp_root.join("restored");
        archive::extract_archive(&dest, &restore_dir).unwrap();

        for i in 0..20 {
            let content = fs::read_to_string(restore_dir.join(format!("file_{i}.txt"))).unwrap();
            assert_eq!(content, format!("hello {i}"), "文件 {i} 内容应一致");
        }

        let _ = fs::remove_dir_all(&tmp_root);
    }

    #[test]
    fn manifest_round_trip() {
        let dir = std::env::temp_dir().join(format!("dsh-backup-manifest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let manifest = BackupManifest {
            backups: vec![ManifestEntry {
                timestamp: "2026-08-30T12-00-00".to_string(),
                profile: "web".to_string(),
                path: "/tmp/x.tar.gz".to_string(),
                size: 100,
                include_credentials: false,
            }],
        };
        write_manifest(&dir, &manifest).unwrap();
        let read = read_manifest(&dir).unwrap();
        assert_eq!(read.backups.len(), 1);
        assert_eq!(read.backups[0].timestamp, "2026-08-30T12-00-00");
        assert_eq!(read.backups[0].size, 100);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_real_backup_to_temp() {
        let backup =
            std::path::Path::new("/Users/coderstory/.dsh/.backups/2026-08-31T15-04-18.tar.zst");
        if !backup.exists() {
            eprintln!("[skip] 备份文件不存在: {}", backup.display());
            return;
        }

        let dest = std::env::temp_dir().join("dsh-restore-real-test");
        let _ = fs::remove_dir_all(&dest);
        fs::create_dir_all(&dest).unwrap();

        let backup_size = fs::metadata(backup).unwrap().len();

        archive::extract_archive(backup, &dest).expect("extract_archive 失败");

        fn walk(
            p: &std::path::Path,
            files: &mut u64,
            dirs: &mut u64,
            links: &mut u64,
            size: &mut u64,
        ) {
            let Ok(rd) = fs::read_dir(p) else { return };
            for e in rd.flatten() {
                let Ok(meta) = fs::symlink_metadata(e.path()) else {
                    continue;
                };
                if meta.is_dir() {
                    *dirs += 1;
                    walk(&e.path(), files, dirs, links, size);
                } else if meta.file_type().is_symlink() {
                    *links += 1;
                } else if meta.is_file() {
                    *files += 1;
                    *size += meta.len();
                }
            }
        }
        let mut files = 0u64;
        let mut dirs = 0u64;
        let mut links = 0u64;
        let mut total_size = 0u64;
        walk(&dest, &mut files, &mut dirs, &mut links, &mut total_size);

        println!("\n========== 完整还原报告 ==========");
        println!("备份文件: {} ({} bytes)", backup.display(), backup_size);
        println!("还原目标: {}", dest.display());
        println!("还原统计:");
        println!("  文件:     {}", files);
        println!("  目录:     {}", dirs);
        println!("  符号链接: {}", links);
        println!(
            "  总大小:   {} bytes ({:.2} MB)",
            total_size,
            total_size as f64 / 1_048_576.0
        );

        for name in [
            "cordis.patch.yml",
            "package.json",
            "pnpm-workspace.yaml",
            ".npmrc",
        ] {
            let path = dest.join(name);
            if path.exists() {
                println!(
                    "  ✓ {} ({} bytes)",
                    name,
                    fs::metadata(&path).unwrap().len()
                );
            } else {
                println!("  ✗ {} 缺失", name);
            }
        }

        let nm = dest.join("node_modules");
        if nm.is_dir() {
            let mut nm_n = 0u64;
            walk(&nm, &mut nm_n, &mut 0, &mut 0, &mut 0);
            println!("  ✓ node_modules ({} 文件/目录)", nm_n);
        }

        println!(
            "\n压缩比: {:.2}x ({} → {} bytes)",
            total_size as f64 / backup_size as f64,
            total_size,
            backup_size
        );

        let _ = fs::remove_dir_all(&dest);
    }
}
