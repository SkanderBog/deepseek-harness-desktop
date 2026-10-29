
use crate::config;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{Runtime};

#[cfg(not(windows))]
#[allow(unused_imports)] // debug 构建 dsh shim 不写入；测试仍引用
use super::build::build_sh_shim;
#[allow(unused_imports)] // 构建函数在 debug 构建/异平台下由 cfg 裁剪，测试仍引用
use super::build::{
    build_cmd_shim, build_npm_cmd_shim, build_npm_ps1_shim, build_npx_cmd_shim, build_npx_ps1_shim,
    build_pnpm_cmd_shim, build_pnpm_ps1_shim, build_pnpm_sh_shim, build_ps1_shim, ShimPaths,
};
#[cfg(all(windows, not(debug_assertions)))]
use super::SHIM_PS1_NAME;
#[cfg(windows)]
use super::{
    NPM_SHIM_CMD_NAME, NPM_SHIM_PS1_NAME, NPX_SHIM_CMD_NAME, NPX_SHIM_PS1_NAME, PNPM_SHIM_CMD_NAME,
    PNPM_SHIM_PS1_NAME, SHIM_CMD_NAME,
};
#[cfg(not(windows))]
use super::{PNPM_SHIM_SH_NAME, SHIM_SH_NAME};
use crate::{AppHandle};

const GENERATED_MARKER: &str = "DeepSeek Harness Desktop - ";

fn is_foreign_file(path: &Path) -> bool {
    !is_generated_shim(path)
}

fn is_dangling_symlink(path: &Path) -> bool {
    match path.symlink_metadata() {
        Ok(meta) => meta.file_type().is_symlink() && !path.exists(),
        Err(_) => false,
    }
}

pub fn is_generated_shim(path: &Path) -> bool {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes)
            .lines()
            .take(2)
            .any(|line| line.contains(GENERATED_MARKER)),
        Err(_) => false,
    }
}

fn encode_shim(target: &Path, content: &str) -> Vec<u8> {
    #[cfg(not(windows))]
    let _ = target;
    if content.is_ascii() {
        return content.as_bytes().to_vec();
    }
    #[cfg(windows)]
    match target.extension().and_then(|ext| ext.to_str()) {
        Some("cmd") | Some("bat") => {
            if let Some(bytes) =
                crate::utils::encode_multibyte(content, crate::utils::console_code_page())
            {
                return bytes;
            }
            log::warn!(
                "Shim {target:?} contains characters the console code page cannot represent; \
                 falling back to utf-8, its baked paths will not resolve"
            );
        }
        Some("ps1") => {
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(content.as_bytes());
            return bytes;
        }
        _ => {}
    }
    content.as_bytes().to_vec()
}

fn write_shim_file(target: &Path, content: &str) -> Result<(), String> {
    if is_dangling_symlink(target) {
        log::warn!(
            "Removing dangling symlink {:?} before writing shim (its target is gone)",
            target
        );
        fs::remove_file(target).map_err(|e| {
            format!(
                "SHIM_REMOVE_LINK_FAILED: remove dangling symlink {} failed: {e}",
                target.display()
            )
        })?;
    }
    if target.exists() && is_foreign_file(target) {
        log::warn!(
            "Skipping shim write to {:?}: an existing user file is preserved",
            target
        );
        return Ok(());
    }
    fs::write(target, encode_shim(target, content))
        .map_err(|e| format!("SHIM_WRITE_FAILED: write {} failed: {e}", target.display()))
}

pub fn user_dsh_preserved(bin_dir: &Path) -> bool {
    let path = {
        #[cfg(windows)]
        {
            bin_dir.join(SHIM_CMD_NAME)
        }
        #[cfg(not(windows))]
        {
            bin_dir.join(SHIM_SH_NAME)
        }
    };
    path.is_file() && is_foreign_file(&path)
}

