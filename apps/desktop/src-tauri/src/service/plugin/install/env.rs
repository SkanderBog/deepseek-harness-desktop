use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::AppHandle;
use crate::config;
use crate::service::cli;

pub(crate) fn build_plugin_envs(
    app_handle: &AppHandle,
    prefer_bundled_pnpm: bool,
) -> HashMap<String, String> {
    let node = config::get_node_binary_path(app_handle);
    // （issue #121 的 "Node.js runtime not found"）。已存在（预检通过）的
    let node_abs = dunce::canonicalize(&node).unwrap_or_else(|_| node.clone());
    let bin_dir = cli::get_bin_dir(app_handle);
    let mut envs = HashMap::from([
        (
            "DSH_HOME".to_string(),
            config::get_dsh_data_path(app_handle)
                .to_string_lossy()
                .into_owned(),
        ),
        ("DSH_TELEMETRY_DISABLED".to_string(), "1".to_string()),
        ("NO_COLOR".to_string(), "1".to_string()),
        ("CI".to_string(), "true".to_string()),
        // "Node.js runtime not found" 而应用预检却通过（issue #121）。
        (
            "DSH_NODE".to_string(),
            node_abs.to_string_lossy().into_owned(),
        ),
    ]);
    if prefer_bundled_pnpm {
        envs.insert("DSH_PREFER_BUNDLED_PNPM".to_string(), "1".to_string());
    } else if let Some(pnpm) = cli::find_user_pnpm(app_handle) {
        if let Some(pnpm_value) = cli::pnpm_env_value(&pnpm, &bin_dir) {
            envs.insert("DSH_PNPM".to_string(), pnpm_value);
        }
    }

    let bundled_pnpm = config::get_pnpm_binary_path(app_handle);
    if bundled_pnpm.exists() {
        envs.insert(
            "DSH_PNPM_BIN".to_string(),
            bundled_pnpm.to_string_lossy().into_owned(),
        );
    }

    if let Some(store_dir) = super::pnpm::profile_store_base_dir(app_handle) {
        log::info!("pinning plugin install pnpm store to the profile record: {store_dir}");
        envs.insert("npm_config_store_dir".to_string(), store_dir.clone());
        envs.insert("pnpm_config_store_dir".to_string(), store_dir);
    }

    let mut paths = vec![bin_dir];
    if let Some(node_dir) = node_abs.parent() {
        paths.push(node_dir.to_path_buf());
    }
    if let Some(git_dir) = config::get_git_cmd_dir(app_handle) {
        paths.push(git_dir);
    }
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));

    if let Ok(joined) = std::env::join_paths(paths) {
        envs.insert("PATH".to_string(), joined.to_string_lossy().into_owned());
    }
    // 桌面端代理设置（issue #110）：核心只在启动时读 `http_proxy` 系环境变量，
    envs.extend(config::proxy::proxy_child_env(
        &config::get_store_dat_setting(app_handle).proxy_url,
    ));
    envs.extend(git_https_isolation_env(app_handle));
    envs
}

fn git_https_isolation_env(app_handle: &AppHandle) -> HashMap<String, String> {
    let Some(git) = plugin_git_binary(app_handle) else {
        return HashMap::new();
    };
    let mut envs = HashMap::new();
    if git_scope_has_github_ssh_rewrite(&git, &["--global"]) {
        if let Some(config) = empty_git_config_file() {
            log::info!(
                "git config rewrites GitHub HTTPS to SSH (global); isolating GIT_CONFIG_GLOBAL to force HTTPS"
            );
            envs.insert(
                "GIT_CONFIG_GLOBAL".to_string(),
                config.to_string_lossy().into_owned(),
            );
        }
    }
    if git_scope_has_github_ssh_rewrite(&git, &["--system"]) {
        log::info!(
            "git config rewrites GitHub HTTPS to SSH (system); disabling system git config to force HTTPS"
        );
        envs.insert("GIT_CONFIG_NOSYSTEM".to_string(), "1".to_string());
    }
    envs
}

fn plugin_git_binary(_app_handle: &AppHandle) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        config::get_git_cmd_dir(_app_handle).map(|dir| dir.join("git.exe"))
    }
    #[cfg(not(windows))]
    {
        Some(PathBuf::from("git"))
    }
}

