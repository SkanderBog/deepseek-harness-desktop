//! 背景（issue #466）：`~/.dsh` 可能整棵子树的属主不是当前用户——最常见的原因是

use std::io;
use std::path::{Path, PathBuf};

pub fn is_permission_error(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::PermissionDenied {
        return true;
    }
    error.raw_os_error() == Some(30) && cfg!(unix)
}

/// 用户」这一 issue #466 的典型形态在建目录这一步完全看不出问题，直到后面写清单
pub fn ensure_dir_writable(dir: &Path, prefix: &str) -> Result<(), String> {
    if let Err(error) = std::fs::create_dir_all(dir) {
        let remedy = nearest_existing_dir(dir).unwrap_or_else(|| dir.to_path_buf());
        return Err(dir_error(prefix, dir, &remedy, &error));
    }
    probe(dir, dir, prefix)
}

/// **绝不创建任何目录**：插件核对阶段抢先建出半初始化档案会落进 issue #452 的
pub fn ensure_writable_path(target: &Path, home_root: &Path, prefix: &str) -> Result<(), String> {
    match nearest_existing_dir(target) {
        Some(dir) => probe(&dir, home_root, prefix),
        None => Ok(()),
    }
}

fn nearest_existing_dir(target: &Path) -> Option<PathBuf> {
    let mut current = Some(target);
    while let Some(dir) = current {
        if dir.is_dir() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

fn probe(dir: &Path, remedy_root: &Path, prefix: &str) -> Result<(), String> {
    let probe_path: PathBuf = dir.join(format!(".dsh-write-probe-{}", std::process::id()));
    match std::fs::write(&probe_path, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe_path);
            Ok(())
        }
        Err(error) if is_permission_error(&error) => {
            let remedy = if remedy_root.is_dir() { remedy_root } else { dir };
            Err(dir_error(prefix, dir, remedy, &error))
        }
        Err(error) => {
            log::warn!(
                "{prefix}: writability probe on {} failed (treated as writable): {error}",
                dir.display()
            );
            Ok(())
        }
    }
}

fn dir_error(prefix: &str, failing: &Path, remedy_root: &Path, error: &io::Error) -> String {
    if !is_permission_error(error) {
        return format!("{prefix}: {}: {error}", failing.display());
    }
    format!(
        "{prefix}: {failing} 不可写（{error}{owner}）。应用无权修改该目录的属主/权限，\
         dsh 在其下写 cordis.yml/settings.yaml 与插件依赖必然失败。{remedy}",
        failing = failing.display(),
        owner = owner_suffix(remedy_root),
        remedy = remedy_hint(remedy_root),
    )
}

#[cfg(unix)]
fn owner_suffix(path: &Path) -> String {
    use std::os::unix::fs::MetadataExt;
    let Ok(metadata) = std::fs::metadata(path) else {
        return String::new();
    };
    let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
    format!(
        "；该目录属主 uid={}, gid={}，当前用户 uid={uid}, gid={gid}",
        metadata.uid(),
        metadata.gid(),
    )
}

#[cfg(not(unix))]
fn owner_suffix(_path: &Path) -> String {
    String::new()
}

#[cfg(unix)]
fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

#[cfg(unix)]
fn remedy_hint(root: &Path) -> String {
    format!(
        "请在终端执行 `sudo chown -R \"$(id -u):$(id -g)\" {}` 后重试——该状态通常由此前\
         用 sudo 运行过 dsh 造成（macOS 的 sudo 保留 $HOME，`~/.dsh` 会被 root 创建）。",
        shell_quote(root)
    )
}

#[cfg(not(unix))]
fn remedy_hint(root: &Path) -> String {
    format!(
        "请以管理员身份打开终端执行 `takeown /f \"{path}\" /r /d y && \
         icacls \"{path}\" /grant \"%USERNAME%\":(OI)(CI)F /T` 接管所有权并授予当前用户\
         完全控制后重试。",
        path = root.display(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("dsh-perm-{}-{label}", std::process::id()))
    }

    #[test]
    fn permission_error_detection_is_conservative() {
        assert!(is_permission_error(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
        assert!(!is_permission_error(&io::Error::from(
            io::ErrorKind::NotFound
        )));
        #[cfg(unix)]
        {
            assert!(is_permission_error(&io::Error::from_raw_os_error(13)));
            assert!(is_permission_error(&io::Error::from_raw_os_error(1)));
            assert!(is_permission_error(&io::Error::from_raw_os_error(30)));
            assert!(!is_permission_error(&io::Error::from_raw_os_error(5)));
        }
        #[cfg(windows)]
        assert!(is_permission_error(&io::Error::from_raw_os_error(5)));
    }

    #[test]
    fn writable_dir_passes_probe_and_creates_missing_path() {
        let root = temp_root("ok");
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("profiles").join("web");

        assert!(ensure_dir_writable(&dir, "PROFILE_MKDIR").is_ok());
        assert!(dir.is_dir());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn preflight_never_creates_the_target() {
        let root = temp_root("no-lazy-init");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("profiles").join("safe");

        assert!(ensure_writable_path(&target, &root, "PROFILE_NOT_WRITABLE").is_ok());
        // issue #452：预检不得抢先建出半初始化档案目录
        assert!(!target.exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn non_permission_failure_keeps_raw_error() {
        let root = temp_root("not-a-dir");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("web"), b"").unwrap();

        let error = ensure_dir_writable(&root.join("web"), "PROFILE_MKDIR").unwrap_err();
        assert!(error.starts_with("PROFILE_MKDIR:"), "{error}");
        assert!(!error.contains("chown"), "{error}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn readonly_dir_reports_actionable_permission_error() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("readonly");
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("profiles").join("web");
        std::fs::create_dir_all(&dir).unwrap();

        let mut locked = std::fs::metadata(&dir).unwrap().permissions();
        locked.set_mode(0o555);
        std::fs::set_permissions(&dir, locked).unwrap();

        let writable_as_root = std::fs::write(dir.join("canary"), b"").is_ok();
        if !writable_as_root {
            let error = ensure_dir_writable(&dir, "PROFILE_MKDIR").unwrap_err();
            assert!(error.starts_with("PROFILE_MKDIR:"), "{error}");
            assert!(error.contains("chown -R"), "{error}");
            assert!(error.contains("uid="), "{error}");

            let error = ensure_writable_path(&dir, &root, "PROFILE_NOT_WRITABLE").unwrap_err();
            assert!(error.starts_with("PROFILE_NOT_WRITABLE:"), "{error}");
            assert!(error.contains(&shell_quote(&root)), "{error}");

            let missing = dir.join("safe");
            let error = ensure_dir_writable(&missing, "PROFILE_MKDIR").unwrap_err();
            assert!(error.starts_with("PROFILE_MKDIR:"), "{error}");
            assert!(error.contains(&shell_quote(&dir)), "{error}");
            assert!(!error.contains(&shell_quote(&missing)), "{error}");
        }

        let mut open = std::fs::metadata(&dir).unwrap().permissions();
        open.set_mode(0o755);
        let _ = std::fs::set_permissions(&dir, open);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remedy_hint_is_platform_appropriate() {
        let root = Path::new("data-dir");
        #[cfg(unix)]
        assert!(remedy_hint(root).contains("sudo chown -R"));
        #[cfg(not(unix))]
        {
            assert!(remedy_hint(root).contains("icacls"));
            assert!(!remedy_hint(root).contains("chown"));
        }
    }
}
