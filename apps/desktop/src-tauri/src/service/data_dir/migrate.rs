use std::fs;
use std::path::{Component, Path, PathBuf};

use super::env;
use super::fs_ops::{self, CopyProgress, TreeStats};
use super::state;
use super::{
    MigrationBackup, MigrationOutcome, MigrationPlan, ProgressPayload, ProgressSink,
    harness_stopped,
};
use crate::AppHandle;

const SPACE_MARGIN_PERCENT: u64 = 5;

pub(super) fn find_backups(app_handle: &AppHandle, current: &Path) -> Vec<MigrationBackup> {
    let default = env::default_home();
    let recorded = state::read_last_migration(app_handle).map(PathBuf::from);
    let mut backups: Vec<MigrationBackup> =
        collect_backups(&search_places(current, &default, recorded.as_deref()))
            .into_iter()
            .map(|path| MigrationBackup {
                sessions: fs_ops::count_sessions(&path),
                stamp: stamp_of(&path),
                path: path.to_string_lossy().into_owned(),
            })
            .collect();
    backups.sort_by(|a, b| {
        b.sessions
            .cmp(&a.sessions)
            .then_with(|| b.stamp.cmp(&a.stamp))
    });
    backups
}

fn search_places(current: &Path, default: &Path, recorded: Option<&Path>) -> Vec<PathBuf> {
    let mut places: Vec<PathBuf> = Vec::new();
    for path in [Some(current), Some(default), recorded]
        .into_iter()
        .flatten()
    {
        if !places.iter().any(|seen| same_path(seen, path)) {
            places.push(path.to_path_buf());
        }
    }
    places
}

fn collect_backups(places: &[PathBuf]) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for place in places {
        let Some(leaf) = place.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(dir) = place.parent() else {
            continue;
        };
        for (candidate, _) in state::find_moved_dirs(dir, leaf) {
            if !found.contains(&candidate) {
                found.push(candidate);
            }
        }
    }
    found
}

pub(super) fn plan(
    app_handle: &AppHandle,
    source: &Path,
    target: &Path,
) -> Result<MigrationPlan, String> {
    validate_target(source, target)?;
    let mut stats = TreeStats::default();
    fs_ops::scan_tree(source, &mut stats)?;
    let free = env::free_bytes(target);
    let needed = stats.bytes + stats.bytes / 100 * SPACE_MARGIN_PERCENT;
    let remembered = state::read_last_migration(app_handle)
        .map(PathBuf::from)
        .is_some_and(|recorded| same_path(&recorded, target));
    Ok(MigrationPlan {
        source: source.to_string_lossy().into_owned(),
        target: target.to_string_lossy().into_owned(),
        total_files: stats.files,
        total_bytes: stats.bytes,
        links: stats.links,
        target_free_bytes: free.unwrap_or(0),
        enough_space: free.is_none_or(|available| available >= needed),
        parent_exists: target.parent().is_some_and(Path::exists),
        remembered,
    })
}

