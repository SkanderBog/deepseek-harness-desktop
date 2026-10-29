
pub(super) fn diagnostic_suffix(detail: &str) -> String {
    if detail.is_empty() {
        String::new()
    } else {
        format!(": {detail}")
    }
}

const ERROR_MARKERS: [&str; 14] = [
    "ERR_",
    "error",
    "Error",
    "failed",
    "✖",
    "warning",
    "not recognized",
    "not found",
    "No such file",
    "cannot find",
    "Cannot find",
    "找不到",
    "无法",
    "不是内部或外部命令",
];

pub(super) fn pick_error_message(output: &str, hint: Option<&str>) -> String {
    if let Some(hint) = hint {
        return hint.to_string();
    }
    let cleaned: Vec<String> = output
        .split('\n')
        .filter_map(|line| {
            let trimmed = strip_ansi(line);
            let trimmed = trimmed.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .filter(|line| {
            ERROR_MARKERS.iter().any(|marker| line.contains(marker))
                || line.split_whitespace().any(|word| {
                    let code = word
                        .strip_prefix('[')
                        .and_then(|word| word.strip_suffix(']'))
                        .or_else(|| word.strip_suffix(':'));
                    code.is_some_and(|code| {
                        code.len() > 1
                            && code.starts_with('E')
                            && code.bytes().all(|byte| {
                                byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                            })
                    })
                })
        })
        .take(8)
        .collect();
    let base = if cleaned.is_empty() {
        output.trim().to_string()
    } else {
        cleaned.join("\n")
    };
    base.chars().take(2000).collect()
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // '['
            while let Some(&n) = chars.peek() {
                if n.is_ascii_digit() || n == ';' {
                    chars.next();
                } else {
                    break;
                }
            }
            if chars.peek() == Some(&'m') {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PolicyBlockedVersion {
    pub name: String,
    pub version: String,
}

///   插件操作都在这里硬失败（issue #222 的另一面）。
pub(super) fn policy_blocked_versions(output: &str) -> Vec<PolicyBlockedVersion> {
    let mut found: Vec<PolicyBlockedVersion> = Vec::new();
    for line in output.split('\n') {
        let stripped = strip_ansi(line);
        let Some((key, rest)) = stripped.split_once(" was published at ") else {
            continue;
        };
        if !rest.contains("within the minimumReleaseAge cutoff") {
            continue;
        }
        let Some((name, version)) = key.trim().rsplit_once('@') else {
            continue;
        };
        let name = name.trim();
        let version = version.trim();
        if name.is_empty() || version.is_empty() {
            continue;
        }
        let entry = PolicyBlockedVersion {
            name: name.to_string(),
            version: version.to_string(),
        };
        if !found.contains(&entry) {
            found.push(entry);
        }
    }
    found
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IncompatibleVersion {
    pub name: String,
    pub version: String,
    pub runtime_version: String,
}

pub(super) fn incompatible_versions(output: &str) -> Vec<IncompatibleVersion> {
    let mut found: Vec<IncompatibleVersion> = Vec::new();
    for line in output.split('\n') {
        let stripped = strip_ansi(line);
        let Some(start) = stripped.find("Plugin ") else {
            continue;
        };
        let Some((key, rest)) = stripped[start + "Plugin ".len()..].split_once(" is incompatible with dsh ")
        else {
            continue;
        };
        let Some((name, version)) = key.rsplit_once('@') else {
            continue;
        };
        let runtime_version = rest.split(':').next().unwrap_or_default().trim();
        if name.is_empty()
            || version.is_empty()
            || !runtime_version.starts_with(|c: char| c.is_ascii_digit())
        {
            continue;
        }
        let entry = IncompatibleVersion {
            name: name.to_string(),
            version: version.to_string(),
            runtime_version: runtime_version.to_string(),
        };
        if !found.contains(&entry) {
            found.push(entry);
        }
    }
    found
}

pub(super) fn network_error_hint(output: &str) -> Option<&'static str> {
    const SIGNALS: &[&str] = &[
        "eai_again",
        "enotfound",
        "econnrefused",
        "econnreset",
        "etimedout",
        "network timeout",
        "network request failed",
        "fetch failed",
        "unable to verify the first certificate",
        "self signed certificate",
        "socket hang up",
        "could not resolve host",
        "failed to connect",
        "connection timed out",
        "connection reset",
    ];
    let lower = output.to_ascii_lowercase();
    SIGNALS
        .iter()
        .any(|signal| lower.contains(signal))
        .then_some("网络连接失败，请检查网络或代理设置后重试。")
}

pub(super) fn policy_verification_network_failure(output: &str) -> bool {
    if !policy_blocked_versions(output).is_empty() {
        return false;
    }
    let lower = output.to_ascii_lowercase();
    if !lower.contains("err_pnpm_minimum_release_age_violation") {
        return false;
    }
    const FETCH_FAILURES: [&str; 4] = [
        "error (unknown)",
        "will retry in",
        "fetch failed",
        "failed to fetch",
    ];
    FETCH_FAILURES.iter().any(|signal| lower.contains(signal))
}

pub(super) fn git_transport_hint(output: &str) -> Option<&'static str> {
    const SIGNALS: &[(&str, &str)] = &[
        (
            "spawn git enoent",
            "Git is not installed or not on PATH (pnpm could not spawn git: ENOENT). Install git (e.g. Debian/Ubuntu: `sudo apt install git`; macOS: `brew install git`) or uncheck git-hosted plugins and retry.",
        ),
        (
            "command failed with enoent: git",
            "Git is not installed or not on PATH (pnpm could not run git: ENOENT). Install git (e.g. Debian/Ubuntu: `sudo apt install git`; macOS: `brew install git`) or uncheck git-hosted plugins and retry.",
        ),
        (
            "host key verification failed",
            "git fell back to SSH and could not verify GitHub's host key (no known_hosts entry; the process ran non-interactively). Make sure GitHub is reachable over HTTPS.",
        ),
        (
            "permission denied (publickey)",
            "git reached GitHub over SSH instead of HTTPS (Permission denied (publickey)) — usually your git config rewrites GitHub HTTPS to SSH (url.<base>.insteadOf) while no SSH key is configured. The desktop app isolates git config to force HTTPS for plugin installs; if you still see this, remove that rewrite (git config --global --unset-all 'url.git@github.com:.insteadOf') or configure an SSH key.",
        ),
        (
            "could not read from remote repository",
            "pnpm could not read from the git remote — commonly a git+ssh transport failure. Ensure GitHub is reachable over HTTPS.",
        ),
        (
            "ssh: connect to host",
            "pnpm tried to reach GitHub over SSH (port 22) and the connection was refused. Use HTTPS instead.",
        ),
    ];
    let lower = output.to_ascii_lowercase();
    SIGNALS
        .iter()
        .find(|(sig, _)| lower.contains(sig))
        .map(|(_, hint)| *hint)
}

const RECORDED_PATH_MARKERS: &[&str] = &[
    "currently linked from the store at ",
    "symlinked from the virtual store directory at ",
];
const CURRENT_PATH_MARKERS: &[&str] = &[
    "now wants to use the store at ",
    "now wants to use the virtual store at ",
];

pub(super) fn store_mismatch_hint(output: &str) -> Option<String> {
    const CODES: &[&str] = &[
        "ERR_PNPM_UNEXPECTED_STORE",
        "ERR_PNPM_UNEXPECTED_VIRTUAL_STORE",
        "ERR_PNPM_STORE_BREAKING_CHANGE",
        "ERR_PNPM_MODULES_BREAKING_CHANGE",
    ];
    if !CODES.iter().any(|code| contains_error_code(output, code)) {
        return None;
    }
    Some(match (
        quoted_after(output, RECORDED_PATH_MARKERS),
        quoted_after(output, CURRENT_PATH_MARKERS),
    ) {
        (Some(recorded), Some(current)) => format!(
            "The profile's node_modules was created by a different pnpm major version: it is linked from the store at \"{recorded}\", but the pnpm now in use resolves the store at \"{current}\". Install/update cannot succeed until those match — install the pnpm major version that created this profile, or delete the profile's node_modules directory and retry so pnpm rebuilds it with the current version."
        ),
        _ => "The profile's node_modules is not compatible with the pnpm version now in use (pnpm refused before installing). Install the pnpm major version that created this profile, or delete the profile's node_modules directory and retry so pnpm rebuilds it with the current version.".to_string(),
    })
}

fn quoted_after(output: &str, markers: &[&str]) -> Option<String> {
    let (index, marker) = markers
        .iter()
        .filter_map(|marker| output.find(marker).map(|index| (index, *marker)))
        .min_by_key(|(index, _)| *index)?;
    let rest = output[index + marker.len()..].strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn contains_error_code(output: &str, code: &str) -> bool {
    let upper = output.to_ascii_uppercase();
    let mut from = 0;
    while let Some(index) = upper[from..].find(code) {
        let end = from + index + code.len();
        let boundary = upper[end..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
        if boundary {
            return true;
        }
        from = from + index + 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_error_message_preserves_filesystem_cause_before_cli_summary() {
        for cause in [
            "[EISDIR] EISDIR: illegal operation on a directory, readlink 'F:/apps/.dsh/profiles/tauri/node_modules/dsh-tauri'",
            "EPERM: operation not permitted, unlink 'C:/apps/plugin/package.json'",
            "[ENOTEMPTY] ENOTEMPTY: directory not empty, rmdir 'C:/apps/plugin'",
            "EACCES: permission denied, open '/tmp/profile/package.json'",
            "ENOSPC: no space left on device, write",
        ] {
            let output = format!(
                "Progress: resolved 1, reused 0\n\u{1b}[31m{cause}\u{1b}[0m\ndsh: plugin command failed; diagnostics: C:/profile/.plugin-manager/logs/operation-test/pnpm.log\n"
            );
            assert_eq!(
                pick_error_message(&output, None),
                format!("{cause}\ndsh: plugin command failed; diagnostics: C:/profile/.plugin-manager/logs/operation-test/pnpm.log")
            );
        }
    }

    #[test]
    fn pick_error_message_does_not_promote_errno_in_progress_paths() {
        let output = "Progress: unpacking C:/EISDIR/cache\nResolved EACCES-helper@1.0.0\ndsh: plugin command failed; diagnostics: C:/profile/pnpm.log\n";
        assert_eq!(
            pick_error_message(output, None),
            "dsh: plugin command failed; diagnostics: C:/profile/pnpm.log"
        );
    }

    #[test]
    fn diagnostic_suffix_preserves_non_allowbuilds_failure() {
        assert_eq!(diagnostic_suffix(""), "");
        assert_eq!(
            diagnostic_suffix("ERR_PNPM_LINKING_FAILED: stale symlink"),
            ": ERR_PNPM_LINKING_FAILED: stale symlink"
        );
    }

    const INCOMPATIBLE_OUTPUT: &str = "\ndsh: installation rejected: Plugin dsh-better-sidebar@0.22.1 is incompatible with dsh 0.2.0-rc.1: peerDependencies {\"@deepseek-ai/dsh-llm\":\"^0.1.7-rc.1\"}. Running it may cause crashes or data loss. Update the plugin or install a plugin version compatible with this dsh runtime.\ndsh: nothing was installed.\ndsh: to accept the risk, run: dsh plugin --profile core-020 allow-version dsh-better-sidebar@0.22.1 --dsh-version 0.2.0-rc.1 --accept-risk\n";

    #[test]
    fn incompatible_versions_extracts_exact_grantable_triple() {
        assert_eq!(
            incompatible_versions(INCOMPATIBLE_OUTPUT),
            vec![IncompatibleVersion {
                name: "dsh-better-sidebar".to_string(),
                version: "0.22.1".to_string(),
                runtime_version: "0.2.0-rc.1".to_string(),
            }]
        );
    }

    #[test]
    fn incompatible_versions_reads_header_line_and_own_lines_together() {
        let out = concat!(
            "\ndsh: installation rejected: Plugin dsh-better-sidebar@0.22.1 is incompatible with dsh 0.2.0-rc.1: peerDependencies {\"@deepseek-ai/dsh-llm\":\"^0.1.7-rc.1\"}. Running it may cause crashes or data loss.\n",
            "Plugin dsh-rewind-plugin@0.14.0 is incompatible with dsh 0.2.0-rc.1: peerDependencies {\"@deepseek-ai/dsh-fs\":\"^0.1.7-rc.2\"}. Running it may cause crashes or data loss.\n",
            "dsh: nothing was installed.\n",
        );
        assert_eq!(
            incompatible_versions(out),
            vec![
                IncompatibleVersion {
                    name: "dsh-better-sidebar".to_string(),
                    version: "0.22.1".to_string(),
                    runtime_version: "0.2.0-rc.1".to_string(),
                },
                IncompatibleVersion {
                    name: "dsh-rewind-plugin".to_string(),
                    version: "0.14.0".to_string(),
                    runtime_version: "0.2.0-rc.1".to_string(),
                },
            ]
        );
    }

    #[test]
    fn incompatible_versions_keeps_scoped_package_name() {
        let out = "dsh: installation rejected: Plugin @wenbin_wb/dsh-bridge@1.2.3 is incompatible with dsh 0.2.0-rc.1: peerDependencies {}.\n";
        assert_eq!(
            incompatible_versions(out),
            vec![IncompatibleVersion {
                name: "@wenbin_wb/dsh-bridge".to_string(),
                version: "1.2.3".to_string(),
                runtime_version: "0.2.0-rc.1".to_string(),
            }]
        );
    }

    #[test]
    fn incompatible_versions_dedupes_repeated_refusals() {
        let repeated = format!("{INCOMPATIBLE_OUTPUT}{INCOMPATIBLE_OUTPUT}");
        assert_eq!(incompatible_versions(&repeated).len(), 1);
    }

    #[test]
    fn incompatible_versions_ignores_other_failures_and_malformed_lines() {
        assert!(incompatible_versions("ERR_PNPM_FETCH_404 registry error").is_empty());
        assert!(incompatible_versions("").is_empty());
        assert!(incompatible_versions(
            "dsh: plugin command failed; diagnostics: /tmp/pnpm.log\n"
        )
        .is_empty());
        assert!(incompatible_versions(
            "Plugin foo@1.0.0 is incompatible with dsh unknown-runtime: peerDependencies {}."
        )
        .is_empty());
        assert!(incompatible_versions(
            "Plugin foo is incompatible with dsh 0.2.0-rc.1: peerDependencies {}."
        )
        .is_empty());
    }

    #[test]
    fn pick_error_message_keeps_shim_unavailable_lines() {
        let out = "dsh: pnpm failed in profile directory C:\\Users\\小蔡\\.dsh\\profiles\\safe\n\
                   [pnpm] pnpm not found. Please run DeepSeek Harness Desktop to install it first.\n";
        let picked = pick_error_message(out, None);
        assert!(
            picked.contains("pnpm not found"),
            "shim diagnostic must survive: {picked}"
        );

        let cmd = "'pnpm' is not recognized as an internal or external command,\noperable program or batch file.\n";
        assert!(pick_error_message(cmd, None).contains("not recognized"));

        let label = "The system cannot find the batch label specified - no_pnpm\n";
        assert!(pick_error_message(label, None).contains("batch label"));
    }

    #[test]
    fn pick_error_message_still_drops_noise() {
        let out = "Progress: resolved 1, reused 0, downloaded 0\n\
                   ERR_PNPM_LINKING_FAILED: stale symlink\n\
                   Done in 2.8s\n";
        let picked = pick_error_message(out, None);
        assert!(picked.contains("ERR_PNPM_LINKING_FAILED"));
        assert!(!picked.contains("Done in"), "progress noise must be dropped: {picked}");
    }

    #[test]
    fn git_transport_hint_detects_host_key_failure() {
        let out = "git ls-remote \"git+ssh://git@github.com/foo.git\" HEAD\nHost key verification failed.\nfatal: Could not read from remote repository.\n";
        assert!(git_transport_hint(out).is_some());
    }

    #[test]
    fn git_transport_hint_detects_publickey_and_ssh() {
        assert!(git_transport_hint("git@github.com: Permission denied (publickey)").is_some());
        assert!(
            git_transport_hint("ssh: connect to host github.com port 22: Connection refused")
                .is_some()
        );
    }

    #[test]
    fn git_transport_hint_detects_enoent_missing_git() {
        // issue #369：Linux 无系统 git 时 pnpm spawn git 直接 ENOENT
        let out = "[ENOENT] Command failed with ENOENT: git ls-remote 'git+ssh://git@github.com/omdsh-dev/DSH-better-sidebar.git' HEAD\nspawn git ENOENT\n";
        assert!(git_transport_hint(out).is_some());
        assert!(git_transport_hint("spawn git ENOENT").is_some());
    }

    #[test]
    fn git_transport_hint_none_for_allowbuilds_output() {
        let out = "[ERR_PNPM_GIT_DEP_PREPARE_NOT_ALLOWED] ...\nallowBuilds:\n  node-pty: true\n";
        assert!(git_transport_hint(out).is_none());
    }

    const POLICY_VERIFICATION_FETCH_FAILURE: &str = "✗ Lockfile failed supply-chain policy check (4 entries in 2.7s) [ERR_PNPM_MINIMUM_RELEASE_AGE_VIOLATION]\n1 lockfile entries failed verification:\n[WARN] GET https://registry.npmjs.org/undici error (unknown). Will retry in 10 seconds. 2 retries left.\n";

    #[test]
    fn policy_verification_failure_detects_registry_fetch_error() {
        assert!(policy_verification_network_failure(
            POLICY_VERIFICATION_FETCH_FAILURE
        ));
    }

    #[test]
    fn policy_verification_failure_ignores_real_release_age_violation() {
        let real = "[ERR_PNPM_MINIMUM_RELEASE_AGE_VIOLATION] undici@7.29.1 was published recently (released 5 minutes ago; minimumReleaseAge is 1440)\n";
        assert!(!policy_verification_network_failure(real));
        assert!(!policy_verification_network_failure(
            "ERR_PNPM_FETCH_404 registry error"
        ));
        assert!(!policy_verification_network_failure(""));
    }

    const REAL_POLICY_VIOLATION: &str = "? Verifying lockfile against supply-chain policies (201 entries)...\n✗ Lockfile failed supply-chain policy check (201 entries in 1.5s)\n[ERR_PNPM_MINIMUM_RELEASE_AGE_VIOLATION] 1 lockfile entries failed verification:\n@wenbin_wb/dsh-bridge@2.11.2 was published at 2026-09-28T14:14:07.937Z, within the minimumReleaseAge cutoff (2026-09-27T15:50:11.385Z)\n\nProgress: resolved 48, reused 0, downloaded 0, added 0\n[WARN] GET https://registry.npmjs.org/axios error (UND_ERR_DESTROYED). Will retry in 10 seconds. 2 retries left.\n";

    #[test]
    fn policy_blocked_versions_reads_the_real_lockfile_violation() {
        let blocked = policy_blocked_versions(REAL_POLICY_VIOLATION);

        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].name, "@wenbin_wb/dsh-bridge");
        assert_eq!(blocked[0].version, "2.11.2");
    }

    #[test]
    fn policy_blocked_versions_dedupes_and_ignores_other_output() {
        let output = format!(
            "{REAL_POLICY_VIOLATION}{}\n",
            "lodash@4.17.21 was published at 2026-09-28T10:00:00.000Z, within the minimumReleaseAge cutoff (2026-09-27T15:50:11.385Z)"
        );
        let blocked = policy_blocked_versions(&output);

        assert_eq!(blocked.len(), 2);
        assert_eq!(blocked[1].name, "lodash");
        assert_eq!(blocked[1].version, "4.17.21");

        let resolution_form = "[ERR_PNPM_MINIMUM_RELEASE_AGE_VIOLATION] undici@7.29.1 was published recently (released 5 minutes ago; minimumReleaseAge is 1440)";
        assert!(policy_blocked_versions(resolution_form).is_empty());
        let no_cutoff = "undici@7.29.1 was published at 2026-09-28T10:00:00.000Z";
        assert!(policy_blocked_versions(no_cutoff).is_empty());
    }

    #[test]
    fn genuine_policy_violation_is_not_retried_as_a_network_failure() {
        assert!(!policy_verification_network_failure(REAL_POLICY_VIOLATION));
    }

    /// pnpm 的真实原始输出：档案由 pnpm 10 装好，当前 pnpm 11 解析出另一份 store。
    const UNEXPECTED_STORE_OUTPUT: &str = r#"
 ERR_PNPM_UNEXPECTED_STORE  Unexpected store location

The dependencies at "/Users/gao/.dsh/profiles/web/node_modules" are currently linked from the store at "/Users/gao/Library/pnpm/store/v10".

pnpm now wants to use the store at "/Users/gao/Library/pnpm/store/v11" to link dependencies.

If you want to use the new store location, reinstall your dependencies with "pnpm install".

You may change the global store location by running "pnpm config set store-dir <dir> --global".
(This error may happen if the node_modules was installed with a different major version of pnpm)
"#;

    #[test]
    fn store_mismatch_hint_names_both_store_paths() {
        let hint = store_mismatch_hint(UNEXPECTED_STORE_OUTPUT).expect("store hint");
        assert!(hint.contains("/Users/gao/Library/pnpm/store/v10"));
        assert!(hint.contains("/Users/gao/Library/pnpm/store/v11"));
        assert!(hint.contains("node_modules"));
    }

    #[test]
    fn store_mismatch_hint_covers_virtual_store_and_pathless_codes() {
        let virtual_store = r#"
 ERR_PNPM_UNEXPECTED_VIRTUAL_STORE  Unexpected virtual store location

The dependencies at "/p/node_modules" are currently symlinked from the virtual store directory at "/old/.pnpm".

pnpm now wants to use the virtual store at "/new/.pnpm" to link dependencies from the store.
"#;
        let hint = store_mismatch_hint(virtual_store).expect("virtual store hint");
        assert!(hint.contains("/old/.pnpm"));
        assert!(hint.contains("/new/.pnpm"));

        let pathless = r#"[ERR_PNPM_MODULES_BREAKING_CHANGE] The node_modules structure at "/p/node_modules" is not compatible with the current pnpm version. Run "pnpm install --force" to recreate node_modules."#;
        assert!(store_mismatch_hint(pathless).is_some());
    }

    #[test]
    fn store_mismatch_hint_none_for_other_failures() {
        assert!(store_mismatch_hint("ERR_PNPM_NO_MATCHING_VERSION: no version").is_none());
        assert!(store_mismatch_hint("").is_none());
        assert!(store_mismatch_hint("[ERR_PNPM_UNEXPECTED_STORE_EXTRA] other failure").is_none());
        assert!(store_mismatch_hint("[ERR_PNPM_UNEXPECTED_VIRTUAL_STORE_X] other").is_none());
        assert!(store_mismatch_hint("[ERR_PNPM_UNEXPECTED_STORE] Unexpected store location").is_some());
        assert!(store_mismatch_hint(" ERR_PNPM_UNEXPECTED_STORE  x").is_some());
    }

    #[test]
    fn pick_error_message_drops_store_paths_which_is_why_the_hint_exists() {
        let picked = pick_error_message(UNEXPECTED_STORE_OUTPUT, None);
        assert!(picked.contains("Unexpected store location"));
        assert!(!picked.contains("store/v10"));
        assert!(!picked.contains("store/v11"));
    }
}
