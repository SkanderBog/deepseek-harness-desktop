use std::path::{Path, PathBuf};

use super::bundled_dep_spec;
use super::bundled_plugin_dir;
use super::PreinstallPluginInfo;
use crate::{AppHandle};

pub(super) fn local_path_spec(spec: &str) -> Option<PathBuf> {
    let trimmed = unquote(spec.trim());
    if trimmed.is_empty() {
        return None;
    }
    for prefix in ["link:", "file:"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let rest = strip_file_url(rest);
            if rest.is_empty() {
                return None;
            }
            return Some(PathBuf::from(rest));
        }
    }
    if trimmed.starts_with('/') || trimmed.starts_with("\\\\") {
        return Some(PathBuf::from(trimmed));
    }
    if has_drive_prefix(trimmed) {
        return Some(PathBuf::from(trimmed));
    }
    if trimmed.starts_with("./") || trimmed.starts_with("../") {
        return Some(PathBuf::from(trimmed));
    }
    if trimmed.starts_with(".\\") || trimmed.starts_with("..\\") {
        return Some(PathBuf::from(trimmed));
    }
    None
}

pub(crate) fn local_spec_from_path(path: &Path) -> String {
    format!("link:{}", forward_slashes(path))
}

fn unquote(spec: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(rest) = spec
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return rest.trim();
        }
    }
    spec
}

fn has_drive_prefix(spec: &str) -> bool {
    let bytes = spec.as_bytes();
    bytes.len() > 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'/' || bytes[2] == b'\\')
}

fn strip_file_url(rest: &str) -> &str {
    let trimmed = rest.trim();
    let stripped = trimmed.trim_start_matches('/');
    if has_drive_prefix(stripped) {
        stripped
    } else {
        trimmed
    }
}

pub(super) fn local_dir_of(spec: &str, base: &Path) -> Option<PathBuf> {
    let path = local_path_spec(spec)?;
    let joined = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    Some(
        std::fs::canonicalize(&joined)
            .map(|resolved| dunce::simplified(&resolved).to_path_buf())
            .unwrap_or_else(|_| dunce::simplified(&joined).to_path_buf()),
    )
}

pub(super) fn forward_slashes(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string()
}

