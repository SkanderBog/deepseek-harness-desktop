//! Harness 数据目录（`DSH_HOME`）的查看、迁移与回滚（issue #871）。
//! （issue #848：联接被递归删除会删掉目标里的真实数据）。

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use tauri::{Emitter as _};

mod env;
mod fs_ops;
mod migrate;
mod state;
#[cfg(unix)]
mod unix_env;
use crate::{AppHandle};

/// issue #1195：Windows 启动器可能继承旧环境，须在解析数据目录前恢复持久化值。
pub fn restore_process_env() {
    #[cfg(windows)]
    env::restore_process_env(env::read_user_home());
    #[cfg(unix)]
    unix_env::restore_process_env();
}

pub const PROGRESS_EVENT: &str = "data-dir://progress";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataDirStatus {
    pub supported: bool,
    pub data_dir: String,
    pub default_dir: String,
    pub env_override: String,
    pub rollback_available: bool,
    pub backups: Vec<MigrationBackup>,
    pub debug_build: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationBackup {
    pub path: String,
    pub stamp: String,
    pub sessions: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataDirEntry {
    pub name: String,
    pub path: String,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPlan {
    pub source: String,
    pub target: String,
    pub total_files: u64,
    pub total_bytes: u64,
    pub links: u64,
    pub target_free_bytes: u64,
    pub enough_space: bool,
    pub parent_exists: bool,
    pub remembered: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationOutcome {
    pub source: String,
    pub target: String,
    pub moved_to: String,
    pub files: u64,
    pub bytes: u64,
    pub links: u64,
    pub sessions: u64,
}

pub(super) type ProgressSink<'a> = &'a (dyn Fn(ProgressPayload) + Send + Sync);

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressPayload {
    pub operation: &'static str,
    pub phase: &'static str,
    pub copied_files: u64,
    pub copied_bytes: u64,
    pub total_files: u64,
    pub total_bytes: u64,
}

pub fn status(app_handle: &AppHandle) -> DataDirStatus {
    let data_dir = crate::config::get_dsh_data_path(app_handle);
    let default_dir = env::default_home();
    let env_override = env::read_user_home().ok().flatten().unwrap_or_default();
    let backups: Vec<MigrationBackup> = migrate::find_backups(app_handle, &data_dir)
        .into_iter()
        .collect();
    let rollback_available = !backups.is_empty();
    DataDirStatus {
        supported: !cfg!(debug_assertions),
        data_dir: data_dir.to_string_lossy().into_owned(),
        default_dir: default_dir.to_string_lossy().into_owned(),
        env_override,
        rollback_available,
        backups,
        debug_build: cfg!(debug_assertions),
    }
}

pub fn entries(app_handle: &AppHandle) -> Result<Vec<DataDirEntry>, String> {
    entries_in(&crate::config::get_dsh_data_path(app_handle))
}

fn entries_in(root: &Path) -> Result<Vec<DataDirEntry>, String> {
    let read_dir = match std::fs::read_dir(root) {
        Ok(read_dir) => read_dir,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("DATA_DIR_READ: {}: {error}", root.display())),
    };
    let mut candidates: Vec<DataDirEntry> = Vec::new();
    for entry in read_dir {
        let Ok(entry) = entry else { continue };
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let mut stats = fs_ops::TreeStats::default();
        fs_ops::scan_tree(&entry.path(), &mut stats)?;
        candidates.push(DataDirEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path().to_string_lossy().into_owned(),
            files: stats.files,
            bytes: stats.bytes,
        });
    }
    candidates.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
    Ok(candidates)
}

pub fn preview(app_handle: &AppHandle, parent: &Path, leaf: &str) -> Result<MigrationPlan, String> {
    let source = crate::config::get_dsh_data_path(app_handle);
    let target = join_leaf(parent, leaf)?;
    migrate::plan(app_handle, &source, &target)
}

pub async fn migrate(
    app_handle: AppHandle,
    parent: PathBuf,
    leaf: String,
) -> Result<MigrationOutcome, String> {
    let source = crate::config::get_dsh_data_path(&app_handle);
    let target = join_leaf(&parent, &leaf)?;
    migrate::plan(&app_handle, &source, &target)?;
    let handle = app_handle.clone();
    let emit = move |payload: ProgressPayload| {
        let _ = handle.emit(PROGRESS_EVENT, payload);
    };
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || migrate::run(&app, &source, &target, &emit))
        .await
        .map_err(|e| format!("DATA_DIR_TASK: {e}"))?
}

pub async fn rollback(
    app_handle: AppHandle,
    backup: Option<PathBuf>,
) -> Result<MigrationOutcome, String> {
    let current = crate::config::get_dsh_data_path(&app_handle);
    let handle = app_handle.clone();
    let emit = move |payload: ProgressPayload| {
        let _ = handle.emit(PROGRESS_EVENT, payload);
    };
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        migrate::rollback(&app, &current, backup.as_deref(), &emit)
    })
    .await
    .map_err(|e| format!("DATA_DIR_TASK: {e}"))?
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HarnessMarker {
    Missing,
    Invalid,
    Pid(u32),
}