/// 提示（issue #369）。这里实际探测 git 可执行能否运行，供安装 git 托管插件前
pub(crate) fn git_available(app_handle: &AppHandle) -> bool {
    let Some(git) = plugin_git_binary(app_handle) else {
        return false;
    };
    let mut cmd = std::process::Command::new(&git);
    cmd.arg("--version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn git_scope_has_github_ssh_rewrite(git: &Path, scope: &[&str]) -> bool {
    let mut cmd = std::process::Command::new(git);
    cmd.args(scope).args(["--get-regexp", "^url\\."]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let Ok(output) = cmd.output() else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    has_github_ssh_rewrite_in_output(&String::from_utf8_lossy(&output.stdout))
}

fn has_github_ssh_rewrite_in_output(output: &str) -> bool {
    output.lines().any(rewrite_rule_targets_ssh_github)
}

fn rewrite_rule_targets_ssh_github(line: &str) -> bool {
    let mut parts = line.splitn(2, ' ');
    let (key, value) = match (parts.next(), parts.next()) {
        (Some(k), Some(v)) => (k.to_ascii_lowercase(), v.trim()),
        _ => return false,
    };
    let Some(base) = key
        .strip_prefix("url.")
        .and_then(|k| k.strip_suffix(".insteadof"))
    else {
        return false;
    };
    let ssh_base =
        base.starts_with("git@") || base.starts_with("ssh://") || base.starts_with("git+ssh://");
    ssh_base && github_http_host(value)
}

fn github_http_host(value: &str) -> bool {
    let value = value.trim();
    let scheme_end = value.find("://").map(|i| i + 3).unwrap_or(0);
    let host = value[scheme_end..]
        .split('/')
        .next()
        .unwrap_or(&value[scheme_end..]);
    host == "github.com" || host == "www.github.com"
}

fn empty_git_config_file() -> Option<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "dsh-gitconfig-{}-{nanos:x}.empty",
        std::process::id()
    ));
    std::fs::write(&path, "").ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_rule_targets_ssh_github_detects_common_rewrites() {
        assert!(rewrite_rule_targets_ssh_github(
            "url.git@github.com:.insteadof https://github.com/"
        ));
        assert!(rewrite_rule_targets_ssh_github(
            "url.ssh://git@github.com/.insteadof https://github.com/"
        ));
        assert!(rewrite_rule_targets_ssh_github(
            "url.git+ssh://git@github.com/.insteadof https://github.com"
        ));
        assert!(rewrite_rule_targets_ssh_github(
            "url.git@github.com:.insteadof http://github.com"
        ));
        assert!(rewrite_rule_targets_ssh_github(
            "url.git@Github.com:.insteadOf https://github.com/"
        ));
    }

    #[test]
    fn rewrite_rule_targets_ssh_github_ignores_harmless_rules() {
        assert!(!rewrite_rule_targets_ssh_github(
            "url.https://github.com/.insteadof git@github.com:"
        ));
        assert!(!rewrite_rule_targets_ssh_github(
            "url.git@gitlab.example.com:.insteadof https://gitlab.example.com/"
        ));
        assert!(!rewrite_rule_targets_ssh_github(
            "url.ssh://git@github.com/.pushinsteadof https://github.com/"
        ));
        assert!(!rewrite_rule_targets_ssh_github(
            "url.file:///c:/git/.insteadof https://github.com/"
        ));
        assert!(!rewrite_rule_targets_ssh_github("core.autocrlf true"));
        assert!(!rewrite_rule_targets_ssh_github(""));
        assert!(!rewrite_rule_targets_ssh_github("   "));
    }

    #[test]
    fn has_github_ssh_rewrite_in_output_checks_all_lines() {
        let harmful =
            "core.autocrlf true\nuser.name=x\nurl.git@github.com:.insteadof https://github.com/\n";
        assert!(has_github_ssh_rewrite_in_output(harmful));
        let clean =
            "core.autocrlf true\nuser.name=x\nurl.https://github.com/.insteadof git@github.com:\n";
        assert!(!has_github_ssh_rewrite_in_output(clean));
        assert!(!has_github_ssh_rewrite_in_output(""));
    }
}