pub fn shim_paths<R: Runtime>(app_handle: &AppHandle<R>) -> ShimPaths {
    ShimPaths {
        node_bin: config::get_node_binary_path(app_handle),
        dsh_bin: config::get_dsh_binary_path(app_handle),
        pnpm_bin: config::get_pnpm_binary_path(app_handle),
        bundled_git_dir: bundled_git_dir(app_handle),
    }
}

#[cfg(windows)]
fn bundled_git_dir<R: Runtime>(app_handle: &AppHandle<R>) -> Option<PathBuf> {
    config::get_git_cmd_dir(app_handle)
}

#[cfg(not(windows))]
fn bundled_git_dir<R: Runtime>(_app_handle: &AppHandle<R>) -> Option<PathBuf> {
    None
}

pub fn write_shims<R: Runtime>(app_handle: &AppHandle<R>, bin_dir: &Path) -> Result<(), String> {
    let paths = shim_paths(app_handle);
    let dsh_home = config::get_dsh_data_path(app_handle);
    fs::create_dir_all(bin_dir)
        .map_err(|e| format!("SHIM_MKDIR_FAILED: create bin dir failed: {e}"))?;

    macro_rules! write_if_ours {
        ($path:expr, $content:expr) => {{
            let target = bin_dir.join($path);
            write_shim_file(&target, &$content)?;
            target
        }};
    }

    #[cfg(not(debug_assertions))]
    {
        #[cfg(windows)]
        {
            write_if_ours!(SHIM_CMD_NAME, build_cmd_shim(&paths, &dsh_home));
            write_if_ours!(SHIM_PS1_NAME, build_ps1_shim(&paths, &dsh_home));
        }
        #[cfg(not(windows))]
        {
            write_if_ours!(SHIM_SH_NAME, build_sh_shim(&paths, &dsh_home));
        }
    }
    #[cfg(debug_assertions)]
    {
        let _ = &dsh_home;
        log::debug!("debug build: skip dsh shim write (shared user state kept for release)");
    }

    #[cfg(windows)]
    {
        write_if_ours!(PNPM_SHIM_CMD_NAME, build_pnpm_cmd_shim(&paths));
        write_if_ours!(PNPM_SHIM_PS1_NAME, build_pnpm_ps1_shim(&paths));
        write_if_ours!(NPX_SHIM_CMD_NAME, build_npx_cmd_shim(&paths));
        write_if_ours!(NPX_SHIM_PS1_NAME, build_npx_ps1_shim(&paths));
        write_if_ours!(NPM_SHIM_CMD_NAME, build_npm_cmd_shim(&paths));
        write_if_ours!(NPM_SHIM_PS1_NAME, build_npm_ps1_shim(&paths));
    }
    #[cfg(not(windows))]
    {
        write_if_ours!(PNPM_SHIM_SH_NAME, build_pnpm_sh_shim(&paths));
        let chmod_names: &[&str] = if cfg!(debug_assertions) {
            &[PNPM_SHIM_SH_NAME]
        } else {
            &[SHIM_SH_NAME, PNPM_SHIM_SH_NAME]
        };
        for name in chmod_names {
            let path = bin_dir.join(name);
            if path.is_file() && !is_foreign_file(&path) {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                    .map_err(|e| format!("SHIM_CHMOD_FAILED: chmod shim failed: {e}"))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{sample_dsh_home, sample_shim_paths, shim_paths_for};
    use super::*;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    #[test]
    fn foreign_file_detection() {
        let dir = std::env::temp_dir().join(format!("dsh-shim-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let user_dsh = dir.join(if cfg!(windows) { "dsh.cmd" } else { "dsh" });
        std::fs::write(&user_dsh, "#!/bin/sh\necho my real dsh\n").unwrap();
        assert!(
            is_foreign_file(&user_dsh),
            "user file must be treated as foreign"
        );

        #[cfg(not(windows))]
        let generated = build_sh_shim(&sample_shim_paths(), &sample_dsh_home());
        #[cfg(windows)]
        let generated = build_cmd_shim(&sample_shim_paths(), &sample_dsh_home());
        std::fs::write(&user_dsh, generated).unwrap();
        assert!(
            !is_foreign_file(&user_dsh),
            "generated shim must not be foreign"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsh-shim-write-{tag}-{}-{}",
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

    #[test]
    #[cfg(unix)]
    fn write_shim_file_removes_dangling_symlink() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir("dangling");
        let target = dir.join("dsh");
        symlink(dir.join("missing/source/current/bin/dsh"), &target).unwrap();
        assert!(is_dangling_symlink(&target));

        write_shim_file(&target, "#!/bin/sh\ngenerated shim\n").unwrap();

        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "#!/bin/sh\ngenerated shim\n"
        );
        assert!(
            !std::fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink(),
            "dangling symlink must be replaced by a regular file"
        );
        assert!(!is_dangling_symlink(&target));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn write_shim_file_preserves_valid_user_symlink() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir("userlink");
        let real = dir.join("real-dsh");
        std::fs::write(&real, "#!/bin/sh\necho my real dsh\n").unwrap();
        let target = dir.join("dsh");
        symlink(&real, &target).unwrap();

        write_shim_file(&target, "#!/bin/sh\ngenerated shim\n").unwrap();

        assert!(
            std::fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink(),
            "valid user symlink must be preserved"
        );
        assert_eq!(
            std::fs::read_to_string(&real).unwrap(),
            "#!/bin/sh\necho my real dsh\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn generated_phrase_outside_header_is_not_generated_shim() {
        let dir =
            std::env::temp_dir().join(format!("dsh-shim-header-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join(if cfg!(windows) { "dsh.cmd" } else { "dsh" });
        std::fs::write(
            &target,
            "@echo off\necho user shim\necho see DeepSeek Harness Desktop - readme note here\n",
        )
        .unwrap();
        assert!(
            is_foreign_file(&target),
            "phrase outside the header must not mark a user file as generated"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_shim_file_overwrites_generated_shim() {
        let dir = temp_dir("overwrite");
        let target = dir.join("dsh");
        std::fs::write(
            &target,
            "#!/bin/sh\n# DeepSeek Harness Desktop - old shim\n",
        )
        .unwrap();

        write_shim_file(
            &target,
            "#!/bin/sh\n# DeepSeek Harness Desktop - new shim\n",
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "#!/bin/sh\n# DeepSeek Harness Desktop - new shim\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// issue #581：旧版本落盘的 LF-only `.cmd` 仍带生成标记（可被覆盖），
    #[test]
    fn write_shim_file_migrates_legacy_lf_only_cmd_shim() {
        let dir = temp_dir("legacy-lf-cmd");
        let target = dir.join("pnpm.cmd");
        let current = build_pnpm_cmd_shim(&shim_paths_for(&dir.join("app")));
        assert!(current.contains("\r\n"));
        std::fs::write(&target, current.replace("\r\n", "\n")).unwrap();
        assert!(
            !is_foreign_file(&target),
            "legacy generated shim must stay overwritable"
        );

        write_shim_file(&target, &current).unwrap();

        let content = std::fs::read_to_string(&target).unwrap();
        assert_eq!(content, current);
        assert_eq!(
            content.matches('\n').count(),
            content.matches("\r\n").count(),
            "migrated shim must not keep unpaired LF"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(windows)]
    fn cmd_shim_with_non_ascii_path_uses_console_code_page() {
        let dir = temp_dir("non-ascii-cmd");
        let target = dir.join("pnpm.cmd");
        let content = "@echo off\r\nrem DeepSeek Harness Desktop - pnpm command shim (generated)\r\nset \"PNPM_BIN=C:\\Users\\小蔡\\pnpm.cjs\"\r\n";
        assert!(!content.is_ascii());

        write_shim_file(&target, content).unwrap();

        let bytes = std::fs::read(&target).unwrap();
        let codepage = crate::utils::console_code_page();
        match crate::utils::encode_multibyte(content, codepage) {
            Some(encoded) => {
                assert_ne!(
                    bytes,
                    content.as_bytes(),
                    "non-ascii cmd shim must not be written as raw utf-8"
                );
                assert_eq!(bytes, encoded);
                assert_eq!(
                    crate::utils::decode_multibyte(&bytes, codepage).as_deref(),
                    Some(content),
                    "cmd shim must round-trip through the console code page"
                );
            }
            None => assert_eq!(bytes, content.as_bytes()),
        }
        assert!(
            !is_foreign_file(&target),
            "code-page encoded shim must still be recognised as generated"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(windows)]
    fn ps1_shim_with_non_ascii_path_gets_utf8_bom() {
        let dir = temp_dir("non-ascii-ps1");
        let target = dir.join("pnpm.ps1");
        let content =
            "# DeepSeek Harness Desktop - pnpm command shim (generated)\r\n$pnpmBin = 'C:\\Users\\小蔡\\pnpm.cjs'\r\n";

        write_shim_file(&target, content).unwrap();

        let bytes = std::fs::read(&target).unwrap();
        assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF], "ps1 shim needs a utf-8 bom");
        assert_eq!(
            std::fs::read_to_string(&target).unwrap().trim_start_matches('\u{feff}'),
            content
        );
        assert!(!is_foreign_file(&target));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ascii_shim_bytes_are_unchanged() {
        let dir = temp_dir("ascii-bytes");
        let target = dir.join("pnpm.cmd");
        let content = "@echo off\r\nset \"PNPM_BIN=C:\\tools\\pnpm.cjs\"\r\n";

        write_shim_file(&target, content).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), content.as_bytes());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(windows)]
    fn code_page_encoded_shim_stays_overwritable() {
        let dir = temp_dir("codepage-overwrite");
        let target = dir.join("pnpm.cmd");
        let header = "@echo off\r\nrem DeepSeek Harness Desktop - pnpm command shim (generated)\r\n";
        write_shim_file(&target, &format!("{header}rem C:\\Users\\小蔡\r\n")).unwrap();
        assert!(!is_foreign_file(&target));

        write_shim_file(&target, &format!("{header}rem C:\\Users\\小蔡\\fixed\r\n")).unwrap();

        let codepage = crate::utils::console_code_page();
        let fixed = format!("{header}rem C:\\Users\\小蔡\\fixed\r\n");
        let expected = crate::utils::encode_multibyte(&fixed, codepage)
            .unwrap_or_else(|| fixed.as_bytes().to_vec());
        assert_eq!(std::fs::read(&target).unwrap(), expected);
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    #[cfg(windows)]
    fn write_shims_writes_npx_and_npm_shims() {
        let dir = temp_dir("write-npx-npm");
        let bin_dir = dir.join("bin");
        let mut context = mock_context(noop_assets());
        context.config_mut().identifier = "dsh-tauri".into();
        let app = mock_builder().build(context).unwrap();

        write_shims(app.handle(), &bin_dir).unwrap();

        for name in [
            "pnpm.cmd", "pnpm.ps1", "npx.cmd", "npx.ps1", "npm.cmd", "npm.ps1",
        ] {
            let path = bin_dir.join(name);
            assert!(path.is_file(), "{name} was not written");
            assert!(!is_foreign_file(&path), "{name} must be a generated shim");
        }

        let npx = std::fs::read_to_string(bin_dir.join("npx.cmd")).unwrap();
        assert!(
            npx.contains("npx-cli.js"),
            "npx.cmd must bake the npm cli entry: {npx}"
        );
        assert!(
            npx.contains("npx command shim (generated)"),
            "npx.cmd lost its generated marker: {npx}"
        );
        assert!(!bin_dir.join("dsh.cmd").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
