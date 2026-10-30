use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// 打光，也不再每次启动/重启都向日志刷一条吓人又无法处置的 403 警告（issue #48）。
pub mod github_api {
    use super::*;

    const COOLDOWN: Duration = Duration::from_secs(30 * 60);

    static COOLDOWN_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

    pub fn rate_limited() -> bool {
        COOLDOWN_UNTIL
            .lock()
            .map(|slot| slot.is_some_and(|until| Instant::now() < until))
            .unwrap_or(false)
    }

    pub fn mark_rate_limited() {
        if let Ok(mut slot) = COOLDOWN_UNTIL.lock() {
            let now = Instant::now();
            let already_cooling = slot.is_some_and(|until| now < until);
            if !already_cooling {
                log::warn!(
                    "GitHub API rate-limited (HTTP 403), pausing api.github.com for {} min, using fallback sources",
                    COOLDOWN.as_secs() / 60
                );
            }
            *slot = Some(now + COOLDOWN);
        }
    }
}

#[cfg(unix)]
pub fn fix_recursive_permissions(path: &Path) -> io::Result<()> {
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)?;

    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            fix_recursive_permissions(&entry?.path())?;
        }
    }
    Ok(())
}

const PRESERVE_DIRS: &[&str] = &["node_modules", ".git"];

/// Renames the single `*.asar` at `dest` to the stable name the manifest entry
/// resolves (`dsh.asar`). Upstream names the archive after the platform, but the
/// entry path must not change per platform.
pub fn normalize_asar_name(dest: &Path) -> Result<PathBuf, String> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dest).map_err(|e| format!("ASAR_READ_DIR: {e}"))? {
        entries.push(entry.map_err(|e| format!("ASAR_READ_DIR: {e}"))?.path());
    }
    let archives: Vec<&PathBuf> = entries
        .iter()
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("asar"))
        })
        .collect();
    if entries.len() != 1 || archives.len() != 1 {
        return Err(format!(
            "ASAR_LAYOUT_INVALID: expected exactly one top-level .asar in {}, found {} entries and {} archives",
            dest.display(),
            entries.len(),
            archives.len()
        ));
    }
    let archive = archives[0];
    let target = dest.join("dsh.asar");
    if archive == &target {
        return Ok(target);
    }
    if target.exists() {
        return Err(format!(
            "ASAR_LAYOUT_INVALID: duplicate target {}",
            target.display()
        ));
    }
    fs::rename(archive, &target).map_err(|e| format!("ASAR_RENAME: {e}"))?;
    log::info!(
        "Normalized dsh asar: {} -> {}",
        archive.display(),
        target.display()
    );
    Ok(target)
}

pub fn flatten_directory(dest: &PathBuf) -> Result<(), String> {
    let mut sub_dirs = Vec::new();
    for entry in fs::read_dir(dest).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| !name.starts_with('.') && !PRESERVE_DIRS.contains(&name))
        {
            sub_dirs.push(path);
        }
    }

    if sub_dirs.len() != 1 {
        return Ok(());
    }

    let sub_dir = &sub_dirs[0];
    log::debug!("Flattening subdirectory: {:?}", sub_dir);

    for entry in fs::read_dir(sub_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let from = entry.path();
        let file_name = entry.file_name();
        let to = dest.join(&file_name);

        force_move(&from, &to).map_err(|e| {
            let msg = format!("Failed to move {:?} to {:?}: {}", from, to, e);
            log::error!("{}", msg);
            msg
        })?;
    }

    if let Err(e) = fs::remove_dir(sub_dir) {
        log::warn!("Could not remove empty directory {:?}: {}", sub_dir, e);
    }

    Ok(())
}

fn force_move(from: &Path, to: &Path) -> io::Result<()> {
    if let Err(err) = fs::rename(from, to) {
        if to.exists() {
            log::debug!("Target exists, removing: {:?}", to);
            if to.is_dir() {
                fs::remove_dir_all(to)?;
            } else {
                fs::remove_file(to)?;
            }
            fs::rename(from, to)?;
        } else {
            return Err(err);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::github_api;
    use std::fs;

    #[test]
    fn asar_normalization_requires_one_archive_only() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("package-linux.asar");
        fs::write(&source, b"archive").unwrap();
        assert_eq!(
            super::normalize_asar_name(dir.path()).unwrap(),
            dir.path().join("dsh.asar")
        );
        let extra = dir.path().join("README.txt");
        fs::write(&extra, b"extra").unwrap();
        assert!(super::normalize_asar_name(dir.path()).is_err());
    }

    #[test]
    fn rate_limit_cooldown_activates_after_mark() {
        github_api::mark_rate_limited();
        assert!(github_api::rate_limited());
        github_api::mark_rate_limited();
        assert!(github_api::rate_limited());
    }
}