pub(super) fn run(
    app_handle: &AppHandle,
    source: &Path,
    target: &Path,
    emit: ProgressSink<'_>,
) -> Result<MigrationOutcome, String> {
    if !harness_stopped(source) {
        return Err(format!(
            "DATA_DIR_HARNESS_RUNNING: {} 仍在使用数据目录",
            source.display()
        ));
    }
    validate_target(source, target)?;
    emit(step("migrate", "scan", 0, 0, 0, 0));
    let mut expected = TreeStats::default();
    fs_ops::scan_tree(source, &mut expected)?;

    let tick = |files: u64, bytes: u64| {
        emit(step(
            "migrate",
            "copy",
            files,
            bytes,
            expected.files,
            expected.bytes,
        ));
    };
    let progress = CopyProgress::new(&tick);
    fs_ops::copy_tree(source, target, &progress)?;
    let copied = progress.stats();

    emit(step(
        "migrate",
        "verify",
        copied.files,
        copied.bytes,
        expected.files,
        expected.bytes,
    ));
    let mut actual = TreeStats::default();
    fs_ops::scan_tree(target, &mut actual)?;
    if !actual.covers(&expected) {
        return Err(format!(
            "DATA_DIR_COPY_INCOMPLETE: {} -> {}: {} files/{} bytes, expected {} files/{} bytes",
            source.display(),
            target.display(),
            actual.files,
            actual.bytes,
            expected.files,
            expected.bytes
        ));
    }

    emit(step(
        "migrate",
        "finalize",
        copied.files,
        copied.bytes,
        expected.files,
        expected.bytes,
    ));
    let stamp = env::now_stamp();
    let moved = state::with_moved_suffix(source, &stamp)?;
    state::write_last_migration(app_handle, &source.to_string_lossy())?;
    fs::rename(source, &moved).map_err(|e| {
        format!(
            "DATA_DIR_RENAME: {} -> {}: {e}",
            source.display(),
            moved.display()
        )
    })?;

    let text = target.to_string_lossy().into_owned();
    if let Err(error) = env::write_user_home(&text) {
        if let Err(restore) = fs::rename(&moved, source) {
            log::error!("DATA_DIR_RENAME_BACK: {}: {restore}", moved.display());
        }
        return Err(error);
    }
    unsafe {
        std::env::set_var(env::DATA_DIR_ENV, &text);
    }
    refresh_shims(app_handle, target);

    emit(step(
        "migrate",
        "done",
        copied.files,
        copied.bytes,
        expected.files,
        expected.bytes,
    ));
    Ok(MigrationOutcome {
        source: source.to_string_lossy().into_owned(),
        target: text,
        moved_to: moved.to_string_lossy().into_owned(),
        files: copied.files,
        bytes: copied.bytes,
        links: copied.links,
        sessions: fs_ops::count_sessions(target),
    })
}

pub(super) fn rollback(
    app_handle: &AppHandle,
    current: &Path,
    backup: Option<&Path>,
    emit: ProgressSink<'_>,
) -> Result<MigrationOutcome, String> {
    if !harness_stopped(current) {
        return Err(format!(
            "DATA_DIR_HARNESS_RUNNING: {} 仍在使用数据目录",
            current.display()
        ));
    }
    emit(step("rollback", "scan", 0, 0, 0, 0));

    let chosen = match backup {
        Some(explicit) => explicit.to_path_buf(),
        None => find_backups(app_handle, current)
            .first()
            .map(|item| PathBuf::from(&item.path))
            .ok_or_else(|| format!("DATA_DIR_NO_BACKUP: {}", current.display()))?,
    };
    if !chosen.is_dir() {
        return Err(format!("DATA_DIR_BACKUP_MISSING: {}", chosen.display()));
    }
    let restore_to = state::original_of(&chosen)
        .ok_or_else(|| format!("DATA_DIR_BACKUP_NAME_INVALID: {}", chosen.display()))?;

    let stamp = env::now_stamp();
    let aside = if fs_ops::is_dir_empty(&restore_to) {
        if restore_to.exists() {
            fs::remove_dir(&restore_to)
                .map_err(|e| format!("DATA_DIR_ROLLBACK_CLEAR: {}: {e}", restore_to.display()))?;
        }
        String::new()
    } else {
        let target = state::with_current_suffix(&restore_to, &stamp)?;
        fs::rename(&restore_to, &target)
            .map_err(|e| format!("DATA_DIR_ROLLBACK_ASIDE: {}: {e}", restore_to.display()))?;
        target.to_string_lossy().into_owned()
    };

    emit(step("rollback", "finalize", 0, 0, 0, 0));
    if let Some(parent) = restore_to.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("DATA_DIR_ROLLBACK_MKDIR: {e}"))?;
    }
    if let Err(error) = fs::rename(&chosen, &restore_to) {
        if !aside.is_empty() {
            let _ = fs::rename(&aside, &restore_to);
        }
        return Err(format!(
            "DATA_DIR_ROLLBACK_MOVE: {} -> {}: {error}",
            chosen.display(),
            restore_to.display()
        ));
    }

    let back_to_default = same_path(&restore_to, &env::default_home());
    let apply = |text: &str| {
        if back_to_default {
            let cleared = env::clear_user_home();
            if cleared.is_ok() {
                unsafe {
                    std::env::remove_var(env::DATA_DIR_ENV);
                }
            }
            cleared
        } else {
            let written = env::write_user_home(text);
            if written.is_ok() {
                unsafe {
                    std::env::set_var(env::DATA_DIR_ENV, text);
                }
            }
            written
        }
    };
    rewrite_env_or_undo(&chosen, &restore_to, &aside, apply)?;
    if let Err(error) = state::write_last_migration(app_handle, "") {
        log::warn!("DATA_DIR_STATE_CLEAR: {error}");
    }
    refresh_shims(app_handle, &restore_to);

    let mut stats = TreeStats::default();
    if let Err(error) = fs_ops::scan_tree(&restore_to, &mut stats) {
        log::warn!("DATA_DIR_ROLLBACK_SCAN: {error}");
    }
    emit(step(
        "rollback",
        "done",
        stats.files,
        stats.bytes,
        stats.files,
        stats.bytes,
    ));
    Ok(MigrationOutcome {
        source: current.to_string_lossy().into_owned(),
        target: restore_to.to_string_lossy().into_owned(),
        moved_to: aside,
        files: stats.files,
        bytes: stats.bytes,
        links: stats.links,
        sessions: fs_ops::count_sessions(&restore_to),
    })
}

