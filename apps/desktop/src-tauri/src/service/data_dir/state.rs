use crate::AppHandle;
use std::fs;
use std::path::{Path, PathBuf};

const MOVED_INFIX: &str = ".moved-";

pub(super) const CURRENT_INFIX: &str = ".current-";

pub(super) fn state_path(app_handle: &AppHandle) -> PathBuf {
    crate::service::cli::get_bin_dir(app_handle)
        .parent()
        .map(|dir| dir.to_path_buf())
        .unwrap_or_else(std::env::temp_dir)
        .join("last-migration.txt")
}

pub(super) fn read_last_migration(app_handle: &AppHandle) -> Option<String> {
    let text = fs::read_to_string(state_path(app_handle)).ok()?;
    let first = text.lines().next()?.trim();
    if first.is_empty() {
        None
    } else {
        Some(first.to_string())
    }
}

pub(super) fn write_last_migration(app_handle: &AppHandle, value: &str) -> Result<(), String> {
    let path = state_path(app_handle);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("DATA_DIR_STATE_MKDIR: {e}"))?;
    }
    fs::write(&path, value).map_err(|e| format!("DATA_DIR_STATE_WRITE: {e}"))
}

pub(super) fn moved_stamp(name: &str) -> Option<&str> {
    let index = name.rfind(MOVED_INFIX)?;
    let stamp = &name[index + MOVED_INFIX.len()..];
    if stamp.is_empty() {
        return None;
    }
    if stamp
        .chars()
        .all(|c| c.is_ascii_digit() || c == 'T' || c == '-')
    {
        Some(stamp)
    } else {
        None
    }
}

pub(super) fn original_of(moved: &Path) -> Option<PathBuf> {
    let name = moved.file_name()?.to_str()?;
    let stamp = moved_stamp(name)?;
    let leaf = &name[..name.len() - MOVED_INFIX.len() - stamp.len()];
    if leaf.is_empty() {
        return None;
    }
    Some(moved.with_file_name(leaf))
}

pub(super) fn find_moved_dirs(parent: &Path, leaf: &str) -> Vec<(PathBuf, String)> {
    let Ok(read_dir) = fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in read_dir.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(stamp) = moved_stamp(name) else {
            continue;
        };
        if name.len() != leaf.len() + MOVED_INFIX.len() + stamp.len() {
            continue;
        }
        if !name.starts_with(leaf) {
            continue;
        }
        found.push((entry.path(), stamp.to_string()));
    }
    found
}

pub(super) fn with_moved_suffix(original: &Path, stamp: &str) -> Result<PathBuf, String> {
    suffixed(original, MOVED_INFIX, stamp)
}

pub(super) fn with_current_suffix(original: &Path, stamp: &str) -> Result<PathBuf, String> {
    suffixed(original, CURRENT_INFIX, stamp)
}

fn suffixed(original: &Path, infix: &str, stamp: &str) -> Result<PathBuf, String> {
    let name = original
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("DATA_DIR_NAME_INVALID: {}", original.display()))?;
    if original.parent().is_none() {
        return Err(format!("DATA_DIR_NAME_INVALID: {}", original.display()));
    }
    let mut candidate = original.with_file_name(format!("{name}{infix}{stamp}"));
    let mut attempt = 2;
    while candidate.exists() {
        candidate = original.with_file_name(format!("{name}{infix}{stamp}-{attempt}"));
        attempt += 1;
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_moved_suffix_produces_a_script_compatible_name() {
        let dir = std::env::temp_dir().join(format!(
            "dsh-data-dir-suffix-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let original = dir.join("DSHHome");
        fs::create_dir_all(&original).unwrap();

        let moved = with_moved_suffix(&original, "2026-10-02T21-33-14").unwrap();
        assert_eq!(
            moved.file_name().unwrap().to_str().unwrap(),
            "DSHHome.moved-2026-10-02T21-33-14"
        );
        assert_eq!(original_of(&moved).unwrap(), original);

        fs::create_dir_all(&moved).unwrap();
        let second = with_moved_suffix(&original, "2026-10-02T21-33-14").unwrap();
        assert_ne!(second, moved);
        assert_eq!(original_of(&second).unwrap(), original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn with_current_suffix_keeps_the_original_path_recoverable() {
        let original = Path::new("/DSHHome");
        let aside = with_current_suffix(original, "2026-10-02T21-33-14").unwrap();
        assert_eq!(
            aside.file_name().unwrap().to_str().unwrap(),
            "DSHHome.current-2026-10-02T21-33-14"
        );
        assert_eq!(
            moved_stamp(aside.file_name().unwrap().to_str().unwrap()),
            None
        );
    }

    #[test]
    fn moved_stamp_accepts_only_script_compatible_suffixes() {
        assert_eq!(
            moved_stamp("DSHHome.moved-2026-10-02T21-33-14"),
            Some("2026-10-02T21-33-14")
        );
        assert_eq!(moved_stamp("DSHHome.moved-"), None);
        assert_eq!(moved_stamp("DSHHome.moved-2026.10.02"), None);
        assert_eq!(moved_stamp("DSHHome.current-2026-10-02T21-33-14"), None);
        assert_eq!(moved_stamp("DSHHome"), None);
    }

    #[test]
    fn original_of_strips_the_moved_suffix() {
        let moved = Path::new("D:\\DSHHome.moved-2026-10-02T21-33-14");
        assert_eq!(
            original_of(moved).unwrap(),
            Path::new("D:\\DSHHome").to_path_buf()
        );
        assert_eq!(original_of(Path::new("D:\\DSHHome")), None);
    }

    #[test]
    fn find_moved_dirs_matches_only_the_same_leaf() {
        let root = std::env::temp_dir().join(format!(
            "dsh-data-dir-state-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("DSHHome.moved-2026-10-02T21-33-14")).unwrap();
        fs::create_dir_all(root.join("DSHHome.moved-2026-10-03T01-02-03")).unwrap();
        fs::create_dir_all(root.join("Other.moved-2026-10-02T21-33-14")).unwrap();
        fs::create_dir_all(root.join("DSHHome.current-2026-10-02T21-33-14")).unwrap();

        let mut found: Vec<String> = find_moved_dirs(&root, "DSHHome")
            .into_iter()
            .map(|(_, stamp)| stamp)
            .collect();
        found.sort();
        assert_eq!(found, vec!["2026-10-02T21-33-14", "2026-10-03T01-02-03"]);
        let _ = fs::remove_dir_all(&root);
    }
}
