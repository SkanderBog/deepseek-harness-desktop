use regex::Regex;
use serde::{Deserialize, Serialize};

use super::errors;

use crate::AppHandle;
use extract::{
    classify_reason, extract_duplicate_loader_entry, extract_plugin_refs, extract_slot_conflict,
};
use ownership::resolve_recovery_plugins;
pub(crate) use uninstall::patch_entry_targets;
use uninstall::{remove_plugin_dir, remove_plugin_from_manifest, strip_cordis_patch_for};

pub(crate) use super::installed::profile_dir;

mod extract;
mod ownership;
mod uninstall;

pub(crate) const RECOVERY_REQUIRED_EVENT: &str = "plugin-recovery-required";

pub(crate) fn is_core_package(name: &str) -> bool {
    name.starts_with("@deepseek-ai/")
}

pub(crate) fn is_package_name(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.contains(':') || s.chars().any(|c| c.is_whitespace()) {
        return false;
    }

    let (scope, name) = if let Some((scope, name)) = s.split_once('/') {
        if !s.starts_with('@') || name.is_empty() || name.contains('/') {
            return false;
        }
        (Some(scope), name)
    } else {
        if s.starts_with('@') {
            return false;
        }
        (None, s)
    };

    fn valid_component(component: &str, is_scope: bool) -> bool {
        let component = if is_scope {
            component.strip_prefix('@').unwrap_or(component)
        } else {
            component
        };
        !component.is_empty()
            && component != "."
            && component != ".."
            && !component.starts_with('.')
            && !component.ends_with('.')
            && !component.starts_with("..")
            && !component.ends_with("..")
            && component
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    }

    if scope.is_some_and(|scope| !valid_component(scope, true)) {
        return false;
    }
    valid_component(name, false)
}