fn rewrite_env_or_undo<F>(
    chosen: &Path,
    restore_to: &Path,
    aside: &str,
    apply: F,
) -> Result<(), String>
where
    F: FnOnce(&str) -> Result<(), String>,
{
    if let Err(error) = apply(&restore_to.to_string_lossy()) {
        undo_rollback(chosen, restore_to, aside);
        return Err(error);
    }
    Ok(())
}

fn undo_rollback(chosen: &Path, restore_to: &Path, aside: &str) {
    if let Err(error) = fs::rename(restore_to, chosen) {
        log::error!("DATA_DIR_ROLLBACK_UNDO: {}: {error}", restore_to.display());
        return;
    }
    if aside.is_empty() {
        return;
    }
    if let Err(error) = fs::rename(Path::new(aside), restore_to) {
        log::error!("DATA_DIR_ROLLBACK_UNDO_ASIDE: {aside}: {error}");
    }
}

fn validate_target(source: &Path, target: &Path) -> Result<(), String> {
    if !target.is_absolute() {
        return Err(format!(
            "DATA_DIR_TARGET_NOT_ABSOLUTE: {}",
            target.display()
        ));
    }
    let resolved_source = resolved_path(source)?;
    let resolved_target = resolved_path(target)?;
    let source = resolved_source.as_path();
    let target = resolved_target.as_path();
    if same_path(source, target) {
        return Err(format!("DATA_DIR_SAME_AS_SOURCE: {}", target.display()));
    }
    if is_within(target, source) {
        return Err(format!(
            "DATA_DIR_TARGET_INSIDE_SOURCE: {}",
            target.display()
        ));
    }
    if is_within(source, target) {
        return Err(format!(
            "DATA_DIR_SOURCE_INSIDE_TARGET: {}",
            target.display()
        ));
    }
    if target.is_file() {
        return Err(format!("DATA_DIR_TARGET_NOT_DIR: {}", target.display()));
    }
    if !fs_ops::is_dir_empty(target) {
        return Err(format!("DATA_DIR_TARGET_NOT_EMPTY: {}", target.display()));
    }
    Ok(())
}

