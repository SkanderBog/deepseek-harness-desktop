
use std::path::{Path, PathBuf};

pub fn validate_id(id: &str) -> Result<(), String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("INVALID_ID: id is empty".to_string());
    }
    if id.len() > 128 {
        return Err("INVALID_ID: id too long".to_string());
    }
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
    {
        return Err(format!(
            "INVALID_ID: id contains forbidden characters: {id}"
        ));
    }
    if id == "." || id == ".." || id.starts_with("..") || id.ends_with("..") {
        return Err(format!("INVALID_ID: path traversal attempt: {id}"));
    }
    Ok(())
}

pub fn ensure_within(child: &Path, root: &Path) -> Result<PathBuf, String> {
    let root_real = dunce::canonicalize(root).map_err(|e| format!("ROOT_RESOLVE_FAILED: {e}"))?;
    let child_real = dunce::canonicalize(child).map_err(|e| format!("PATH_RESOLVE_FAILED: {e}"))?;
    if !child_real.starts_with(&root_real) {
        return Err(format!(
            "PATH_ESCAPE_REJECTED: {} is outside {}",
            child_real.display(),
            root_real.display()
        ));
    }
    Ok(child_real)
}

pub fn safe_remove_target(root: &Path, id: &str) -> Result<PathBuf, String> {
    validate_id(id)?;
    let dir = root.join(id);
    if !dir.exists() {
        return Err("TARGET_NOT_FOUND: target directory does not exist".to_string());
    }
    let target = ensure_within(&dir, root)?;
    let root_real = dunce::canonicalize(root).map_err(|e| format!("ROOT_RESOLVE_FAILED: {e}"))?;
    // issue #848：链接别名可绕过词法相等检查，删除前必须拒绝真实根目录。
    if target == root_real {
        return Err("REMOVE_ROOT_REJECTED: refusing to remove the root directory".to_string());
    }
    Ok(target)
}

pub fn join_safe(root: &Path, id: &str) -> Result<PathBuf, String> {
    validate_id(id)?;
    Ok(root.join(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn validate_id_rejects_traversal() {
        for bad in [
            "..", ".", "../x", "..\\x", "/etc", "\\etc", "a/b", "a b", "a\tb", "a:b",
        ] {
            assert!(validate_id(bad).is_err(), "should reject {bad:?}");
        }
        for good in ["web", "my-profile", "dsh-1.2.3", "plugin-a_b", "app-7"] {
            assert!(validate_id(good).is_ok(), "should accept {good:?}");
        }
    }

    #[test]
    fn safe_remove_rejects_escape() {
        let dir = std::env::temp_dir().join(format!("dsh-fsguard-{}", std::process::id()));
        let root = dir.join("root");
        fs::create_dir_all(&root).unwrap();
        let guard = std::panic::catch_unwind(|| {
            assert!(safe_remove_target(&root, "..").is_err());
            assert!(safe_remove_target(&root, "nonexistent").is_err());
            let res = safe_remove_target(&root, "web");
            assert!(res.is_err());
            assert!(res.unwrap_err().starts_with("TARGET_NOT_FOUND"));
        });
        let _ = fs::remove_dir_all(&dir);
        assert!(guard.is_ok(), "test panicked: {guard:?}");
    }

    #[test]
    fn safe_remove_rejects_symlink_escape() {
        let dir = std::env::temp_dir().join(format!("dsh-fsguard3-{}", std::process::id()));
        let root = dir.join("profiles");
        fs::create_dir_all(&root).unwrap();
        let outside = dir.join("outside-target");
        fs::create_dir_all(&outside).unwrap();
        #[cfg(unix)]
        {
            let link = root.join("escape-link");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert!(safe_remove_target(&root, "escape-link").is_err());
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn safe_remove_rejects_link_to_root() {
        let dir = std::env::temp_dir().join(format!("dsh-fsguard-root-{}", std::process::id()));
        let root = dir.join("profiles");
        fs::create_dir_all(&root).unwrap();
        let link = root.join("alias");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&root, &link).unwrap();
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let status = std::process::Command::new("cmd")
                .arg("/C")
                .raw_arg(format!("mklink /J \"{}\" \"{}\"", link.display(), root.display()))
                .creation_flags(0x08000000)
                .status()
                .unwrap();
            assert!(status.success());
        }
        let result = safe_remove_target(&root, "alias");
        fs::remove_dir_all(&dir).unwrap();
        assert!(result.unwrap_err().starts_with("REMOVE_ROOT_REJECTED:"));
    }

    #[test]
    fn safe_remove_accepts_valid_existing_dir() {
        let dir = std::env::temp_dir().join(format!("dsh-fsguard2-{}", std::process::id()));
        let root = dir.join("profiles");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(root.join("web")).unwrap();
        let res = safe_remove_target(&root, "web");
        assert!(res.is_ok());
        let res = join_safe(&root, "app-1");
        assert_eq!(res.unwrap(), root.join("app-1"));
        let _ = fs::remove_dir_all(&dir);
    }
}