pub(crate) fn harness_markers(root: &Path) -> [HarnessMarker; 2] {
    crate::config::HARNESS_PID_MARKER_NAMES.map(|name| harness_marker_at(&root.join(name)))
}

pub(crate) fn all_harness_markers(root: &Path) -> Vec<HarnessMarker> {
    let mut markers = harness_markers(root).to_vec();
    match std::fs::read_dir(root.join("profiles")) {
        Ok(entries) => {
            for entry in entries {
                match entry {
                    Ok(entry) if entry.path().is_dir() => {
                        markers.extend(harness_markers(&entry.path()))
                    }
                    Ok(_) => {}
                    Err(_) => markers.push(HarnessMarker::Invalid),
                }
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(_) => markers.push(HarnessMarker::Invalid),
    }
    markers
}

fn harness_marker_at(marker: &Path) -> HarnessMarker {
    let text = match std::fs::read_to_string(marker) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return HarnessMarker::Missing,
        Err(error) => {
            log::warn!("DATA_DIR_HARNESS_MARKER: {error}");
            return HarnessMarker::Invalid;
        }
    };
    let Some(pid) = text
        .lines()
        .next()
        .and_then(|line| line.trim().parse::<u32>().ok())
    else {
        log::warn!("DATA_DIR_HARNESS_MARKER: 无法解析 {}", marker.display());
        return HarnessMarker::Invalid;
    };
    HarnessMarker::Pid(pid)
}

pub(super) fn harness_stopped(root: &Path) -> bool {
    all_harness_markers(root)
        .into_iter()
        .all(|marker| match marker {
            HarnessMarker::Missing => true,
            HarnessMarker::Invalid => false,
            HarnessMarker::Pid(pid) => !process_alive(pid),
        })
}

pub(crate) fn process_alive(pid: u32) -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let mut system = System::new();
    let target = [Pid::from_u32(pid)];
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&target),
        true,
        ProcessRefreshKind::nothing(),
    );
    system.process(Pid::from_u32(pid)).is_some()
}