fn resolved_path(path: &Path) -> Result<PathBuf, String> {
    let mut existing = path;
    let mut missing = Vec::new();
    loop {
        match fs::canonicalize(existing) {
            Ok(mut resolved) => {
                if !missing.is_empty() && !resolved.is_dir() {
                    return Err(format!("DATA_DIR_TARGET_NOT_DIR: {}", existing.display()));
                }
                for name in missing.iter().rev() {
                    resolved.push(name);
                }
                return Ok(resolved);
            }
            Err(error) => {
                if error.kind() == std::io::ErrorKind::NotFound
                    && matches!(fs::symlink_metadata(existing), Err(ref metadata_error) if metadata_error.kind() == std::io::ErrorKind::NotFound)
                {
                    if let (Some(name), Some(parent)) = (existing.file_name(), existing.parent()) {
                        missing.push(name);
                        existing = parent;
                        continue;
                    }
                }
                return Err(format!(
                    "DATA_DIR_PATH_RESOLVE: {}: {error}",
                    path.display()
                ));
            }
        }
    }
}

fn refresh_shims(app_handle: &AppHandle, home: &Path) {
    if let Err(error) = crate::service::cli::ensure_shims(app_handle) {
        log::warn!("DATA_DIR_SHIM_REWRITE: {error}");
    }
    if let Err(error) = crate::service::migrate::heal_stale_pnpm_metadata(home) {
        log::warn!("DATA_DIR_PNPM_HEAL: {error}");
    }
}

fn stamp_of(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(state::moved_stamp)
        .map(str::to_string)
        .unwrap_or_default()
}

fn same_path(a: &Path, b: &Path) -> bool {
    is_within(a, b) && is_within(b, a)
}

fn is_within(child: &Path, parent: &Path) -> bool {
    let parent = dunce::simplified(parent);
    if parent.as_os_str().is_empty() {
        return false;
    }
    let mut remaining = dunce::simplified(child).components();
    for part in parent.components() {
        let Some(found) = remaining.next() else {
            return false;
        };
        if !same_component(&found, &part) {
            return false;
        }
    }
    true
}

fn same_component(a: &Component, b: &Component) -> bool {
    if cfg!(windows) {
        a.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
    } else {
        a == b
    }
}

