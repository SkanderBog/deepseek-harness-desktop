//! 字符串拼接遗留的「重复映射键」损坏文件（issue #49）。

use std::collections::HashMap;
use std::path::PathBuf;

use serde_yaml::{Mapping, Value};

use super::profile_dir;
use crate::service::profile::parse_workspace_document;
use crate::{AppHandle};

pub(super) fn parse_allowlist_keys(output: &str) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    let lines: Vec<&str> = output.lines().collect();

    for (idx, line) in lines.iter().enumerate() {
        if line.trim() == "allowBuilds:" {
            if let Some(next) = lines.get(idx + 1) {
                if let Some(key) = extract_allow_line_key(next) {
                    push_unique(&mut keys, key);
                }
            }
        }
    }

    for line in &lines {
        if let Some(sub) = line.split("Ignored build scripts:").nth(1) {
            for token in sub.split([',', ' ']) {
                let token = token.trim();
                if token.is_empty() {
                    continue;
                }
                let name = token
                    .rsplit_once('@')
                    .map_or(token, |(name, _)| name)
                    .trim();
                if !name.is_empty() {
                    push_unique(&mut keys, name.to_string());
                }
            }
        }
    }

    for (idx, line) in lines.iter().enumerate() {
        if line.trim() == "onlyBuiltDependencies:" {
            for next in lines.iter().skip(idx + 1) {
                let trimmed = next.trim_start();
                if let Some(rest) = trimmed.strip_prefix('-') {
                    let item = rest.trim().trim_matches(['"', '\'']);
                    if !item.is_empty() {
                        push_unique(&mut keys, item.to_string());
                    }
                } else if !next.starts_with(' ')
                    && !next.starts_with('\t')
                    && !trimmed.is_empty()
                    && !trimmed.starts_with('#')
                {
                    break; // 顶层键（无缩进且非列表项），已离开列表
                }
            }
        }
    }

    for line in &lines {
        if let Some(name) = extract_only_builds_git_name(line) {
            push_unique(&mut keys, name);
        }
    }

    keys
}

fn extract_allow_line_key(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.len() == line.len() {
        return None; // 无缩进，不是白名单条目
    }
    let suffix = trimmed.strip_suffix(": true")?;
    let key = suffix.trim_end();
    if key.is_empty() {
        return None;
    }
    Some(key.to_string())
}

fn push_unique(keys: &mut Vec<String>, key: String) {
    if !keys.iter().any(|k| k == &key) {
        keys.push(key);
    }
}

fn extract_only_builds_git_name(line: &str) -> Option<String> {
    if !line.contains("\"onlyBuiltDependencies\" allowlist") {
        return None;
    }
    let marker = "The git-hosted package \"";
    let start = line.find(marker)? + marker.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    let quoted = &rest[..end]; // "name@version"
    let (name, _version) = quoted.rsplit_once('@')?;
    (!name.is_empty()).then(|| name.to_string())
}

fn dep_path_to_name(key: &str) -> String {
    const GIT_RES_ID_MARKERS: &[&str] =
        &["@git+", "@https://", "@http://", "@git://", "@github.com/"];
    for marker in GIT_RES_ID_MARKERS {
        if let Some(pos) = key.find(marker) {
            return key[..pos].to_string();
        }
    }
    key.to_string()
}

fn profile_workspace_path(app_handle: &AppHandle) -> PathBuf {
    profile_dir(app_handle).join("pnpm-workspace.yaml")
}