fn join_leaf(parent: &Path, leaf: &str) -> Result<PathBuf, String> {
    let trimmed = leaf.trim();
    if trimmed.is_empty() {
        return Err(format!("DATA_DIR_NAME_INVALID: {leaf}"));
    }
    let allowed = trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'));
    if !allowed {
        return Err(format!("DATA_DIR_NAME_INVALID: {leaf}"));
    }
    if parent.as_os_str().is_empty() {
        return Err(format!("DATA_DIR_PARENT_MISSING: {leaf}"));
    }
    Ok(parent.join(trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_markers_block_migration_when_live_or_unreadable() {
        let root = std::env::temp_dir().join(format!("dsh-profile-marker-{}", std::process::id()));
        let profile = root.join("profiles").join("isolation-check");
        std::fs::create_dir_all(&profile).unwrap();
        let marker = profile.join(crate::config::HARNESS_PID_MARKER_NAMES[0]);
        std::fs::write(&marker, format!("{}\n41000\n", std::process::id())).unwrap();
        assert!(!harness_stopped(&root));
        assert!(all_harness_markers(&root).contains(&HarnessMarker::Pid(std::process::id())));
        std::fs::write(&marker, "invalid").unwrap();
        assert!(!harness_stopped(&root));
        std::fs::remove_file(marker).unwrap();
        assert!(harness_stopped(&root));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn join_leaf_accepts_plain_names_only() {
        let parent = Path::new("D:\\");
        assert_eq!(
            join_leaf(parent, "DSHHome").unwrap(),
            parent.join("DSHHome")
        );
        assert_eq!(
            join_leaf(parent, " DSH Home ").unwrap(),
            parent.join("DSH Home")
        );
        assert!(join_leaf(parent, "D:\\evil").is_err());
        assert!(join_leaf(parent, "a/b").is_err());
        assert!(join_leaf(parent, "a\\b").is_err());
        assert!(join_leaf(parent, "   ").is_err());
        assert!(join_leaf(Path::new(""), "DSHHome").is_err());
    }

    #[test]
    fn process_alive_reports_the_current_process() {
        assert!(process_alive(std::process::id()));
    }

    #[test]
    fn harness_marker_tells_missing_from_unreadable() {
        let root = temp_dir("marker");
        let marker = root.join(".harness.pid");
        assert_eq!(harness_marker_at(&marker), HarnessMarker::Missing);

        std::fs::write(&marker, "4242\n3080\n").unwrap();
        assert_eq!(harness_marker_at(&marker), HarnessMarker::Pid(4242));

        std::fs::write(&marker, "not-a-pid\n3080\n").unwrap();
        assert_eq!(harness_marker_at(&marker), HarnessMarker::Invalid);
        std::fs::write(&marker, "").unwrap();
        assert_eq!(harness_marker_at(&marker), HarnessMarker::Invalid);

        std::fs::remove_file(&marker).unwrap();
        std::fs::create_dir(&marker).unwrap();
        assert_eq!(harness_marker_at(&marker), HarnessMarker::Invalid);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn migration_requires_both_channels_to_be_stopped() {
        let root = temp_dir("channels");
        assert_eq!(harness_markers(&root), [HarnessMarker::Missing; 2]);
        assert!(harness_stopped(&root));
        for (index, name) in [".harness.pid", ".harness-nightly.pid"]
            .into_iter()
            .enumerate()
        {
            let path = root.join(name);
            std::fs::write(&path, format!("{}\n3080\n", std::process::id())).unwrap();
            let mut expected = [HarnessMarker::Missing; 2];
            expected[index] = HarnessMarker::Pid(std::process::id());
            assert_eq!(harness_markers(&root), expected);
            assert!(!harness_stopped(&root));
            std::fs::write(&path, "broken\n").unwrap();
            assert!(!harness_stopped(&root));
            std::fs::write(&path, format!("{}\n3080\n", u32::MAX)).unwrap();
            assert!(harness_stopped(&root));
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn entries_treat_a_missing_directory_as_empty() {
        let root = temp_dir("entries-missing");
        let _ = std::fs::remove_dir_all(&root);
        assert!(entries_in(&root).unwrap().is_empty());
        let file = root.with_extension("txt");
        std::fs::write(&file, "not a directory").unwrap();
        assert!(entries_in(&file)
            .unwrap_err()
            .starts_with("DATA_DIR_READ: "));
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn entries_lists_directories_largest_first() {
        let root = temp_dir("entries-order");
        write(&root.join("small").join("one.bin"), "abc");
        write(&root.join("big").join("one.bin"), "abcdef");
        write(&root.join("big").join("two.bin"), "abcdef");
        write(&root.join("loose.txt"), "x");

        let listed = entries_in(&root).unwrap();
        let names: Vec<&str> = listed.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, vec!["big", "small"]);
        assert_eq!((listed[0].files, listed[0].bytes), (2, 12));
        assert_eq!((listed[1].files, listed[1].bytes), (1, 3));
        let _ = std::fs::remove_dir_all(&root);
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsh-data-dir-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, text).unwrap();
    }
}