fn step(
    operation: &'static str,
    phase: &'static str,
    copied_files: u64,
    copied_bytes: u64,
    total_files: u64,
    total_bytes: u64,
) -> ProgressPayload {
    ProgressPayload {
        operation,
        phase,
        copied_files,
        copied_bytes,
        total_files,
        total_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_nested_targets_hidden_by_parent_components() {
        let root = temp_dir("resolved-parent");
        let source = root.join("home");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(root.join("other")).unwrap();
        let target = root.join("other").join("..").join("home").join("copy");
        let result = validate_target(&source, &target);
        assert!(
            matches!(result, Err(ref error) if error.starts_with("DATA_DIR_TARGET_INSIDE_SOURCE:")),
            "{result:?}"
        );
        assert!(!source.join("copy").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn validate_rejects_nested_targets_reached_through_symlinked_parents() {
        let root = temp_dir("resolved-link");
        let source = root.join("home");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("sentinel"), "preserve").unwrap();
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        let result = validate_target(&source, &alias.join("copy"));
        assert!(
            matches!(result, Err(ref error) if error.starts_with("DATA_DIR_TARGET_INSIDE_SOURCE:")),
            "{result:?}"
        );
        assert!(!source.join("copy").exists());
        assert_eq!(
            fs::read_to_string(source.join("sentinel")).unwrap(),
            "preserve"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn validate_resolves_the_source_before_comparing_target_boundaries() {
        let root = temp_dir("resolved-source");
        let source = root.join("home");
        fs::create_dir_all(&source).unwrap();
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        let result = validate_target(&alias, &source.join("copy"));
        assert!(
            matches!(result, Err(ref error) if error.starts_with("DATA_DIR_TARGET_INSIDE_SOURCE:")),
            "{result:?}"
        );
        let result = validate_target(&alias, &source);
        assert!(
            matches!(result, Err(ref error) if error.starts_with("DATA_DIR_SAME_AS_SOURCE:")),
            "{result:?}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn validate_allows_separate_targets_under_symlinked_parents() {
        let root = temp_dir("separate-link");
        let source = root.join("home");
        let destination = root.join("separate");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&destination, &alias).unwrap();
        assert_eq!(
            validate_target(&source, &alias.join("new").join("home")),
            Ok(())
        );
        assert!(!destination.join("new").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn validate_rejects_targets_below_a_regular_file() {
        let root = temp_dir("file-parent");
        let source = root.join("home");
        fs::create_dir_all(&source).unwrap();
        let file = root.join("file");
        fs::write(&file, "preserve").unwrap();
        assert!(validate_target(&source, &file.join("copy")).is_err());
        assert_eq!(fs::read_to_string(file).unwrap(), "preserve");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn validate_rejects_dangling_and_cyclic_destination_links() {
        let root = temp_dir("broken-link");
        let source = root.join("home");
        fs::create_dir_all(&source).unwrap();
        let dangling = root.join("dangling");
        std::os::unix::fs::symlink(root.join("missing"), &dangling).unwrap();
        let cyclic = root.join("cyclic");
        std::os::unix::fs::symlink(&cyclic, &cyclic).unwrap();
        for parent in [&dangling, &cyclic] {
            let result = validate_target(&source, &parent.join("copy"));
            assert!(
                matches!(result, Err(ref error) if error.starts_with("DATA_DIR_PATH_RESOLVE:")),
                "{result:?}"
            );
        }
        assert!(!root.join("missing").exists());
        fs::remove_dir_all(root).unwrap();
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsh-data-dir-migrate-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn validate_rejects_same_nested_and_non_empty_targets() {
        let root = temp_dir("validate");
        let source = root.join("home");
        fs::create_dir_all(source.join("sessions")).unwrap();
        assert!(validate_target(&source, &source).is_err());
        assert!(validate_target(&source, &source.join("inner")).is_err());
        assert!(validate_target(&source.join("inner"), &source).is_err());
        assert!(validate_target(&source, Path::new("relative")).is_err());

        let filled = root.join("filled");
        fs::create_dir_all(&filled).unwrap();
        fs::write(filled.join("a.txt"), "1").unwrap();
        assert!(validate_target(&source, &filled).is_err());

        let empty = root.join("empty");
        fs::create_dir_all(&empty).unwrap();
        assert!(validate_target(&source, &empty).is_ok());
        assert!(validate_target(&source, &root.join("missing")).is_ok());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn is_within_compares_path_components_not_prefixes() {
        if cfg!(windows) {
            let data = Path::new("D:\\Data");
            assert!(is_within(Path::new("D:\\Data\\sub"), data));
            assert!(is_within(data, data));
            assert!(!is_within(Path::new("D:\\Database"), data));
            assert!(!is_within(Path::new("D:\\Other"), data));
        } else {
            let data = Path::new("/data");
            assert!(is_within(Path::new("/data/sub"), data));
            assert!(is_within(data, data));
            assert!(!is_within(Path::new("/database"), data));
            assert!(!is_within(Path::new("/other"), data));
        }
        assert!(!is_within(Path::new("/data"), Path::new("")));
    }

    #[cfg(windows)]
    #[test]
    fn is_within_ignores_case_and_the_verbatim_prefix() {
        assert!(is_within(Path::new("d:\\data\\sub"), Path::new("D:\\Data")));
        assert!(is_within(
            Path::new("\\\\?\\D:\\Data\\sub"),
            Path::new("D:\\Data")
        ));
    }

    #[test]
    fn same_path_matches_only_the_same_directory() {
        if cfg!(windows) {
            assert!(same_path(Path::new("D:\\Data"), Path::new("D:\\Data\\")));
            assert!(!same_path(Path::new("D:\\Data"), Path::new("D:\\Data2")));
        } else {
            assert!(same_path(Path::new("/data"), Path::new("/data/")));
            assert!(!same_path(Path::new("/data"), Path::new("/data2")));
        }
    }

    #[test]
    fn search_places_dedupes_without_losing_order() {
        let current = Path::new("D:\\Data\\DSHHome");
        let default = Path::new("C:\\Users\\me\\.dsh");
        let places = search_places(current, default, Some(Path::new("E:\\Old\\DSHHome")));
        assert_eq!(places.len(), 3);
        assert_eq!(places[0], current);
        assert_eq!(places[1], default);
        assert_eq!(places[2], Path::new("E:\\Old\\DSHHome"));
        assert_eq!(search_places(default, default, None).len(), 1);
        assert_eq!(search_places(current, default, None).len(), 2);
    }

    #[test]
    fn collect_backups_matches_each_place_own_name() {
        let root = temp_dir("backups");
        let moved_from = root.join("moved");
        let moved_to = root.join("moved-to");
        fs::create_dir_all(&moved_from).unwrap();
        fs::create_dir_all(&moved_to).unwrap();
        let from = moved_from.join("DSHHome");
        let to = moved_to.join(".dsh");
        fs::create_dir_all(from.join("profiles")).unwrap();
        fs::create_dir_all(to.join("profiles")).unwrap();
        let backup_from = moved_from.join("DSHHome.moved-2026-01-01T00-00-00");
        let backup_to = moved_to.join(".dsh.moved-2026-01-02T00-00-00");
        fs::create_dir_all(&backup_from).unwrap();
        fs::create_dir_all(&backup_to).unwrap();

        let found = collect_backups(&[from.clone(), to.clone()]);
        assert!(
            found.contains(&backup_from),
            "当前位置的备份未被找到：{found:?}"
        );
        assert!(
            found.contains(&backup_to),
            "默认位置的备份未被找到：{found:?}"
        );
        assert!(state::find_moved_dirs(&moved_to, "DSHHome").is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn env_write_failure_puts_both_directories_back() {
        let root = temp_dir("undo");
        let restore_to = root.join("DSHHome");
        let chosen = root.join("DSHHome.moved-2026-01-01T00-00-00");
        let aside = root.join("DSHHome.current-2026-01-01T00-00-00");
        fs::create_dir_all(&restore_to).unwrap();
        fs::write(restore_to.join("sessions.json"), "backup").unwrap();
        fs::create_dir_all(&aside).unwrap();
        fs::write(aside.join("sessions.json"), "current").unwrap();

        let error = rewrite_env_or_undo(&chosen, &restore_to, &aside.to_string_lossy(), |_| {
            Err("DATA_DIR_ENV_WRITE: 注册表写不进去".to_string())
        })
        .unwrap_err();
        assert!(error.contains("DATA_DIR_ENV_WRITE"), "{error}");
        assert_eq!(
            fs::read_to_string(chosen.join("sessions.json")).unwrap(),
            "backup"
        );
        assert_eq!(
            fs::read_to_string(restore_to.join("sessions.json")).unwrap(),
            "current"
        );
        assert!(!aside.exists(), "挪到一边的目录没有被放回去");

        let plain = root.join("Plain");
        let plain_backup = root.join("Plain.moved-2026-01-01T00-00-00");
        fs::create_dir_all(&plain).unwrap();
        fs::write(plain.join("sessions.json"), "current").unwrap();
        assert!(rewrite_env_or_undo(&plain_backup, &plain, "", |_| Err("x".to_string())).is_err());
        assert!(!plain.exists());
        assert_eq!(
            fs::read_to_string(plain_backup.join("sessions.json")).unwrap(),
            "current"
        );

        rewrite_env_or_undo(&chosen, &restore_to, "", |text| {
            assert!(text.ends_with("DSHHome"), "{text}");
            Ok(())
        })
        .unwrap();
        assert!(chosen.is_dir());
        assert!(restore_to.is_dir());
        let _ = fs::remove_dir_all(&root);
    }
}
