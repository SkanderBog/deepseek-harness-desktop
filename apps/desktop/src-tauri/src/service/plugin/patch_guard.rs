//! home 层（home 层作用于所有档案，安全档案不例外），连兜底一起失效（issue #525）。

use std::path::{Path, PathBuf};

use super::patch_entries::{
    scan_unresolved_entries_in, unresolved_entries_message, PATCH_ENTRY_UNRESOLVED,
};
use super::profile_dir;
use crate::config;
use crate::{AppHandle};

const PATCH_FILENAME: &str = "cordis.patch.yml";

#[derive(Debug, Clone, serde::Serialize)]
pub struct QuarantinedPatchLayer {
    pub original: String,
    pub backup: String,
    pub error: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PatchQuarantineFailure {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct PatchQuarantineReport {
    pub quarantined: Vec<QuarantinedPatchLayer>,
    pub failures: Vec<PatchQuarantineFailure>,
}

impl PatchQuarantineReport {
    /// 启动再次大声失败，于是又回到同一个失败循环（issue #525 的形态）。此时应把
    pub fn has_failures(&self) -> bool {
        !self.failures.is_empty()
    }
}

pub(crate) fn patch_layer_paths(profile_dir: &Path, dsh_home: &Path) -> [PathBuf; 2] {
    [
        profile_dir.join(PATCH_FILENAME),
        dsh_home.join(PATCH_FILENAME),
    ]
}

pub(crate) fn preflight_at(
    profile: &Path,
    data_home: &Path,
    install_anchor: Option<&Path>,
) -> Result<(), String> {
    let entries = scan_unresolved_entries_in(profile, data_home, install_anchor);
    if entries.is_empty() {
        return Ok(());
    }
    let names = entries
        .iter()
        .map(|entry| format!("{}:{} {}", entry.layer, entry.line, entry.name))
        .collect::<Vec<_>>()
        .join(", ");
    log::error!("{PATCH_ENTRY_UNRESOLVED}: {names}");
    Err(unresolved_entries_message(&entries))
}

pub(super) const PATCH_ENCODING_FAILED: &str = "INTERNAL_PLUGIN_PATCH_ENCODING_FAILED";

fn parse_error(path: &Path) -> Option<String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            return Some(format!("{PATCH_ENCODING_FAILED}: {error}"));
        }
        Err(_) => return None,
    };
    serde_yaml::from_str::<serde_yaml::Value>(&raw)
        .err()
        .map(|error| error.to_string())
}

pub(super) fn backup_path(path: &Path, suffix: &str, stamp: &str) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut candidate = path.with_file_name(format!("{name}.{suffix}-{stamp}"));
    let mut extra = 2;
    while candidate.exists() {
        candidate = path.with_file_name(format!("{name}.{suffix}-{stamp}-{extra}"));
        extra += 1;
    }
    candidate
}

fn quarantine_layers(paths: &[PathBuf], stamp: &str) -> PatchQuarantineReport {
    let mut report = PatchQuarantineReport::default();
    for path in paths {
        let Some(error) = parse_error(path) else {
            continue;
        };
        let backup = backup_path(path, "broken", stamp);
        match std::fs::rename(path, &backup) {
            Ok(()) => {
                log::warn!(
                    "PATCH_LAYER_QUARANTINED: {} -> {} ({error})",
                    path.display(),
                    backup.display()
                );
                report.quarantined.push(QuarantinedPatchLayer {
                    original: path.display().to_string(),
                    backup: backup.display().to_string(),
                    error,
                });
            }
            Err(rename_error) => {
                let message = rename_error.to_string();
                log::warn!(
                    "PATCH_LAYER_QUARANTINE_FAILED: {}: {message}",
                    path.display()
                );
                report.failures.push(PatchQuarantineFailure {
                    path: path.display().to_string(),
                    error: message,
                });
            }
        }
    }
    report
}

pub(super) fn now_stamp() -> String {
    use time::OffsetDateTime;
    let now = OffsetDateTime::now_utc();
    let date = now.date();
    let time = now.time();
    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}",
        date.year(),
        date.month() as u8,
        date.day(),
        time.hour(),
        time.minute(),
        time.second()
    )
}

pub(crate) fn quarantine_patch_layers_in(
    profile_dir: &Path,
    dsh_home: &Path,
) -> PatchQuarantineReport {
    quarantine_layers(&patch_layer_paths(profile_dir, dsh_home), &now_stamp())
}

pub(crate) fn quarantine_active_patch_layers(app_handle: &AppHandle) -> PatchQuarantineReport {
    quarantine_patch_layers_in(
        &profile_dir(app_handle),
        &config::get_dsh_data_path(app_handle),
    )
}