/// TODO(v1): 移除对旧版损坏文件（issue #49）的自愈逻辑。v1 起只解析干净配置，
/// `'node-pty': true` 并存，见 issue #49）。此处解析失败时先做一次
pub(super) fn add_allow_build_keys(app_handle: &AppHandle, keys: &[String]) -> Result<(), String> {
    let path = profile_workspace_path(app_handle);
    let dir = path
        .parent()
        .ok_or("PREINSTALL_BAD_PROFILE_DIR: no profile dir")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("PREINSTALL_MKDIR: {e}"))?;

    let content = if path.exists() {
        std::fs::read_to_string(&path).map_err(|e| format!("PREINSTALL_READ_WORKSPACE: {e}"))?
    } else {
        "packages:\n  - .\n\nnodeLinker: hoisted\nautoInstallPeers: false\n".to_string()
    };

    let rendered = apply_allow_build_keys(&content, keys)?;
    if rendered == content {
        return Ok(()); // 无变化（所有键已就位），避免无意义写盘
    }

    log::info!(
        "pnpm-workspace.yaml rewritten with allowBuilds {keys:?} at {}",
        path.display()
    );
    std::fs::write(&path, rendered).map_err(|e| format!("PREINSTALL_WRITE_WORKSPACE: {e}"))
}

/// `'node-pty': true` 并存，见 issue #49）。此处先尝试严格解析；解析失败时
fn apply_allow_build_keys(content: &str, keys: &[String]) -> Result<String, String> {
    // 「多文档」（`---` 分隔，issue #526）同样让 serde_yaml 与 pnpm 都拒绝——交给
    let (mut doc, repaired): (Value, bool) = match parse_workspace_document(content) {
        Ok((value, normalized)) => (value, normalized),
        Err(first_err) => {
            let normalized = collapse_allow_builds_duplicates(content);
            if normalized == content {
                return Err(format!("PREINSTALL_WORKSPACE_INVALID_YAML: {first_err}"));
            }
            let value = parse_workspace_document(&normalized)
                .map(|(value, _)| value)
                .map_err(|e| format!("PREINSTALL_WORKSPACE_INVALID_YAML: {e}"))?;
            (value, true)
        }
    };

    if doc.is_null() {
        doc = Value::Mapping(Mapping::new());
    }

    let map = doc.as_mapping_mut().ok_or_else(|| {
        "PREINSTALL_WORKSPACE_NOT_MAP: pnpm-workspace.yaml must be a mapping".to_string()
    })?;

    let allow_key = Value::String("allowBuilds".to_string());
    if !map.contains_key(&allow_key) {
        map.insert(allow_key.clone(), Value::Mapping(Mapping::new()));
    }
    let allow_builds = map
        .get_mut(&allow_key)
        .and_then(Value::as_mapping_mut)
        .ok_or_else(|| {
            "PREINSTALL_WORKSPACE_ALLOWBUILDS_NOT_MAP: allowBuilds must be a mapping".to_string()
        })?;

    let mut dirty = false;
    for key in keys {
        let k = Value::String(key.clone());
        if allow_builds.get(&k) == Some(&Value::Bool(true)) {
            continue; // 已是 true，幂等跳过
        }
        allow_builds.insert(k, Value::Bool(true));
        dirty = true;
    }

    for (key, value) in allow_builds.iter_mut() {
        if value.as_bool().is_none()
            && key
                .as_str()
                .is_some_and(|key| keys.contains(&dep_path_to_name(key)))
        {
            *value = Value::Bool(true);
            dirty = true;
        }
    }

    let only_key = Value::String("onlyBuiltDependencies".to_string());
    let to_add: Vec<Value> = {
        let existing_only = map.get(&only_key).and_then(Value::as_sequence);
        keys.iter()
            .map(|k| dep_path_to_name(k))
            .filter(|name| {
                existing_only.is_none_or(|seq| !seq.contains(&Value::String(name.clone())))
            })
            .map(Value::String)
            .collect()
    };
    if !to_add.is_empty() {
        if !map.contains_key(&only_key) {
            map.insert(only_key.clone(), Value::Sequence(Vec::new()));
        }
        let only_builds = map
            .get_mut(&only_key)
            .and_then(Value::as_sequence_mut)
            .ok_or_else(|| {
                "PREINSTALL_WORKSPACE_ONLYBUILT_NOT_SEQ: onlyBuiltDependencies must be a sequence"
                    .to_string()
            })?;
        for v in to_add {
            only_builds.push(v);
            dirty = true;
        }
    }
    if !dirty && !repaired {
        return Ok(content.to_string());
    }

    serde_yaml::to_string(&doc).map_err(|e| format!("PREINSTALL_WORKSPACE_RENDER: {e}"))
}