pub(crate) fn is_actionable_plugin_ref(s: &str) -> bool {
    is_package_name(s) && !is_core_package(s.trim())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginRecoveryInfo {
    pub plugins: Vec<String>,
    pub reason: String,
    pub detail: String,
    pub raw_error: String,
}

pub fn detect(app_handle: &AppHandle, log_lines: &[String]) -> PluginRecoveryInfo {
    let text = log_lines.join("\n");
    let refs = extract_plugin_refs(&text);
    let duplicate_entry = extract_duplicate_loader_entry(&text);
    let slot_conflict = extract_slot_conflict(&text);
    let (reason, detail) = classify_reason(&text);
    let plugins = resolve_recovery_plugins(
        app_handle,
        &refs,
        duplicate_entry.as_deref(),
        slot_conflict.as_deref(),
    );
    let raw_error = refs_text(&text);
    PluginRecoveryInfo {
        plugins,
        reason,
        detail,
        raw_error,
    }
}

fn refs_text(text: &str) -> String {
    let marker_any =
        Regex::new(r"(?i)error|duplicate|fatal|panic|throw|failed|exception|✖").expect("literal");
    let mut err_lines: Vec<&str> = text
        .lines()
        .filter(|l| marker_any.is_match(l))
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    if err_lines.is_empty() {
        let tail: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let start = tail.len().saturating_sub(8);
        err_lines = tail[start..].to_vec();
    }
    let joined = err_lines.join("\n");
    joined.chars().take(2000).collect()
}

pub fn uninstall(app_handle: &AppHandle, id: &str) -> Result<(), String> {
    if !is_actionable_plugin_ref(id) {
        return Err(format!(
            "PLUGIN_RECOVERY_REFUSED: refusing to remove core/official package {id}"
        ));
    }
    let profile = profile_dir(app_handle);
    let manifest_path = profile.join("package.json");
    if !manifest_path.exists() {
        return Err("PLUGIN_RECOVERY_NO_MANIFEST: profile package.json missing".to_string());
    }
    let content = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("PLUGIN_RECOVERY_READ: {e}"))?;
    let mut manifest: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("PLUGIN_RECOVERY_PARSE: {e}"))?;

    let modified = remove_plugin_from_manifest(&mut manifest, id);
    if modified {
        let rendered = serde_json::to_string_pretty(&manifest)
            .map_err(|e| format!("PLUGIN_RECOVERY_RENDER: {e}"))?;
        std::fs::write(&manifest_path, format!("{rendered}\n"))
            .map_err(|e| format!("PLUGIN_RECOVERY_WRITE: {e}"))?;
        log::info!("Recovery uninstall removed plugin {id} from profile manifest");
    }

    remove_plugin_dir(&profile, id);
    strip_cordis_patch_for(&profile, id);
    if let Err(e) = std::fs::remove_file(profile.join("pnpm-lock.yaml")) {
        if e.kind() != std::io::ErrorKind::NotFound {
            log::warn!("failed to remove pnpm-lock.yaml during recovery: {e}");
        }
    }
    if let Err(e) = errors::clear(app_handle, id) {
        log::warn!("failed to clear plugin error for {id} during recovery: {e}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_package_predicate_preserves_exact_prefix_boundary() {
        for (name, expected) in [
            ("@deepseek-ai/pkg", true),
            ("@deepseek-ai/", true),
            ("@deepseek-ai", false),
            ("@deepseek-ai-extra/pkg", false),
            (" @deepseek-ai/pkg", false),
            ("@Deepseek-ai/pkg", false),
            ("dshmarket", false),
            ("", false),
        ] {
            assert_eq!(is_core_package(name), expected, "{name}");
        }
    }

    #[test]
    fn package_name_validation() {
        assert!(is_package_name("dshmarket"));
        assert!(is_package_name("@scope/pkg"));
        assert!(is_package_name("dsh-better-sidebar"));
        assert!(!is_package_name(""));
        assert!(!is_package_name("has space"));
        assert!(!is_package_name("with:colon"));
        assert!(!is_package_name("@bare"));
        assert!(!is_package_name("foo/../../target"));
        assert!(!is_package_name("@scope/../../target"));
        assert!(!is_package_name("@scope/pkg/extra"));
        assert!(!is_package_name(r"foo\..\target"));
        assert!(!is_package_name(r"@scope\pkg"));
        assert!(!is_package_name("@scope/@pkg"));
        assert!(!is_package_name("@scope/.."));
        assert!(!is_package_name("@scope/."));
        assert!(!is_package_name(".pnpm"));
        assert!(!is_package_name(".hidden"));
        assert!(!is_package_name("foo."));
        assert!(!is_package_name("@.scope/pkg"));
        assert!(!is_package_name("@scope/.hidden"));
        assert!(!is_package_name("@scope/foo."));
        assert!(is_package_name("foo.bar"));
        assert!(is_package_name("@scope/pkg.name"));
        assert!(is_actionable_plugin_ref("dshmarket"));
        assert!(is_actionable_plugin_ref("dsh-better-sidebar"));
    }

    #[test]
    fn official_packages_remain_non_actionable() {
        assert!(is_package_name("@deepseek-ai/dsh-base"));
        assert!(is_package_name("@deepseek-ai/dsh-client-ui-chat"));
        assert!(!is_actionable_plugin_ref("@deepseek-ai/dsh-base"));
        assert!(!is_actionable_plugin_ref("@deepseek-ai/dsh-client-ui-chat"));
    }

    #[test]
    fn recovery_info_serializes_camel_case_fields() {
        let info = PluginRecoveryInfo {
            plugins: vec!["dsh-tauri".to_string()],
            reason: "load_failed".to_string(),
            detail: String::new(),
            raw_error: "failed to apply loader entry 644301cc (dsh-tauri)".to_string(),
        };
        let json = serde_json::to_value(&info).expect("serialize recovery info");

        assert_eq!(
            json["rawError"],
            "failed to apply loader entry 644301cc (dsh-tauri)"
        );
        assert_eq!(json["plugins"][0], "dsh-tauri");
        assert_eq!(json["reason"], "load_failed");
        assert!(json.get("raw_error").is_none());
    }
}