pub(crate) fn quarantine_failure_message(report: &PatchQuarantineReport) -> String {
    let detail = report
        .failures
        .iter()
        .map(|failure| format!("{}: {}", failure.path, failure.error))
        .collect::<Vec<_>>()
        .join("; ");
    format!("PATCH_LAYER_QUARANTINE_FAILED: {detail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsh-patch-guard-{name}-{}-{}",
            std::process::id(),
            now_stamp()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// issue #525 的真实形态：`!!js` 表达式带 `: ` 却没加引号。
    const BROKEN_UNQUOTED_JS: &str = r#"- id: session-telemetry-otel
  config:
    YUNXIAO_ACCESS_TOKEN: !!js process.env.TOKEN ? process.env.TOKEN : (function () { return '' })()
"#;
    const VALID_LAYER: &str = "- id: dsh-base\n  disabled: false\n";

    #[test]
    fn preflight_at_rereads_both_layers_and_uses_the_real_install_anchor_resolution() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let profile = home.join("profiles/probe");
        std::fs::create_dir_all(&profile).unwrap();
        let manifest = profile.join("package.json");
        std::fs::write(
            &manifest,
            r#"{"dependencies":{"dsh-preflight-probe":"1.0.0"}}"#,
        )
        .unwrap();
        let layer = "- insert:\n    - id: probe\n      name: dsh-preflight-probe\n";
        let paths = patch_layer_paths(&profile, &home);
        for path in &paths {
            std::fs::write(path, layer).unwrap();
        }

        let error = preflight_at(&profile, &home, None).unwrap_err();
        let payload: serde_json::Value = serde_json::from_str(
            error
                .strip_prefix("PATCH_LAYER_ENTRY_UNRESOLVED: ")
                .unwrap(),
        )
        .unwrap();
        let entries = payload["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        for (entry, path) in entries.iter().zip(&paths) {
            assert_eq!(entry["layer"], path.display().to_string());
            assert_eq!(entry["line"], 3);
            assert_eq!(entry["name"], "dsh-preflight-probe");
            assert_eq!(entry["declared"], true);
            assert_eq!(std::fs::read_to_string(path).unwrap(), layer);
        }

        let anchor = temp.path().join("core/lib/bin.js");
        let installed = temp.path().join("core/node_modules/dsh-preflight-probe");
        std::fs::create_dir_all(&installed).unwrap();
        let package = installed.join("package.json");
        std::fs::write(&package, "{}\n").unwrap();
        assert_eq!(preflight_at(&profile, &home, Some(&anchor)), Ok(()));
        std::fs::remove_file(package).unwrap();
        assert!(preflight_at(&profile, &home, Some(&anchor))
            .unwrap_err()
            .starts_with("PATCH_LAYER_ENTRY_UNRESOLVED: "));
    }

    #[test]
    fn parse_error_reports_unquoted_js_expression() {
        let dir = tmp_dir("parse-error");
        let path = dir.join(PATCH_FILENAME);
        std::fs::write(&path, BROKEN_UNQUOTED_JS).unwrap();

        assert!(parse_error(&path).is_some());
        std::fs::write(&path, VALID_LAYER).unwrap();
        assert!(parse_error(&path).is_none());
        assert!(parse_error(&dir.join("missing.yml")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quarantine_preserves_non_utf8_bytes_in_both_layers() {
        for bytes in [
            b"\xff\xfe-\0 \0i\0d\0:\0 \0x\0\n\0".as_slice(),
            b"- id: plugin\n  name: \xc4\xe3\xba\xc3\n".as_slice(),
            b"# truncated \xe4\xb8".as_slice(),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let home = temp.path();
            let profile = home.join("profiles/tauri");
            std::fs::create_dir_all(&profile).unwrap();
            let paths = patch_layer_paths(&profile, home);
            for path in &paths {
                std::fs::write(path, bytes).unwrap();
                let error = parse_error(path).expect("encoding damage must be quarantinable");
                assert!(error.contains("UTF-8"), "{error}");
                assert_eq!(std::fs::read(path).unwrap(), bytes);
            }

            let report = quarantine_patch_layers_in(&profile, home);
            assert!(!report.has_failures(), "{report:?}");
            assert_eq!(report.quarantined.len(), 2);
            for (record, path) in report.quarantined.iter().zip(&paths) {
                assert_eq!(record.original, path.display().to_string());
                assert!(!path.exists());
                assert_eq!(std::fs::read(&record.backup).unwrap(), bytes);
            }
            let repeated = quarantine_patch_layers_in(&profile, home);
            assert!(repeated.quarantined.is_empty());
            assert!(!repeated.has_failures());
        }
    }

    #[test]
    fn quarantine_does_not_mistake_io_errors_for_encoding_damage() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(PATCH_FILENAME);
        std::fs::create_dir(&path).unwrap();
        assert!(parse_error(&path).is_none());
        let report = quarantine_layers(std::slice::from_ref(&path), "20261017000000");
        assert!(report.quarantined.is_empty());
        assert!(report.failures.is_empty());
        assert!(path.is_dir());
    }

    #[test]
    fn quarantine_moves_only_unparsable_layers() {
        let profile = tmp_dir("quarantine-profile");
        let home = tmp_dir("quarantine-home");
        let profile_patch = profile.join(PATCH_FILENAME);
        let home_patch = home.join(PATCH_FILENAME);
        std::fs::write(&profile_patch, VALID_LAYER).unwrap();
        std::fs::write(&home_patch, BROKEN_UNQUOTED_JS).unwrap();

        let report = quarantine_patch_layers_in(&profile, &home);

        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.quarantined.len(), 1);
        let record = &report.quarantined[0];
        assert_eq!(record.original, home_patch.display().to_string());
        assert!(!home_patch.exists());
        assert_eq!(
            std::fs::read_to_string(&record.backup).unwrap(),
            BROKEN_UNQUOTED_JS
        );
        assert_eq!(
            std::fs::read_to_string(&profile_patch).unwrap(),
            VALID_LAYER
        );
        assert_eq!(std::fs::read_dir(&profile).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&profile);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn quarantine_is_idempotent() {
        let profile = tmp_dir("idempotent-profile");
        let home = tmp_dir("idempotent-home");
        std::fs::write(home.join(PATCH_FILENAME), BROKEN_UNQUOTED_JS).unwrap();

        assert_eq!(
            quarantine_patch_layers_in(&profile, &home)
                .quarantined
                .len(),
            1
        );
        let second = quarantine_patch_layers_in(&profile, &home);
        assert!(second.quarantined.is_empty());
        assert!(second.failures.is_empty());
        let _ = std::fs::remove_dir_all(&profile);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn backup_path_never_overwrites_existing_backup() {
        let dir = tmp_dir("backup-collision");
        let patch = dir.join(PATCH_FILENAME);
        std::fs::write(&patch, BROKEN_UNQUOTED_JS).unwrap();
        let first = dir.join(format!("{PATCH_FILENAME}.broken-20260914123456"));
        std::fs::write(&first, "keep me\n").unwrap();

        let report = quarantine_layers(std::slice::from_ref(&patch), "20260914123456");

        assert_eq!(report.quarantined.len(), 1);
        let record = &report.quarantined[0];
        assert!(record.backup.ends_with("-2"), "{}", record.backup);
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "keep me\n");
        assert_eq!(
            std::fs::read_to_string(&record.backup).unwrap(),
            BROKEN_UNQUOTED_JS
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quarantine_ignores_valid_and_missing_layers() {
        let profile = tmp_dir("valid-profile");
        let home = tmp_dir("valid-home");
        std::fs::write(profile.join(PATCH_FILENAME), VALID_LAYER).unwrap();
        std::fs::write(home.join(PATCH_FILENAME), "[]\n").unwrap();

        let report = quarantine_patch_layers_in(&profile, &home);

        assert!(report.quarantined.is_empty());
        assert!(report.failures.is_empty());
        assert_eq!(
            std::fs::read_to_string(profile.join(PATCH_FILENAME)).unwrap(),
            VALID_LAYER
        );
        let _ = std::fs::remove_dir_all(&profile);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn patch_layer_paths_are_profile_then_home() {
        let paths = patch_layer_paths(Path::new("/profiles/safe"), Path::new("/dsh-home"));
        assert_eq!(paths[0], Path::new("/profiles/safe").join(PATCH_FILENAME));
        assert_eq!(paths[1], Path::new("/dsh-home").join(PATCH_FILENAME));
    }

    #[test]
    fn report_flags_failures_and_renders_message() {
        let broken = PathBuf::from("/profiles/safe/cordis.patch.yml");
        let report = PatchQuarantineReport {
            quarantined: Vec::new(),
            failures: vec![PatchQuarantineFailure {
                path: broken.display().to_string(),
                error: "Access is denied. (os error 5)".to_string(),
            }],
        };

        assert!(report.has_failures());
        let message = quarantine_failure_message(&report);
        assert!(
            message.starts_with("PATCH_LAYER_QUARANTINE_FAILED: "),
            "{message}"
        );
        assert!(message.contains(&broken.display().to_string()), "{message}");
        assert!(message.contains("os error 5"), "{message}");
    }

    #[test]
    fn successful_quarantine_has_no_failures() {
        let profile = tmp_dir("success-profile");
        let home = tmp_dir("success-home");
        std::fs::write(home.join(PATCH_FILENAME), BROKEN_UNQUOTED_JS).unwrap();

        let report = quarantine_patch_layers_in(&profile, &home);

        assert!(!report.has_failures());
        assert_eq!(report.quarantined.len(), 1);
        let _ = std::fs::remove_dir_all(&profile);
        let _ = std::fs::remove_dir_all(&home);
    }
}
