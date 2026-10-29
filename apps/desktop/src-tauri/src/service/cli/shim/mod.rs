
use std::path::Path;

mod build;
mod templates;
mod write;

pub use write::{is_generated_shim, user_dsh_preserved, write_shims};

#[cfg(windows)]
pub const SHIM_CMD_NAME: &str = "dsh.cmd";
#[cfg(windows)]
pub const SHIM_PS1_NAME: &str = "dsh.ps1";
#[cfg(windows)]
pub const PNPM_SHIM_CMD_NAME: &str = "pnpm.cmd";
#[cfg(windows)]
pub const PNPM_SHIM_PS1_NAME: &str = "pnpm.ps1";
#[cfg(windows)]
pub const NPX_SHIM_CMD_NAME: &str = "npx.cmd";
#[cfg(windows)]
pub const NPX_SHIM_PS1_NAME: &str = "npx.ps1";
#[cfg(windows)]
pub const NPM_SHIM_CMD_NAME: &str = "npm.cmd";
#[cfg(windows)]
pub const NPM_SHIM_PS1_NAME: &str = "npm.ps1";

#[cfg(unix)]
pub const SHIM_SH_NAME: &str = "dsh";
#[cfg(unix)]
pub const PNPM_SHIM_SH_NAME: &str = "pnpm";

#[inline]
pub fn escape_path_cmd(path: &Path) -> String {
    path.to_string_lossy().replace('%', "%%")
}

pub fn portable_path_cmd(path: &Path) -> String {
    let text = path.to_string_lossy();
    if text.is_ascii() {
        return escape_path_cmd(path);
    }
    for (var, token) in [
        ("APPDATA", "%APPDATA%"),
        ("LOCALAPPDATA", "%LOCALAPPDATA%"),
        ("USERPROFILE", "%USERPROFILE%"),
    ] {
        let Some(root) = std::env::var_os(var) else {
            continue;
        };
        if let Some(rendered) = under_root(path, Path::new(&root), token) {
            return rendered;
        }
    }
    #[cfg(windows)]
    if let Some(short) = crate::utils::short_path(path) {
        if short.to_string_lossy().is_ascii() {
            return escape_path_cmd(&short);
        }
    }
    escape_path_cmd(path)
}

fn under_root(path: &Path, root: &Path, token: &str) -> Option<String> {
    let rest = path.strip_prefix(root).ok()?;
    if rest.as_os_str().is_empty() {
        return None;
    }
    Some(format!("{token}\\{}", escape_path_cmd(rest)))
}

#[inline]
pub fn escape_path_ps1(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

#[inline]
pub fn escape_path_sh(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "'\\''")
}

#[cfg(test)]
mod test_util {
    use std::path::PathBuf;

    pub(super) fn sample_shim_paths() -> super::build::ShimPaths {
        shim_paths_for(&sample_app_dir())
    }

    pub(super) fn shim_paths_for(app_dir: &std::path::Path) -> super::build::ShimPaths {
        let app_dir = app_dir.to_path_buf();
        let node_root = app_dir.join("runtime");
        super::build::ShimPaths {
            node_bin: if cfg!(windows) {
                node_root.join("node.exe")
            } else {
                node_root.join("bin").join("node")
            },
            dsh_bin: app_dir
                .join("dependencies")
                .join("dsh")
                .join("node_modules")
                .join("@deepseek-ai")
                .join("dsh")
                .join("lib")
                .join("bin.js"),
            pnpm_bin: app_dir
                .join("dependencies")
                .join("pnpm")
                .join("bin")
                .join("pnpm.cjs"),
            bundled_git_dir: Some(app_dir.join("dependencies").join("git").join("cmd")),
        }
    }

    pub(super) fn sample_app_dir() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\Users\test\AppData\Roaming\dsh-tauri")
        } else {
            PathBuf::from("/home/test/.local/share/dsh-tauri")
        }
    }

    pub(super) fn sample_dsh_home() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\Users\test\.dsh")
        } else {
            PathBuf::from("/home/test/.dsh")
        }
    }

    pub(super) fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsh-shim-{tag}-{}-{}",
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_path_cmd_doubles_percent() {
        assert_eq!(
            escape_path_cmd(Path::new(r"C:\Users\%test%\x")),
            r"C:\Users\%%test%%\x"
        );
        assert_eq!(escape_path_cmd(Path::new("/tmp/a b")), "/tmp/a b");
    }

    #[test]
    fn escape_path_ps1_doubles_single_quotes() {
        assert_eq!(
            escape_path_ps1(Path::new(r"C:\Users\o'brien")),
            r"C:\Users\o''brien"
        );
        assert_eq!(escape_path_ps1(Path::new("/plain/path")), "/plain/path");
    }

    #[test]
    fn escape_path_sh_escapes_single_quotes() {
        assert_eq!(
            escape_path_sh(Path::new("/home/o'brien/.dsh")),
            r"/home/o'\''brien/.dsh"
        );
        assert_eq!(escape_path_sh(Path::new("/plain/.dsh")), "/plain/.dsh");
    }

    #[test]
    #[cfg(windows)]
    fn portable_path_cmd_renders_non_ascii_user_paths_as_tokens() {
        let root = Path::new(r"C:\Users\小蔡\AppData\Roaming");
        assert_eq!(
            under_root(
                Path::new(r"C:\Users\小蔡\AppData\Roaming\dsh-tauri\dependencies\pnpm\bin\pnpm.cjs"),
                root,
                "%APPDATA%",
            )
            .as_deref(),
            Some(r"%APPDATA%\dsh-tauri\dependencies\pnpm\bin\pnpm.cjs")
        );
        assert_eq!(
            under_root(Path::new(r"C:\Users\小蔡\.dsh"), Path::new(r"C:\Users\小蔡"), "%USERPROFILE%")
                .as_deref(),
            Some(r"%USERPROFILE%\.dsh")
        );
        assert_eq!(under_root(root, root, "%APPDATA%"), None);
        assert_eq!(
            under_root(Path::new(r"D:\tools\x"), root, "%APPDATA%"),
            None
        );
        assert_eq!(
            under_root(Path::new(r"C:\Users\小蔡\a%20b"), Path::new(r"C:\Users\小蔡"), "%USERPROFILE%")
                .as_deref(),
            Some(r"%USERPROFILE%\a%%20b")
        );
    }

    #[test]
    fn portable_path_cmd_keeps_ascii_paths_verbatim() {
        assert_eq!(
            portable_path_cmd(Path::new(r"C:\Users\test\AppData\Roaming\dsh-tauri\pnpm.cjs")),
            r"C:\Users\test\AppData\Roaming\dsh-tauri\pnpm.cjs"
        );
        assert_eq!(
            portable_path_cmd(Path::new(r"C:\tools\100%\pnpm.cjs")),
            r"C:\tools\100%%\pnpm.cjs"
        );
    }
}
