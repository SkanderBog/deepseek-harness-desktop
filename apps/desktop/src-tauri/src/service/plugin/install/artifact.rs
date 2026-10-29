
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use tauri::{Emitter};

use serde_json::Value as JsonValue;

use super::append_command_output;
use super::errors;
use super::installed_name;
use super::load_presets;
use super::profile_dir;
use super::InstallTarget;
use super::{new_process_owner, run_plugin_process, PreinstallLogPayload, PREINSTALL_LOG_EVENT};
use crate::{AppHandle, WebviewWindow};

pub(super) fn verify_installed_products(
    app_handle: &AppHandle,
    targets: &[InstallTarget],
    command_output: &str,
) -> Result<(), String> {
    let node_modules = profile_dir(app_handle).join("node_modules");
    let mut missing = Vec::new();
    for target in targets {
        let Some(name) = target.name.as_deref() else {
            continue;
        };
        let manifest = node_modules.join(name).join("package.json");
        if manifest.is_file() {
            if let Err(e) = errors::clear(app_handle, &target.id) {
                log::warn!("failed to clear plugin error for {}: {e}", target.id);
            }
        } else {
            missing.push((target.id.clone(), manifest));
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    let detail = if let Some(source_detail) = bundled_source_missing_detail(app_handle, &missing) {
        source_detail
    } else {
        silent_install_failure_detail(&missing, command_output)
    };
    log::error!("{detail}");
    for (id, _) in &missing {
        if let Err(e) = errors::record(app_handle, id, "install", &detail) {
            log::warn!("failed to record plugin error for {id}: {e}");
        }
    }
    Err(detail)
}

fn silent_install_failure_detail(missing: &[(String, PathBuf)], command_output: &str) -> String {
    let ids = missing
        .iter()
        .map(|(id, _)| id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let manifests = missing
        .iter()
        .map(|(_, path)| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let output_hint = if command_output.trim().is_empty() {
        "The dsh plugin command produced no output."
    } else {
        "Review the preinstall log above for the command output."
    };
    format!(
        "PREINSTALL_SILENT_FAIL: dsh plugin exited with code 0, but no install artifact was created for [{ids}]. Expected package manifests: [{manifests}]. {output_hint} Retry the installation; if it repeats, include the preinstall log and these paths in the bug report."
    )
}

fn bundled_source_missing_detail(
    app_handle: &AppHandle,
    missing: &[(String, PathBuf)],
) -> Option<String> {
    let resolve = |id: &str| super::bundled_plugin_dir(app_handle, id);
    bundled_source_missing_detail_with(missing, &resolve)
}

fn bundled_source_missing_detail_with(
    missing: &[(String, PathBuf)],
    resolve: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<String> {
    let mut source_missing: Vec<String> = Vec::new();
    for (id, _) in missing {
        let Some(dir) = resolve(id) else {
            continue;
        };
        if !dir.join("package.json").is_file() {
            source_missing.push(format!("{id}（{}）", dir.display()));
        }
    }
    if source_missing.is_empty() {
        return None;
    }
    Some(format!(
        "BUNDLED_PLUGIN_SOURCE_MISSING: 应用资源中的内置插件源目录缺失或不可读，pnpm 静默跳过安装（exit 0 无产物），导致以下内置插件未落盘: {}. 这些源目录随应用安装包分发；若应用安装目录被移动/删除、盘符变更或资源目录被清理，会触发此问题。请重新安装应用或恢复应用资源目录后重试。",
        source_missing.join("; ")
    ))
}

fn declared_main_entry(pkg_dir: &Path) -> Option<PathBuf> {
    fn join_contained(pkg_dir: &Path, entry: &str) -> Option<PathBuf> {
        let candidate = Path::new(entry);
        if candidate.is_absolute()
            || candidate
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return None;
        }
        let joined = pkg_dir.join(candidate);
        joined.starts_with(pkg_dir).then_some(joined)
    }
    let from_main_field = |value: &JsonValue| -> Option<PathBuf> {
        value
            .get("main")
            .and_then(|v| v.as_str())
            .and_then(|main| join_contained(pkg_dir, main))
    };
    let plugin_manifest_text = std::fs::read_to_string(pkg_dir.join("dsh.plugin.json")).ok();
    if let Some(text) = plugin_manifest_text {
        if let Ok(value) = serde_json::from_str::<JsonValue>(&text) {
            if let Some(entry) = from_main_field(&value) {
                return Some(entry);
            }
        }
    }
    let pkg_text = std::fs::read_to_string(pkg_dir.join("package.json")).ok()?;
    let value = serde_json::from_str::<JsonValue>(&pkg_text).ok()?;
    if let Some(entry) = from_main_field(&value) {
        return Some(entry);
    }
    let exports = value.get("exports");
    if let Some(shorthand) = exports.and_then(|v| v.as_str()) {
        return join_contained(pkg_dir, shorthand);
    }
    let dot = exports.and_then(|v| v.get("."));
    for key in ["default", "import", "require"] {
        if let Some(entry) = dot
            .and_then(|d| d.get(key))
            .and_then(|v| v.as_str())
            .and_then(|s| join_contained(pkg_dir, s))
        {
            return Some(entry);
        }
    }
    None
}

pub(super) fn installed_package_name(app_handle: &AppHandle, id: &str) -> Option<String> {
    if let Some(preset) = load_presets(app_handle).iter().find(|p| p.id == id) {
        return Some(installed_name(preset).to_string());
    }
    let manifest_path = profile_dir(app_handle).join("package.json");
    let text = std::fs::read_to_string(&manifest_path).ok()?;
    let value = serde_json::from_str::<JsonValue>(&text).ok()?;
    value
        .get("dependencies")
        .and_then(|d| d.as_object())
        .and_then(|deps| {
            deps.keys()
                .find(|name| name.rsplit('/').next() == Some(id))
                .cloned()
        })
}

pub(super) async fn ensure_plugin_entry_built(
    app_handle: &AppHandle,
    id: &str,
    pkg_dir: &Path,
    envs: &HashMap<String, String>,
    window: &WebviewWindow,
) -> Result<(), String> {
    if !pkg_dir.is_dir() {
        return Err(format!(
            "PLUGIN_ENTRY_MISSING: {id} 未安装（{} 不存在），请重新安装",
            pkg_dir.display()
        ));
    }
    let Some(entry) = declared_main_entry(pkg_dir) else {
        log::warn!("plugin {id} declares no verifiable main entry, skipping build verify");
        return Ok(());
    };
    if entry.is_file() {
        return Ok(());
    }

    let Some((program, pre_args)) = super::super::verify::pnpm_direct(app_handle).await? else {
        return Err(format!(
            "PNPM_NOT_FOUND: no usable pnpm (bundled or user) to rebuild plugin {id}"
        ));
    };
    log::warn!(
        "plugin {id} declared entry {} is missing; rebuilding in place",
        entry.display()
    );
    let attempts: [(&str, &[&str]); 3] = [
        ("build", &["run", "build"]),
        ("prepare", &["run", "prepare"]),
        (
            "tsdown --config-loader unrun",
            &["exec", "tsdown", "--config-loader", "unrun"],
        ),
    ];
    let mut all_output = String::new();
    for (label, args) in attempts {
        let _ = window.emit(
            PREINSTALL_LOG_EVENT,
            PreinstallLogPayload {
                line: format!(
                    "[pnpm] {id} 缺少构建产物（{}），正在补构建（{label}）…",
                    entry.display()
                ),
            },
        );
        let mut full_args = pre_args.clone();
        full_args.extend(args.iter().map(|a| OsString::from(*a)));
        match run_plugin_process(
            &program,
            &full_args,
            pkg_dir,
            envs,
            window,
            new_process_owner(),
        )
        .await
        {
            Ok((code, output)) => {
                append_command_output(&mut all_output, &output);
                if entry.is_file() {
                    if code != 0 {
                        log::warn!(
                            "plugin {id} build via {label} exited {code} but entry {} exists",
                            entry.display()
                        );
                    }
                    log::info!("plugin {id} rebuilt via {label}: {}", entry.display());
                    return Ok(());
                }
                log::warn!("plugin {id} build via {label} exited {code}; trying next strategy");
            }
            Err(e) => {
                log::warn!(
                    "plugin {id} build via {label} failed to spawn: {e}; trying next strategy"
                );
            }
        }
    }
    let tail = all_output
        .lines()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    Err(format!(
        "PLUGIN_ENTRY_MISSING: {id} 声明的入口 {} 构建失败（build/prepare 均未产出），尾部日志：\n{tail}",
        entry.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn silent_install_failure_reports_empty_output_and_expected_artifact() {
        let missing = vec![(
            "dsh-win-terminal-inspector".to_string(),
            PathBuf::from(
                "C:\\Users\\test\\.dsh\\profiles\\web\\node_modules\\dsh-win-terminal-inspector\\package.json",
            ),
        )];

        let detail = silent_install_failure_detail(&missing, "  \r\n");

        assert!(detail.starts_with("PREINSTALL_SILENT_FAIL:"));
        assert!(detail.contains("dsh-win-terminal-inspector"));
        assert!(detail.contains("package.json"));
        assert!(detail.contains("produced no output"));
        assert!(detail.contains("Retry the installation"));
    }

    #[test]
    fn silent_install_failure_points_to_existing_command_log() {
        let missing = vec![(
            "dshmarket".to_string(),
            PathBuf::from("/tmp/profile/node_modules/dshmarket/package.json"),
        )];

        let detail = silent_install_failure_detail(&missing, "pnpm completed\n");

        assert!(detail.contains("Review the preinstall log above"));
        assert!(!detail.contains("produced no output"));
    }

    #[test]
    fn bundled_source_missing_reports_actionable_detail() {
        let missing = vec![
            (
                "dsh-tauri".to_string(),
                PathBuf::from("C:\\Users\\t\\.dsh\\profiles\\safe\\node_modules\\dsh-tauri\\package.json"),
            ),
            (
                "dsh-tauri-ui".to_string(),
                PathBuf::from("C:\\Users\\t\\.dsh\\profiles\\safe\\node_modules\\dsh-tauri-ui\\package.json"),
            ),
        ];
        let resolve = |id: &str| {
            Some(PathBuf::from(format!(
                "E:\\Deepseek Harness Desktop\\resources\\node_modules\\{id}"
            )))
        };

        let detail = bundled_source_missing_detail_with(&missing, &resolve).expect("diagnosed");

        assert!(detail.starts_with("BUNDLED_PLUGIN_SOURCE_MISSING:"));
        assert!(detail.contains("dsh-tauri"));
        assert!(detail.contains("dsh-tauri-ui"));
        assert!(detail.contains("Deepseek Harness Desktop"));
        assert!(detail.contains("resources"));
        assert!(detail.contains("node_modules"));
        assert!(detail.contains("重新安装应用"));
    }

    #[test]
    fn bundled_source_present_falls_back_to_generic_detail() {
        let root = std::env::temp_dir().join(format!(
            "dsh-source-present-test-{}",
            std::process::id()
        ));
        let dir = root.join("dsh-tauri");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("package.json"), r#"{"name":"dsh-tauri"}"#).unwrap();

        let missing = vec![(
            "dsh-tauri".to_string(),
            root.join("node_modules/dsh-tauri/package.json"),
        )];
        let resolve = |_: &str| Some(dir.clone());

        assert!(bundled_source_missing_detail_with(&missing, &resolve).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn non_internal_missing_plugin_falls_back_to_generic_detail() {
        let missing = vec![(
            "dshmarket".to_string(),
            PathBuf::from("/tmp/profile/node_modules/dshmarket/package.json"),
        )];
        let resolve = |_: &str| None;

        assert!(bundled_source_missing_detail_with(&missing, &resolve).is_none());
    }

    fn tmp_pkg_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dsh-entry-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn declared_main_prefers_dsh_plugin_manifest() {
        let dir = tmp_pkg_dir("manifest");
        std::fs::write(
            dir.join("dsh.plugin.json"),
            r#"{"id":"x","main":"./lib/index.js"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"x","main":"dist/index.js"}"#,
        )
        .unwrap();
        assert_eq!(declared_main_entry(&dir).unwrap(), dir.join("lib/index.js"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn declared_main_falls_back_to_package_json() {
        let dir = tmp_pkg_dir("pkgjson");
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"x","main":"lib/index.js"}"#,
        )
        .unwrap();
        assert_eq!(declared_main_entry(&dir).unwrap(), dir.join("lib/index.js"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn declared_main_reads_exports_default() {
        let dir = tmp_pkg_dir("exports");
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"x","exports":{".":{"types":"./lib/types.d.ts","default":"./lib/index.js"}}}"#,
        )
        .unwrap();
        assert_eq!(declared_main_entry(&dir).unwrap(), dir.join("lib/index.js"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn declared_main_reads_exports_string_shorthand_and_require() {
        let dir = tmp_pkg_dir("exports-shorthand");
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"x","exports":"./dist/main.js"}"#,
        )
        .unwrap();
        assert_eq!(declared_main_entry(&dir).unwrap(), dir.join("dist/main.js"));
        let _ = std::fs::remove_dir_all(&dir);

        let dir2 = tmp_pkg_dir("exports-require");
        std::fs::write(
            dir2.join("package.json"),
            r#"{"name":"x","exports":{".":{"require":"./cjs/index.js"}}}"#,
        )
        .unwrap();
        assert_eq!(
            declared_main_entry(&dir2).unwrap(),
            dir2.join("cjs/index.js")
        );
        let _ = std::fs::remove_dir_all(&dir2);
    }

    #[test]
    fn declared_main_rejects_escaping_or_absolute_entries() {
        let dir = tmp_pkg_dir("escape");
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"x","main":"../outside/index.js"}"#,
        )
        .unwrap();
        assert!(declared_main_entry(&dir).is_none());
        let absolute_entry = if cfg!(windows) {
            "C:\\\\outside\\\\index.js"
        } else {
            "/outside/index.js"
        };
        std::fs::write(
            dir.join("package.json"),
            format!(r#"{{"name":"x","main":"{}"}}"#, absolute_entry),
        )
        .unwrap();
        assert!(declared_main_entry(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn declared_main_falls_through_broken_plugin_manifest() {
        let dir = tmp_pkg_dir("broken-plugin-manifest");
        std::fs::write(dir.join("dsh.plugin.json"), r#"not-json"#).unwrap();
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"x","main":"lib/index.js"}"#,
        )
        .unwrap();
        assert_eq!(declared_main_entry(&dir).unwrap(), dir.join("lib/index.js"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn declared_main_none_without_manifest() {
        let dir = tmp_pkg_dir("none");
        assert!(declared_main_entry(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
