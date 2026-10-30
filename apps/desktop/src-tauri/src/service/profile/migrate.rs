use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use semver::Version;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use crate::AppHandle;
use crate::config;
use crate::service::fs_guard;
use crate::service::plugin::compat::{self, PluginInspect};
use crate::service::plugin::{
    ProfilePackageJson, inspect_specs, load_presets, packument, parse_patch_layer,
    render_patch_layer, watch,
};

const POLICY_KEY: &str = "minimumReleaseAgeExclude";

const CREDENTIALS_FILE: &str = ".credentials.yaml";
const PATCH_FILE: &str = "cordis.patch.yml";
const POLICY_FILE: &str = "pnpm-workspace.yaml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MigrationVerdict {
    Compatible,
    Upgrade,
    Downgrade,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationEntry {
    pub id: String,
    pub version: String,
    pub spec: String,
    pub verdict: MigrationVerdict,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MigrationDataKind {
    Patch,
    Policy,
    Credentials,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationDataItem {
    pub kind: MigrationDataKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    pub covered: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationAnalysis {
    pub plugins: Vec<MigrationEntry>,
    pub data: Vec<MigrationDataItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationFailure {
    pub kind: MigrationDataKind,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateReport {
    pub applied: Vec<MigrationDataKind>,
    pub skipped: Vec<MigrationDataKind>,
    pub failures: Vec<MigrationFailure>,
}

fn is_registry_range(raw: &str) -> bool {
    matches!(
        raw.trim().as_bytes().first(),
        Some(b'0'..=b'9' | b'^' | b'~' | b'>' | b'<' | b'=' | b'v' | b'*')
    )
}

pub(crate) fn pick_nearest(packument: &serde_json::Value, runtime: &str) -> Option<String> {
    let mut best: Option<Version> = None;
    for (raw, manifest) in packument.get("versions")?.as_object()? {
        if compat::evaluate(&compat::peers_from_manifest(manifest), runtime) != Some(true) {
            continue;
        }
        let Ok(version) = Version::parse(raw) else {
            continue;
        };
        if best
            .as_ref()
            .map(|current| version > *current)
            .unwrap_or(true)
        {
            best = Some(version);
        }
    }
    best.map(|version| version.to_string())
}

fn compare_versions(installed: &str, nearest: &str) -> (MigrationVerdict, Option<String>) {
    let (Ok(current), Ok(target)) = (Version::parse(installed), Version::parse(nearest)) else {
        return (MigrationVerdict::Unknown, None);
    };
    if target > current {
        (MigrationVerdict::Upgrade, Some(nearest.to_string()))
    } else if target < current {
        (MigrationVerdict::Downgrade, Some(nearest.to_string()))
    } else {
        (MigrationVerdict::Unknown, None)
    }
}

fn candidates(app_handle: &AppHandle, dir: &Path) -> Vec<(String, String)> {
    let content = match fs::read_to_string(dir.join("package.json")) {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };
    let manifest: ProfilePackageJson = match serde_json::from_str(&content) {
        Ok(manifest) => manifest,
        Err(_) => return Vec::new(),
    };
    let installed: BTreeMap<String, String> = watch::parse_plugins(dir, &load_presets(app_handle))
        .into_iter()
        .filter(|plugin| !plugin.version.is_empty())
        .map(|plugin| (plugin.id, plugin.version))
        .collect();
    let mut entries: Vec<(String, String)> = manifest
        .dependencies
        .into_iter()
        .filter(|(id, raw)| is_registry_range(raw) && installed.contains_key(id))
        .map(|(id, _)| {
            let version = installed[&id].clone();
            (id, version)
        })
        .collect();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

async fn nearest(app_handle: &AppHandle, id: &str, runtime: Option<&str>) -> Option<String> {
    let packument = packument(app_handle, id).await?;
    pick_nearest(&packument, runtime?)
}

pub async fn analyze(app_handle: &AppHandle, source_id: &str) -> Result<MigrationAnalysis, String> {
    let root = config::get_dsh_data_path(app_handle).join("profiles");
    let dir = fs_guard::join_safe(&root, source_id)?;
    if !dir.is_dir() {
        return Err("PROFILE_MIGRATION_SOURCE_MISSING: source profile does not exist".to_string());
    }
    let entries = candidates(app_handle, &dir);
    let target = fs_guard::join_safe(&root, &super::active_profile(app_handle))?;
    let specs: Vec<String> = entries
        .iter()
        .map(|(id, version)| format!("{id}@{version}"))
        .collect();
    let inspected: Vec<PluginInspect> = inspect_specs(app_handle, &specs, None)
        .await
        .unwrap_or_default();
    let runtime = crate::service::core::active_version(app_handle);
    let mut plugins = Vec::with_capacity(entries.len());
    for (index, (id, version)) in entries.into_iter().enumerate() {
        let (verdict, target_version) = match inspected.get(index).and_then(|item| item.compatible)
        {
            Some(true) => (MigrationVerdict::Compatible, None),
            Some(false) => match nearest(app_handle, &id, runtime.as_deref()).await {
                Some(nearest) => compare_versions(&version, &nearest),
                None => (MigrationVerdict::Unknown, None),
            },
            None => (MigrationVerdict::Unknown, None),
        };
        let spec = match &target_version {
            Some(target) => format!("{id}@{target}"),
            None => format!("{id}@{version}"),
        };
        plugins.push(MigrationEntry {
            id,
            version,
            spec,
            verdict,
            target_version,
        });
    }
    Ok(MigrationAnalysis {
        plugins,
        data: data_items(&dir, &target),
    })
}

pub fn apply(
    app_handle: &AppHandle,
    source_id: &str,
    kinds: &[MigrationDataKind],
) -> Result<MigrateReport, String> {
    let root = config::get_dsh_data_path(app_handle).join("profiles");
    let source = fs_guard::join_safe(&root, source_id)?;
    if !source.is_dir() {
        return Err("PROFILE_MIGRATION_SOURCE_MISSING: source profile does not exist".to_string());
    }
    let active = super::active_profile(app_handle);
    if active == source_id {
        return Err("PROFILE_MIGRATION_SAME_PROFILE: source is the active profile".to_string());
    }
    let target = fs_guard::join_safe(&root, &active)?;
    let mut report = MigrateReport {
        applied: Vec::new(),
        skipped: Vec::new(),
        failures: Vec::new(),
    };
    for kind in kinds {
        match merge(*kind, &source, &target) {
            Ok(true) => report.applied.push(*kind),
            Ok(false) => report.skipped.push(*kind),
            Err(message) => report.failures.push(MigrationFailure {
                kind: *kind,
                message,
            }),
        }
    }
    Ok(report)
}

fn data_items(source: &Path, target: &Path) -> Vec<MigrationDataItem> {
    let mut items = Vec::new();
    let patch = patch_layer_entries(&source.join(PATCH_FILE)).unwrap_or_default();
    if !patch.is_empty() {
        items.push(MigrationDataItem {
            kind: MigrationDataKind::Patch,
            count: Some(patch.len()),
            covered: patch_covered(&patch, target),
        });
    }
    let policy = release_age_excludes(source);
    if !policy.is_empty() {
        let existing = release_age_excludes(target);
        items.push(MigrationDataItem {
            kind: MigrationDataKind::Policy,
            count: Some(policy.len()),
            covered: policy.iter().all(|entry| existing.contains(entry)),
        });
    }
    if source.join(CREDENTIALS_FILE).is_file() {
        items.push(MigrationDataItem {
            kind: MigrationDataKind::Credentials,
            count: None,
            covered: target.join(CREDENTIALS_FILE).exists(),
        });
    }
    items
}

fn patch_covered(source: &[Value], target: &Path) -> bool {
    let existing = patch_layer_entries(&target.join(PATCH_FILE)).unwrap_or_default();
    let ids: HashSet<&str> = existing.iter().filter_map(patch_entry_id).collect();
    source.iter().all(|entry| match patch_entry_id(entry) {
        Some(id) => ids.contains(id),
        None => false,
    })
}

fn patch_layer_entries(path: &Path) -> Option<Vec<Value>> {
    let content = fs::read_to_string(path).ok()?;
    let document = parse_patch_layer(&content).ok()?;
    Some(document.as_sequence()?.clone())
}

fn patch_entry_id(entry: &Value) -> Option<&str> {
    entry.get("id").and_then(Value::as_str)
}

fn release_age_excludes(dir: &Path) -> Vec<String> {
    let Ok(content) = fs::read_to_string(dir.join(POLICY_FILE)) else {
        return Vec::new();
    };
    let Ok((document, _)) = super::parse_workspace_document(&content) else {
        return Vec::new();
    };
    document
        .as_mapping()
        .and_then(|mapping| mapping.get(Value::String(POLICY_KEY.to_string())))
        .and_then(Value::as_sequence)
        .map(|sequence| {
            sequence
                .iter()
                .filter_map(|item| item.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn merge(kind: MigrationDataKind, source: &Path, target: &Path) -> Result<bool, String> {
    match kind {
        MigrationDataKind::Patch => merge_patch(source, target),
        MigrationDataKind::Policy => merge_policy(source, target),
        MigrationDataKind::Credentials => merge_credentials(source, target),
    }
}

fn merge_patch(source: &Path, target: &Path) -> Result<bool, String> {
    let Some(incoming) = patch_layer_entries(&source.join(PATCH_FILE)) else {
        return Ok(false);
    };
    let mut entries = patch_layer_entries(&target.join(PATCH_FILE)).unwrap_or_default();
    let mut existing: HashSet<String> = entries
        .iter()
        .filter_map(patch_entry_id)
        .map(String::from)
        .collect();
    let mut added = false;
    for entry in incoming {
        if let Some(id) = patch_entry_id(&entry) {
            if !existing.insert(id.to_string()) {
                continue;
            }
        }
        entries.push(entry);
        added = true;
    }
    if !added {
        return Ok(false);
    }
    let rendered = render_patch_layer(&Value::Sequence(entries))
        .map_err(|e| format!("PROFILE_MIGRATION_PATCH_RENDER: {e}"))?;
    fs::write(target.join(PATCH_FILE), rendered)
        .map_err(|e| format!("PROFILE_MIGRATION_PATCH_WRITE: {e}"))?;
    Ok(true)
}

fn merge_policy(source: &Path, target: &Path) -> Result<bool, String> {
    let existing = release_age_excludes(target);
    let missing: Vec<String> = release_age_excludes(source)
        .into_iter()
        .filter(|entry| !existing.contains(entry))
        .collect();
    if missing.is_empty() {
        return Ok(false);
    }
    super::release_age_excludes_at(target, &missing)?;
    Ok(true)
}

fn merge_credentials(source: &Path, target: &Path) -> Result<bool, String> {
    let from = source.join(CREDENTIALS_FILE);
    if !from.is_file() {
        return Ok(false);
    }
    let to = target.join(CREDENTIALS_FILE);
    if to.exists() {
        return Ok(false);
    }
    fs::create_dir_all(target).map_err(|e| format!("PROFILE_MIGRATION_CREDENTIALS_DIR: {e}"))?;
    fs::copy(&from, &to).map_err(|e| format!("PROFILE_MIGRATION_CREDENTIALS_COPY: {e}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn workspace(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dsh-migrate-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn registry_ranges_exclude_local_and_git_sources() {
        for raw in ["1.2.3", "^0.24.1", "~1.0.0", ">=1 <2", "v1.2.3", "*"] {
            assert!(is_registry_range(raw), "{raw} 应视为 registry 版本");
        }
        for raw in [
            "link:D:/software/plugin",
            "file:../plugin",
            "workspace:^1.0.0",
            "catalog:^1.0.0",
            "portal:./plugin",
            "github:owner/repo",
            "git+https://example.com/a.git",
            "npm:alias@1.0.0",
        ] {
            assert!(!is_registry_range(raw), "{raw} 不应进入迁移列表");
        }
    }

    #[test]
    fn pick_nearest_returns_highest_compatible_version() {
        let packument = json!({
            "versions": {
                "0.24.1": { "peerDependencies": { "@deepseek-ai/dsh-llm": "^0.2.0" } },
                "0.23.0": { "peerDependencies": { "@deepseek-ai/dsh-llm": "^0.1.5" } },
                "0.22.0": { "peerDependencies": { "@deepseek-ai/dsh-llm": "^0.1.5" } },
                "0.25.0": { "peerDependencies": { "@deepseek-ai/dsh-llm": "^0.3.0" } }
            }
        });

        assert_eq!(
            pick_nearest(&packument, "0.1.7").as_deref(),
            Some("0.23.0"),
            "最高兼容版本必须是 0.23.0，而不是 latest 或最旧"
        );
    }

    #[test]
    fn pick_nearest_ignores_versions_without_dsh_peers() {
        let packument = json!({
            "versions": {
                "1.0.0": { "dependencies": { "zod": "^4.0.0" } },
                "0.9.0": { "peerDependencies": { "@deepseek-ai/dsh": "^0.1.0" } }
            }
        });

        assert_eq!(pick_nearest(&packument, "0.1.7").as_deref(), Some("0.9.0"));
    }

    #[test]
    fn pick_nearest_returns_none_when_nothing_is_proven_compatible() {
        let packument = json!({
            "versions": { "2.0.0": { "peerDependencies": { "@deepseek-ai/dsh": "^0.3.0" } } }
        });

        assert_eq!(pick_nearest(&packument, "0.1.7"), None);
        assert_eq!(pick_nearest(&json!({}), "0.1.7"), None);
    }

    #[test]
    fn compare_versions_maps_incompatible_install_to_upgrade_or_downgrade() {
        assert_eq!(
            compare_versions("0.22.0", "0.24.0"),
            (MigrationVerdict::Upgrade, Some("0.24.0".to_string()))
        );
        assert_eq!(
            compare_versions("0.24.0", "0.22.0"),
            (MigrationVerdict::Downgrade, Some("0.22.0".to_string()))
        );
        assert_eq!(
            compare_versions("0.24.0", "0.24.0"),
            (MigrationVerdict::Unknown, None)
        );
        assert_eq!(
            compare_versions("nightly", "0.24.0"),
            (MigrationVerdict::Unknown, None)
        );
    }

    #[test]
    fn merge_patch_appends_only_missing_ids() {
        let source = workspace("patch-source");
        let target = workspace("patch-target");
        fs::write(
            source.join(PATCH_FILE),
            "- id: shared\n  name: '@scope/shared'\n- id: new\n  name: '@scope/new'\n",
        )
        .unwrap();
        fs::write(
            target.join(PATCH_FILE),
            "- id: shared\n  name: '@scope/local'\n",
        )
        .unwrap();

        assert!(merge_patch(&source, &target).unwrap());

        let merged = patch_layer_entries(&target.join(PATCH_FILE)).unwrap();
        assert_eq!(merged.len(), 2, "{merged:?}");
        assert_eq!(
            merged[0].get("name").and_then(Value::as_str),
            Some("@scope/local"),
            "目标已有的 id 必须保留目标条目"
        );
        assert_eq!(merged[1].get("id").and_then(Value::as_str), Some("new"));
        assert!(
            !merge_patch(&source, &target).unwrap(),
            "无新增时必须报告未改动，避免无意义的写盘"
        );

        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn merge_credentials_never_overwrites_existing_target() {
        let source = workspace("credentials-source");
        let target = workspace("credentials-target");
        fs::write(source.join(CREDENTIALS_FILE), "token: source\n").unwrap();

        assert!(merge_credentials(&source, &target).unwrap());
        assert_eq!(
            fs::read_to_string(target.join(CREDENTIALS_FILE)).unwrap(),
            "token: source\n"
        );

        assert!(
            !merge_credentials(&source, &target).unwrap(),
            "目标已有凭据时必须跳过"
        );
        fs::write(source.join(CREDENTIALS_FILE), "token: changed\n").unwrap();
        assert!(!merge_credentials(&source, &target).unwrap());
        assert_eq!(
            fs::read_to_string(target.join(CREDENTIALS_FILE)).unwrap(),
            "token: source\n",
            "目标凭据不可被源覆盖"
        );

        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn release_age_excludes_reads_only_the_policy_key() {
        let dir = workspace("policy");
        fs::write(
            dir.join(POLICY_FILE),
            "packages:\n  - .\nminimumReleaseAgeExclude:\n  - zod@4.4.3\n  - '@scope/pkg@1.0.0'\n",
        )
        .unwrap();

        assert_eq!(
            release_age_excludes(&dir),
            vec!["zod@4.4.3".to_string(), "@scope/pkg@1.0.0".to_string()]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_policy_writes_into_the_captured_target_dir() {
        let source = workspace("policy-source");
        let target = workspace("policy-target");
        fs::write(
            source.join(POLICY_FILE),
            "minimumReleaseAgeExclude:\n  - zod@4.4.3\n",
        )
        .unwrap();
        fs::write(target.join(POLICY_FILE), "packages:\n  - .\n").unwrap();

        assert!(merge_policy(&source, &target).unwrap());
        assert_eq!(release_age_excludes(&target), vec!["zod@4.4.3".to_string()]);
        assert!(!merge_policy(&source, &target).unwrap());

        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn data_items_lists_only_what_the_source_profile_has() {
        let dir = workspace("items");
        let target = workspace("items-target");
        assert!(
            data_items(&dir, &target).is_empty(),
            "空档案不应列出任何数据项"
        );

        fs::write(dir.join(PATCH_FILE), "- id: only-patch\n").unwrap();
        fs::write(dir.join(CREDENTIALS_FILE), "token: x\n").unwrap();

        let items = data_items(&dir, &target);
        assert_eq!(
            items.iter().map(|item| item.kind).collect::<Vec<_>>(),
            vec![MigrationDataKind::Patch, MigrationDataKind::Credentials]
        );
        assert_eq!(items[0].count, Some(1));
        assert_eq!(items[0].covered, false, "目标档案没有这些条目，必须可迁移");
        assert_eq!(items[1].count, None);
        assert_eq!(items[1].covered, false);

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn merge_patch_keeps_js_expressions() {
        let source = workspace("js-source");
        let target = workspace("js-target");
        fs::write(
            source.join(PATCH_FILE),
            "- id: new\n  config:\n    token: !!js process.env.TOKEN\n",
        )
        .unwrap();
        fs::write(
            target.join(PATCH_FILE),
            "- id: local\n  name: !!js process.env.LOCAL_PLUGIN\n",
        )
        .unwrap();

        assert!(merge_patch(&source, &target).unwrap());

        assert_eq!(
            fs::read_to_string(target.join(PATCH_FILE)).unwrap(),
            "- id: local\n  name: !!js process.env.LOCAL_PLUGIN\n- id: new\n  config:\n    token: !!js process.env.TOKEN\n"
        );
        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn data_items_marks_fully_contained_entries_as_covered() {
        let source = workspace("covered-source");
        let target = workspace("covered-target");
        fs::write(source.join(PATCH_FILE), "- id: shared\n- id: extra\n").unwrap();
        fs::write(target.join(PATCH_FILE), "- id: shared\n").unwrap();
        fs::write(
            source.join(POLICY_FILE),
            "minimumReleaseAgeExclude:\n  - zod@4.4.3\n  - '@scope/pkg@1.0.0'\n",
        )
        .unwrap();
        fs::write(
            target.join(POLICY_FILE),
            "minimumReleaseAgeExclude:\n  - zod@4.4.3\n  - '@scope/pkg@1.0.0'\n",
        )
        .unwrap();
        fs::write(source.join(CREDENTIALS_FILE), "token: x\n").unwrap();
        fs::write(target.join(CREDENTIALS_FILE), "token: y\n").unwrap();

        let items = data_items(&source, &target);
        let covered: Vec<(MigrationDataKind, bool)> =
            items.iter().map(|item| (item.kind, item.covered)).collect();
        assert_eq!(
            covered,
            vec![
                (MigrationDataKind::Patch, false),
                (MigrationDataKind::Policy, true),
                (MigrationDataKind::Credentials, true),
            ],
            "只有目标已完整包含的类目才算 covered"
        );

        fs::write(target.join(PATCH_FILE), "- id: shared\n- id: extra\n").unwrap();
        let items = data_items(&source, &target);
        assert!(
            items.iter().all(|item| item.covered),
            "目标补齐后全部类目都应视为已包含：{items:?}"
        );

        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&target);
    }
}