/// 仅作为旧版字符串拼接遗留损坏（重复映射键，见 issue #49）的兜底归一化：
fn collapse_allow_builds_duplicates(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut in_allow = false;
    let mut key_indexes: HashMap<String, Vec<usize>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed == "allowBuilds:" {
            in_allow = true;
            continue;
        }
        if in_allow {
            let is_indent = line.starts_with(' ') || line.starts_with('\t');
            let is_comment = trimmed.starts_with('#');
            if !is_indent || is_comment {
                in_allow = false; // 遇到顶层键或注释即离开 allowBuilds
                continue;
            }
            if let Some(col) = trimmed.find(':') {
                let key = trimmed[..col].trim().trim_matches(['\'', '"']);
                if !key.is_empty() {
                    if !key_indexes.contains_key(key) {
                        order.push(key.to_string());
                    }
                    key_indexes.entry(key.to_string()).or_default().push(idx);
                }
            }
        }
    }

    let mut keep: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for key in &order {
        if let Some(idxs) = key_indexes.get(key) {
            if let Some(&last) = idxs.last() {
                keep.insert(last);
            }
        }
    }
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    for (idx, line) in lines.iter().enumerate() {
        if key_indexes.values().any(|v| v.contains(&idx)) && !keep.contains(&idx) {
            continue; // 是被去重掉的重复键行
        }
        out.push(line);
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_git_dep_path_key() {
        let out = "\
[ERR_PNPM_GIT_DEP_PREPARE_NOT_ALLOWED] Failed to prepare git-hosted package fetched from \"...\"
The git-hosted package \"dsh-better-sidebar@0.14.0\" needs to execute build scripts but is not in the \"allowBuilds\" allowlist.
...
allowBuilds:
  dsh-better-sidebar@git+ssh://git@github.com/omdsh-dev/DSH-better-sidebar.git#6c89: true
";
        let keys = parse_allowlist_keys(out);
        assert!(keys.contains(
            &"dsh-better-sidebar@git+ssh://git@github.com/omdsh-dev/DSH-better-sidebar.git#6c89"
                .to_string()
        ));
        assert!(!keys.contains(&"dsh-better-sidebar".to_string()));
    }

    #[test]
    fn parse_ignored_builds_name() {
        let out = "[ERR_PNPM_IGNORED_BUILDS] Ignored build scripts: node-pty@1.1.0\n";
        let keys = parse_allowlist_keys(out);
        assert_eq!(keys, vec!["node-pty".to_string()]);
    }

    #[test]
    fn parse_ignored_builds_scoped_name() {
        let out = "[ERR_PNPM_IGNORED_BUILDS] Ignored build scripts: @deepseek-ai/dsh-base@0.0.4, node-pty@1.1.0\n";
        let keys = parse_allowlist_keys(out);
        assert_eq!(
            keys,
            vec!["@deepseek-ai/dsh-base".to_string(), "node-pty".to_string()]
        );
    }

    #[test]
    fn parse_empty_when_irrelevant() {
        let out = "everything looks fine output\nno allowlist here\n";
        assert!(parse_allowlist_keys(out).is_empty());
    }

    #[test]
    fn parse_pnpm10_only_built_dependencies_list() {
        let out = "\
[ERR_PNPM_GIT_DEP_PREPARE_NOT_ALLOWED] Failed to prepare git-hosted package fetched from \"https://codeload.github.com/omdsh-dev/DSH-better-sidebar/tar.gz/f9153dfc1ce47cf43445c1b351ee3ae47b4ad9f1\"
The git-hosted package \"dsh-better-sidebar@0.16.1\" needs to execute build scripts but is not in the \"onlyBuiltDependencies\" allowlist.
...
This error happened while installing a direct dependency of C:\\Users\\hairy\\.dsh.dev\\profiles\\web
Add the package to \"onlyBuiltDependencies\" in your project's pnpm-workspace.yaml to allow it to run scripts. For example:
onlyBuiltDependencies:
- \"dsh-better-sidebar\"
";
        let keys = parse_allowlist_keys(out);
        assert_eq!(keys, vec!["dsh-better-sidebar".to_string()]);
    }

    #[test]
    fn parse_pnpm10_unindented_list_items() {
        let out = "onlyBuiltDependencies:\n- \"dsh-better-sidebar\"\n";
        let keys = parse_allowlist_keys(out);
        assert_eq!(keys, vec!["dsh-better-sidebar".to_string()]);
    }

    #[test]
    fn parse_pnpm10_scoped_package_from_message() {
        let out = "The git-hosted package \"@deepseek-ai/dsh-base@0.0.4\" needs to execute build scripts but is not in the \"onlyBuiltDependencies\" allowlist.\n";
        let keys = parse_allowlist_keys(out);
        assert_eq!(keys, vec!["@deepseek-ai/dsh-base".to_string()]);
    }

    #[test]
    fn extract_only_builds_git_name_strips_version_and_handles_scoped() {
        assert_eq!(
            extract_only_builds_git_name(
                "The git-hosted package \"node-pty@1.1.0\" needs to execute build scripts but is not in the \"onlyBuiltDependencies\" allowlist."
            ),
            Some("node-pty".to_string())
        );
        assert_eq!(
            extract_only_builds_git_name(
                "The git-hosted package \"@deepseek-ai/dsh-base@0.0.4\" needs to execute build scripts but is not in the \"onlyBuiltDependencies\" allowlist."
            ),
            Some("@deepseek-ai/dsh-base".to_string())
        );
        assert_eq!(
            extract_only_builds_git_name(
                "The git-hosted package \"dsh-better-sidebar@0.14.0\" needs to execute build scripts but is not in the \"allowBuilds\" allowlist."
            ),
            None
        );
        assert_eq!(
            extract_only_builds_git_name("allowBuilds:\n  x: true"),
            None
        );
    }

    #[test]
    fn dep_path_to_name_strips_git_resolution_keeps_plain_names() {
        assert_eq!(
            dep_path_to_name(
                "dsh-better-sidebar@git+ssh://git@github.com/omdsh-dev/DSH-better-sidebar.git#6c89"
            ),
            "dsh-better-sidebar"
        );
        assert_eq!(
            dep_path_to_name(
                "@deepseek-ai/dsh-base@git+https://github.com/deepseek-ai/dsh-base.git#abc123"
            ),
            "@deepseek-ai/dsh-base"
        );
        assert_eq!(dep_path_to_name("node-pty"), "node-pty");
        assert_eq!(dep_path_to_name("node-pty@1.1.0"), "node-pty@1.1.0");
        assert_eq!(dep_path_to_name("@scope/pkg"), "@scope/pkg");
    }

    #[test]
    fn allow_line_key_requires_indent() {
        let key = extract_allow_line_key("  node-pty: true");
        assert_eq!(key.as_deref(), Some("node-pty"));

        assert_eq!(extract_allow_line_key("packages:"), None);
        assert_eq!(extract_allow_line_key("allowBuilds:"), None);
    }

    // ---- 归并写回 pnpm-workspace.yaml（issue #49 回归）----

    fn allow_builds_map(yaml: &str) -> serde_yaml::Mapping {
        let doc: serde_yaml::Value = serde_yaml::from_str(yaml).expect("output must be valid YAML");
        doc.get("allowBuilds")
            .and_then(serde_yaml::Value::as_mapping)
            .expect("allowBuilds must be a mapping")
            .clone()
    }

    #[test]
    fn apply_adds_new_key_when_absent() {
        let base = "packages:\n  - .\n\nnodeLinker: hoisted\nautoInstallPeers: false\n";
        let out = apply_allow_build_keys(base, &["node-pty".to_string()]).unwrap();
        let map = allow_builds_map(&out);
        assert_eq!(map.get("node-pty"), Some(&serde_yaml::Value::Bool(true)));
        let doc: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert!(doc.get("packages").is_some());
        assert!(doc.get("nodeLinker").is_some());
    }

    #[test]
    fn apply_is_idempotent_and_does_not_duplicate() {
        let base = "packages:\n  - .\nnodeLinker: hoisted\nautoInstallPeers: false\nallowBuilds:\n  node-pty: true\nonlyBuiltDependencies:\n  - node-pty\n";
        let out = apply_allow_build_keys(base, &["node-pty".to_string()]).unwrap();
        assert_eq!(out, base);
    }

    #[test]
    fn apply_settles_pnpm_pending_build_entry() {
        let base = "\
allowBuilds:
  '@modusensus/dsh-mneme': true
  '@modusensus/dsh-mneme@https://codeload.github.com/modusensus/dsh-mneme/tar.gz/add7c451...': set this to true or false
  '@modusensus/dsh-mneme@https://codeload.github.com/modusensus/dsh-mneme/tar.gz/denied': false
  '@modusensus/dsh-mneme-extra@https://example.com/other.tgz': set this to true or false
onlyBuiltDependencies:
  - '@modusensus/dsh-mneme'
";
        let keys = parse_allowlist_keys(
            "[ERR_PNPM_IGNORED_BUILDS] Ignored build scripts: @modusensus/dsh-mneme@https://codeload.github.com/modusensus/dsh-mneme/tar.gz/add7c451...\n",
        );
        assert_eq!(keys, vec!["@modusensus/dsh-mneme".to_string()]);
        let out = apply_allow_build_keys(base, &keys).unwrap();
        let map = allow_builds_map(&out);
        assert_eq!(
            map.get("@modusensus/dsh-mneme@https://codeload.github.com/modusensus/dsh-mneme/tar.gz/add7c451..."),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            map.get("@modusensus/dsh-mneme@https://codeload.github.com/modusensus/dsh-mneme/tar.gz/denied"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            map.get("@modusensus/dsh-mneme-extra@https://example.com/other.tgz"),
            Some(&Value::String("set this to true or false".to_string()))
        );
        let doc: Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(
            doc["onlyBuiltDependencies"],
            Value::Sequence(vec![Value::String("@modusensus/dsh-mneme".to_string())])
        );
        assert_eq!(apply_allow_build_keys(&out, &keys).unwrap(), out);
    }

    #[test]
    fn apply_writes_both_allowbuilds_and_only_built_dependencies() {
        let base = "packages:\n  - .\n\nnodeLinker: hoisted\nautoInstallPeers: false\n";
        let out = apply_allow_build_keys(base, &["dsh-better-sidebar".to_string()]).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(
            doc["allowBuilds"]["dsh-better-sidebar"],
            serde_yaml::Value::Bool(true)
        );
        let only = doc
            .get("onlyBuiltDependencies")
            .and_then(serde_yaml::Value::as_sequence)
            .expect("onlyBuiltDependencies must be a sequence");
        assert_eq!(
            only,
            &vec![serde_yaml::Value::String("dsh-better-sidebar".to_string())]
        );
        assert!(doc.get("packages").is_some());
        assert!(doc.get("nodeLinker").is_some());
    }

    #[test]
    fn apply_git_dep_path_writes_name_selector_for_pnpm10() {
        let base = "packages:\n  - .\n\nnodeLinker: hoisted\nautoInstallPeers: false\n";
        let dep =
            "dsh-better-sidebar@git+ssh://git@github.com/omdsh-dev/DSH-better-sidebar.git#6c89"
                .to_string();
        let out = apply_allow_build_keys(base, std::slice::from_ref(&dep)).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(
            doc["allowBuilds"][&serde_yaml::Value::String(dep)],
            serde_yaml::Value::Bool(true)
        );
        let only = doc
            .get("onlyBuiltDependencies")
            .and_then(serde_yaml::Value::as_sequence)
            .expect("onlyBuiltDependencies must be a sequence");
        assert_eq!(
            only,
            &vec![serde_yaml::Value::String("dsh-better-sidebar".to_string())]
        );
    }

    #[test]
    fn apply_only_built_dependencies_preserves_existing_entries() {
        let base = "packages:\n  - .\nallowBuilds:\n  esbuild: true\nonlyBuiltDependencies:\n  - esbuild\n";
        let out = apply_allow_build_keys(base, &["dsh-better-sidebar".to_string()]).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        let only = doc
            .get("onlyBuiltDependencies")
            .and_then(serde_yaml::Value::as_sequence)
            .expect("onlyBuiltDependencies must be a sequence");
        let names: Vec<&str> = only.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(names, vec!["esbuild", "dsh-better-sidebar"]);
    }

    #[test]
    fn apply_quotes_git_dep_path_keys() {
        let dep =
            "dsh-better-sidebar@git+ssh://git@github.com/omdsh-dev/DSH-better-sidebar.git#6c89"
                .to_string();
        let out = apply_allow_build_keys("", std::slice::from_ref(&dep)).unwrap();
        let map = allow_builds_map(&out);
        assert_eq!(
            map.get(serde_yaml::Value::String(dep)),
            Some(&serde_yaml::Value::Bool(true))
        );
        let doc: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(
            doc["allowBuilds"][&serde_yaml::Value::String(
                "dsh-better-sidebar@git+ssh://git@github.com/omdsh-dev/DSH-better-sidebar.git#6c89"
                    .to_string()
            )],
            serde_yaml::Value::Bool(true)
        );
    }

    #[test]
    fn apply_overwrites_placeholder_value_without_duplicate() {
        let corrupted =
            "allowBuilds:\n  'dsh-better-sidebar@https://code...': true\n  node-pty: set this to true or false\n  'node-pty': true\n";
        let out = apply_allow_build_keys(corrupted, &["node-pty".to_string()]).unwrap();
        let map = allow_builds_map(&out);
        assert_eq!(map.get("node-pty"), Some(&serde_yaml::Value::Bool(true)));
        let node_pty_keys = out
            .lines()
            .filter(|l| {
                l.trim_start().starts_with("node-pty") || l.trim_start().starts_with("'node-pty'")
            })
            .count();
        assert_eq!(node_pty_keys, 1);
    }

    #[test]
    fn collapse_dedupes_allow_builds_keys() {
        let corrupted =
            "packages:\n  - .\nallowBuilds:\n  node-pty: set this to true or false\n  'node-pty': true\n  keep: true\n";
        let normalized = collapse_allow_builds_duplicates(corrupted);
        let node_pty = normalized
            .lines()
            .filter(|l| {
                l.trim_start().starts_with("node-pty") || l.trim_start().starts_with("'node-pty'")
            })
            .count();
        assert_eq!(node_pty, 1);
        assert!(normalized.contains("keep"));
        let out = apply_allow_build_keys(&normalized, &["node-pty".to_string()]).unwrap();
        assert_eq!(
            allow_builds_map(&out).get("node-pty"),
            Some(&serde_yaml::Value::Bool(true))
        );
    }

    /// issue #526：`---` 分隔的多文档 `pnpm-workspace.yaml` 在写回时归一化成单文档，
    #[test]
    fn apply_normalizes_multi_document_workspace() {
        let multi = "packages:\n  - .\nnodeLinker: hoisted\n---\nautoInstallPeers: false\n";
        let out = apply_allow_build_keys(multi, &["node-pty".to_string()]).unwrap();

        assert_eq!(out.matches("\n---").count(), 0, "{out}");
        let doc: serde_yaml::Value = serde_yaml::from_str(&out).expect("output must be single-doc");
        assert!(doc.get("packages").is_some());
        assert!(doc.get("nodeLinker").is_some());
        assert_eq!(
            doc.get("autoInstallPeers")
                .and_then(serde_yaml::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            allow_builds_map(&out).get("node-pty"),
            Some(&serde_yaml::Value::Bool(true))
        );
    }

    #[test]
    fn multi_document_workspace_is_rewritten_even_without_new_keys() {
        let multi = "packages:\n  - .\n---\nnodeLinker: hoisted\n";
        let out = apply_allow_build_keys(multi, &[]).unwrap();
        assert_eq!(out.matches("\n---").count(), 0, "{out}");
        assert!(serde_yaml::from_str::<serde_yaml::Value>(&out).is_ok());
    }
}