pub(super) fn package_name_of_spec(spec: &str, base: &Path) -> Option<String> {
    if let Some(dir) = local_dir_of(spec, base) {
        return dir.is_dir().then(|| local_package_name(&dir)).flatten();
    }
    if spec.contains(':') || spec.contains(char::is_whitespace) {
        return None;
    }
    let name = package_name_of_npm_spec(spec);
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

pub(super) fn local_package_name(path: &Path) -> Option<String> {
    if let Ok(content) = std::fs::read_to_string(path.join("package.json")) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(name) = value.get("name").and_then(|v| v.as_str()) {
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    path.file_name().map(|s| s.to_string_lossy().into_owned())
}

fn package_name_of_npm_spec(spec: &str) -> &str {
    let search_from = if spec.starts_with('@') {
        spec.find('/').map_or(spec.len(), |i| i + 1)
    } else {
        0
    };
    match spec[search_from..].find('@') {
        Some(offset) => spec[..search_from + offset].trim_end_matches('@'),
        None => spec,
    }
}

pub(super) fn bundled_dir_of(
    app_handle: &AppHandle,
    preset: &PreinstallPluginInfo,
) -> Option<PathBuf> {
    if !preset.internal {
        return None;
    }
    bundled_plugin_dir(app_handle, &preset.id)
}

pub(super) fn preset_spec_for_install(
    preset: &PreinstallPluginInfo,
    bundled_dir: Option<PathBuf>,
    core_version: Option<&str>,
) -> Result<String, String> {
    if !preset.internal {
        return Ok(pinned_spec(preset, core_version));
    }
    let dir = bundled_dir.ok_or_else(|| {
        format!(
            "BUNDLED_PLUGIN_MISSING: no bundled dir for internal plugin {} (run pnpm build:plugins at build time)",
            preset.id
        )
    })?;
    Ok(bundled_dep_spec(&dir))
}

fn pinned_spec(preset: &PreinstallPluginInfo, core_version: Option<&str>) -> String {
    let Some(req) = preset
        .version
        .as_ref()
        .and_then(|version| version.plugin_req_for_core(core_version))
    else {
        return preset.spec.clone();
    };
    if !is_bare_package_spec(&preset.spec) {
        return preset.spec.clone();
    }
    format!("{}@{req}", preset.spec)
}

fn is_bare_package_spec(spec: &str) -> bool {
    if spec.contains(':') || spec.contains(char::is_whitespace) {
        return false;
    }
    let body = spec.strip_prefix('@').unwrap_or(spec);
    !body.is_empty() && !body.contains('@')
}

/// 是已知缺陷（issue #3948 / #7243 / #13276，官方已 accepted 仍未修）。公开仓库
/// 无 SSH 密钥的桌面机上同样必败（issue #369 的次要问题），一并改写为 HTTPS。
pub(super) fn normalize_git_spec(spec: &str) -> String {
    let rest = spec
        .strip_prefix("github:")
        .or_else(|| spec.strip_prefix("git+ssh://git@github.com/"));
    let Some(rest) = rest else {
        return spec.to_string();
    };
    let (path, fragment) = match rest.split_once('#') {
        Some((p, f)) => (p.trim_end_matches('/'), Some(f)),
        None => (rest.trim_end_matches('/'), None),
    };
    let mut repo = path.to_string();
    if !repo.ends_with(".git") {
        repo.push_str(".git");
    }
    let mut url = format!("git+https://github.com/{repo}");
    if let Some(fragment) = fragment {
        url.push('#');
        url.push_str(fragment);
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::manifest::{PluginVersion, VersionPair};
    use std::path::PathBuf;

    fn preset(id: &str, spec: &str, internal: bool) -> PreinstallPluginInfo {
        PreinstallPluginInfo {
            id: id.into(),
            spec: spec.into(),
            package: None,
            name: String::new(),
            description: String::new(),
            repo_url: String::new(),
            recommended: false,
            fix: false,
            default_checked: false,
            default_unchecked: false,
            version: None,
            win_only: false,
            internal,
        }
    }

    #[test]
    fn install_spec_passthrough_for_regular_preset() {
        let p = preset("dshmarket", "dshmarket", false);
        assert_eq!(
            preset_spec_for_install(&p, None, None).unwrap(),
            "dshmarket"
        );
        assert_eq!(
            preset_spec_for_install(&p, Some(PathBuf::from("/ignored")), None).unwrap(),
            "dshmarket"
        );
    }

    #[test]
    fn install_spec_pins_recommended_range_for_current_core() {
        let mut p = preset("dsh-better-sidebar", "dsh-better-sidebar", false);
        p.version = Some(PluginVersion::Matrix(vec![
            VersionPair {
                version: "^0.21.1".into(),
                dsh: "^0.1.7-rc.1".into(),
            },
            VersionPair {
                version: "^0.22.0".into(),
                dsh: "^0.2.1-rc.1".into(),
            },
        ]));
        assert_eq!(
            preset_spec_for_install(&p, None, Some("0.1.7-rc.1")).unwrap(),
            "dsh-better-sidebar@^0.21.1"
        );
        assert_eq!(
            preset_spec_for_install(&p, None, Some("0.2.1-rc.1")).unwrap(),
            "dsh-better-sidebar@^0.22.0"
        );
        let mut scoped = preset("@xmanrui/dsh-im", "@xmanrui/dsh-im", false);
        scoped.version = p.version.clone();
        assert_eq!(
            preset_spec_for_install(&scoped, None, Some("0.1.7-rc.2")).unwrap(),
            "@xmanrui/dsh-im@^0.21.1"
        );
        assert_eq!(
            preset_spec_for_install(&p, None, Some("0.2.0")).unwrap(),
            "dsh-better-sidebar"
        );
        assert_eq!(
            preset_spec_for_install(&p, None, None).unwrap(),
            "dsh-better-sidebar"
        );

        // issue #715：清单区间本身就带 `^`，拼接只能再加一个 `@`——出现 `^^` 会让 pnpm
        // 调用，一个坏 spec 就让整批预设都装不上（应用卡在加载中，见 issue #716）。
        for core in [
            "0.1.7-rc.1",
            "0.1.7-rc.2",
            "0.2.1-rc.1",
            "0.3.0",
        ] {
            let spec = preset_spec_for_install(&p, None, Some(core)).unwrap();
            assert!(!spec.contains("^^"), "core={core}: {spec}");
        }
    }

    #[test]
    fn install_spec_never_pins_non_package_specs() {
        let mut git = preset("probe", "git+https://github.com/o/r.git", false);
        git.version = Some(PluginVersion::Matrix(vec![VersionPair {
            version: "^1.2.0".into(),
            dsh: "^0.1.7-rc.1".into(),
        }]));
        assert_eq!(
            preset_spec_for_install(&git, None, Some("0.1.7-rc.1")).unwrap(),
            "git+https://github.com/o/r.git"
        );

        let mut versioned = preset("probe", "probe@1.0.0", false);
        versioned.version = git.version.clone();
        assert_eq!(
            preset_spec_for_install(&versioned, None, Some("0.1.7-rc.1")).unwrap(),
            "probe@1.0.0"
        );

        let mut link = preset("probe", "link:C:/deps/probe", false);
        link.version = git.version.clone();
        assert_eq!(
            preset_spec_for_install(&link, None, Some("0.1.7-rc.1")).unwrap(),
            "link:C:/deps/probe"
        );
    }

    #[test]
    fn install_spec_supports_npm_style_whitespace_ranges() {
        let mut p = preset("probe", "probe", false);
        p.version = Some(PluginVersion::Matrix(vec![VersionPair {
            version: "^1.2.0".into(),
            dsh: ">=0.1.7-rc.1 <0.1.7-rc.5".into(),
        }]));
        assert_eq!(
            preset_spec_for_install(&p, None, Some("0.1.7-rc.3")).unwrap(),
            "probe@^1.2.0"
        );
        assert_eq!(
            preset_spec_for_install(&p, None, Some("0.1.7-rc.5")).unwrap(),
            "probe"
        );
    }

    #[test]
    fn install_spec_uses_bundled_dir_for_internal_preset() {
        let p = preset("dsh-tauri", "dsh-tauri@0.2.0", true);
        let dir = PathBuf::from("C:\\Apps\\dsh\\resources\\internal-plugins\\dsh-tauri");
        assert_eq!(
            preset_spec_for_install(&p, Some(dir), None).unwrap(),
            "link:C:/Apps/dsh/resources/internal-plugins/dsh-tauri"
        );
    }

    #[test]
    fn install_spec_keeps_space_paths_bare_for_supported_cores() {
        let preset = preset("dsh-tauri", "dsh-tauri", true);
        for core in [
            Some("0.1.7-rc.1"),
            Some("0.2.1-rc.1"),
            None,
        ] {
            for (path, expected) in [
                (
                    "C:/Program Files/DeepSeek Harness Desktop/resources/node_modules/dsh-tauri",
                    "link:C:/Program Files/DeepSeek Harness Desktop/resources/node_modules/dsh-tauri",
                ),
                (
                    "/Applications/DeepSeek Harness Desktop.app/Contents/Resources/resources/node_modules/dsh-tauri",
                    "link:/Applications/DeepSeek Harness Desktop.app/Contents/Resources/resources/node_modules/dsh-tauri",
                ),
            ] {
                assert_eq!(
                    preset_spec_for_install(&preset, Some(PathBuf::from(path)), core).unwrap(),
                    expected
                );
            }
        }
    }

    #[test]
    fn install_spec_errors_when_internal_bundle_missing() {
        let p = preset("dsh-tauri", "dsh-tauri@0.2.0", true);
        let err = preset_spec_for_install(&p, None, None).unwrap_err();
        assert!(err.starts_with("BUNDLED_PLUGIN_MISSING"));
        assert!(err.contains("dsh-tauri"));
    }

    // ---- git GitHub 简写规范化（issue #51 根因绕行）----

    #[test]
    fn normalize_github_shorthand_to_git_https() {
        assert_eq!(
            normalize_git_spec("github:baihejiangnan/dsh-session-context-menu"),
            "git+https://github.com/baihejiangnan/dsh-session-context-menu.git"
        );
    }

    #[test]
    fn normalize_github_shorthand_preserves_ref_and_dedup_git_suffix() {
        assert_eq!(
            normalize_git_spec("github:omdsh-dev/DSH-better-sidebar#next"),
            "git+https://github.com/omdsh-dev/DSH-better-sidebar.git#next"
        );
        assert_eq!(
            normalize_git_spec("github:user/repo.git"),
            "git+https://github.com/user/repo.git"
        );
        assert_eq!(
            normalize_git_spec("github:user/repo/"),
            "git+https://github.com/user/repo.git"
        );
    }

    #[test]
    fn normalize_ssh_github_url_to_git_https() {
        // issue #369：裸 git+ssh://git@github.com/... spec（pnpm-lock 常见形态）
        assert_eq!(
            normalize_git_spec("git+ssh://git@github.com/omdsh-dev/DSH-better-sidebar.git"),
            "git+https://github.com/omdsh-dev/DSH-better-sidebar.git"
        );
        assert_eq!(
            normalize_git_spec("git+ssh://git@github.com/omdsh-dev/DSH-better-sidebar#next"),
            "git+https://github.com/omdsh-dev/DSH-better-sidebar.git#next"
        );
    }

    #[test]
    fn normalize_non_github_spec_passes_through() {
        assert_eq!(normalize_git_spec("dshmarket"), "dshmarket");
        assert_eq!(
            normalize_git_spec("git+https://github.com/foo/bar.git"),
            "git+https://github.com/foo/bar.git"
        );
        assert_eq!(
            normalize_git_spec("git+ssh://git@gitlab.com/foo/bar.git"),
            "git+ssh://git@gitlab.com/foo/bar.git"
        );
    }

    fn temp_plugin_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dsh-spec-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        canonical(&dir)
    }

    fn canonical(path: &Path) -> PathBuf {
        dunce::simplified(&std::fs::canonicalize(path).unwrap()).to_path_buf()
    }

    #[test]
    fn local_path_spec_recognizes_every_local_form() {
        for spec in [
            "link:D:/plugins/probe",
            "file:D:/plugins/probe",
            "file:///D:/plugins/probe",
            "D:\\plugins\\probe",
            "D:/plugins/probe",
            "./plugins/probe",
            "../plugins/probe",
            ".\\plugins\\probe",
            "..\\plugins\\probe",
            "/opt/dsh/plugins/probe",
            "link:/opt/dsh/plugins/probe",
            "file:///opt/dsh/plugins/probe",
            "\\\\server\\share\\probe",
            "  D:/plugins/probe  ",
        ] {
            assert!(local_path_spec(spec).is_some(), "应识别为本地 spec: {spec}");
        }
        assert_eq!(
            local_path_spec("file:///D:/plugins/probe"),
            Some(PathBuf::from("D:/plugins/probe"))
        );
    }

    #[test]
    fn local_path_spec_ignores_registry_and_git_specs() {
        for spec in [
            "dshmarket",
            "@scope/dsh-plugin",
            "dshmarket@^2.12.0",
            "@scope/dsh-plugin@1.0.0",
            "github:user/repo#next",
            "git+https://github.com/user/repo.git",
            "git+ssh://git@gitlab.com/user/repo.git",
            "https://example.com/x.tgz",
            "",
            "   ",
            "link:",
            "file:",
        ] {
            assert!(
                local_path_spec(spec).is_none(),
                "不得识别为本地 spec: {spec}"
            );
        }
    }

    #[test]
    fn local_path_spec_strips_pasted_quotes() {
        assert_eq!(
            local_path_spec("\"D:/plugins/probe\""),
            Some(PathBuf::from("D:/plugins/probe"))
        );
        assert_eq!(
            local_path_spec("'D:/plugins/probe'"),
            Some(PathBuf::from("D:/plugins/probe"))
        );
        assert!(local_path_spec("\"\"").is_none());
    }

    #[test]
    fn local_dir_of_resolves_relative_against_the_install_cwd() {
        let base = temp_plugin_dir("base");
        let dir = base.join("plugins").join("probe");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("package.json"),
            "{\"name\":\"dsh-probe\",\"version\":\"1.2.3\"}",
        )
        .unwrap();

        assert_eq!(local_dir_of("./plugins/probe", &base), Some(dir.clone()));
        assert_eq!(
            local_dir_of("plugins/probe", &base),
            None,
            "裸相对路径不是本地 spec，须显式 ./ 或 ../"
        );
        assert_eq!(
            local_dir_of(&format!("link:{}", forward_slashes(&dir)), &base),
            Some(dir.clone())
        );
        assert_eq!(
            local_dir_of(&format!("link:{}/", forward_slashes(&dir)), &base),
            Some(dir.clone()),
            "尾斜杠要剥掉"
        );
        assert_eq!(
            local_dir_of(&format!("link:{}", dir.display()), &base),
            Some(dir.clone()),
            "原生分隔符要归一到同一真实路径"
        );

        #[cfg(windows)]
        assert_eq!(
            local_dir_of(&format!("file:///{}", forward_slashes(&dir)), &base),
            Some(dir.clone()),
            "file:/// 的盘符还原只在 Windows 成立（Unix 上 /D:/x 不是绝对路径）"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn local_dir_of_keeps_missing_paths_for_the_caller_to_reject() {
        let base = temp_plugin_dir("missing");
        assert_eq!(
            local_dir_of("./nowhere/", &base),
            Some(base.join("nowhere"))
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn package_name_of_spec_reads_local_manifest_then_falls_back_to_dir_name() {
        let base = temp_plugin_dir("name");
        let named = base.join("weird-dir-name");
        std::fs::create_dir_all(&named).unwrap();
        std::fs::write(
            named.join("package.json"),
            "{\"name\":\"dsh-reverse-skill\"}",
        )
        .unwrap();
        assert_eq!(
            package_name_of_spec(&format!("link:{}", forward_slashes(&named)), &base),
            Some("dsh-reverse-skill".to_string())
        );
        assert_eq!(
            package_name_of_spec("./weird-dir-name", &base),
            Some("dsh-reverse-skill".to_string())
        );

        let bare = base.join("bare-dir");
        std::fs::create_dir_all(&bare).unwrap();
        assert_eq!(
            package_name_of_spec("./bare-dir", &base),
            Some("bare-dir".to_string())
        );
        assert_eq!(package_name_of_spec("./never-existed", &base), None);

        assert_eq!(
            package_name_of_spec("dshmarket@^2.12.0", &base),
            Some("dshmarket".to_string())
        );
        assert_eq!(
            package_name_of_spec("@scope/dsh-plugin@1.0.0", &base),
            Some("@scope/dsh-plugin".to_string())
        );
        assert_eq!(
            package_name_of_spec("github:user/repo", &base),
            None,
            "含 `:` 且非本地形态：装后目录名不可静态得知"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn forward_slashes_normalizes_separators_and_trailing_slash() {
        assert_eq!(forward_slashes(Path::new("D:\\a\\b\\")), "D:/a/b");
        assert_eq!(forward_slashes(Path::new("D:/a/b/")), "D:/a/b");
        assert_eq!(forward_slashes(Path::new("D:\\a")), "D:/a");
    }
}
