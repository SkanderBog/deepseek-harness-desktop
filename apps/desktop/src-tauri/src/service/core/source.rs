use crate::config;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::OnceLock;

use super::local::{local_core, local_core_package_dir};
use crate::AppHandle;
use crate::service::download::parse_version_from_tag;

static UNSUPPORTED_LOCAL_WARNED: OnceLock<()> = OnceLock::new();

fn meets_baseline(version: &str, baseline: &str) -> bool {
    match (
        semver::Version::parse(version),
        semver::Version::parse(baseline),
    ) {
        (Ok(actual), Ok(baseline)) => actual >= baseline,
        _ => true,
    }
}

pub(super) fn core_supports_bundled_plugins(version: &str, baseline: Option<&str>) -> bool {
    match baseline {
        Some(baseline) => meets_baseline(version, baseline),
        None => true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CoreSource {
    Local,
    App,
}

impl CoreSource {
    pub fn as_str(self) -> &'static str {
        match self {
            CoreSource::Local => "local",
            CoreSource::App => "app",
        }
    }

    pub fn parse(source: &str) -> Option<CoreSource> {
        match source {
            "local" => Some(CoreSource::Local),
            "app" => Some(CoreSource::App),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessCore {
    pub id: String,
    pub source: CoreSource,
    pub version: String,
    pub tag: String,
    pub path: String,
    pub dir: String,
    pub present: bool,
    pub active: bool,
    /// 失败（issue #790）。
    pub removable: bool,
    pub preview: bool,
    pub above_recommended: bool,
    pub orphaned: bool,
    pub bundled: bool,
    pub recommended_version: Option<String>,
    pub error: Option<String>,
}

fn warn_unsupported_local_core(version: &str, baseline: Option<&str>) {
    if UNSUPPORTED_LOCAL_WARNED.set(()).is_err() {
        return;
    }
    log::warn!(
        "CORE_LOCAL_UNSUPPORTED: local dsh {} is below the minimum supported core {}; using the bundled core instead (issue #596)",
        version,
        baseline.unwrap_or("<unset>"),
    );
}

pub fn active_source<R: tauri::Runtime>(app_handle: &AppHandle<R>) -> CoreSource {
    let setting = config::get_store_dat_setting(app_handle);
    let local = local_core(app_handle);
    let baseline = config::manifest::minimum_dsh_version(app_handle);
    let local_usable = local
        .as_ref()
        .is_some_and(|core| core_supports_bundled_plugins(&core.version, baseline.as_deref()));
    if local.is_some() && !local_usable {
        warn_unsupported_local_core(
            local
                .as_ref()
                .map(|core| core.version.as_str())
                .unwrap_or_default(),
            baseline.as_deref(),
        );
    }
    match setting.active_core.as_deref().and_then(CoreSource::parse) {
        Some(CoreSource::App) => CoreSource::App,
        Some(CoreSource::Local) if local_usable => CoreSource::Local,
        _ if local_usable => CoreSource::Local,
        _ => CoreSource::App,
    }
}

pub fn active_dsh_binary(app_handle: &AppHandle) -> PathBuf {
    match active_source(app_handle) {
        CoreSource::Local => local_core(app_handle)
            .map(|c| c.bin)
            .unwrap_or_else(|| config::get_dsh_binary_path(app_handle)),
        CoreSource::App => config::get_dsh_binary_path(app_handle),
    }
}

pub fn active_core_dir(app_handle: &AppHandle) -> PathBuf {
    match active_source(app_handle) {
        CoreSource::Local => local_core_package_dir(app_handle)
            .unwrap_or_else(|| config::get_dsh_install_path(app_handle)),
        CoreSource::App => config::get_dsh_install_path(app_handle),
    }
}

pub fn active_is_bundled(app_handle: &AppHandle) -> bool {
    active_source(app_handle) == CoreSource::App
        && config::dependencies::bundled_core_dir(app_handle).is_some_and(|dir| {
            config::dependencies::active_root(app_handle, config::dependencies::DEP_DSH) == dir
        })
}

pub fn active_version(app_handle: &AppHandle) -> Option<String> {
    match active_source(app_handle) {
        CoreSource::Local => local_core(app_handle).map(|c| c.version),
        CoreSource::App => {
            if active_is_bundled(app_handle) {
                return config::get_dsh_version(app_handle);
            }
            config::get_dsh_pkg_tag(app_handle)
                .as_deref()
                .and_then(parse_version_from_tag)
                .or_else(|| config::get_dsh_version(app_handle))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_gate_accepts_equal_or_newer_versions() {
        let baseline = "0.1.7-rc.1";
        for version in ["0.1.7-rc.1", "0.1.7-rc.2", "0.1.7", "0.2.1-rc.1"] {
            assert!(meets_baseline(version, baseline), "version={version}");
        }
    }

    #[test]
    fn baseline_gate_rejects_older_versions() {
        let baseline = "0.1.7-rc.1";
        for version in [
            "0.1.0-rc.7",
            "0.1.5-rc.1",
            "0.1.6-alpha.2",
            "0.1.6",
            "0.1.7-alpha.1",
            "0.1.7-beta.1",
            "0.1.7-rc.0",
        ] {
            assert!(!meets_baseline(version, baseline), "version={version}");
        }
    }

    #[test]
    fn baseline_gate_passes_unparsable_versions() {
        assert!(meets_baseline("", "0.1.7-rc.1"));
        assert!(meets_baseline("not-a-version", "0.1.7-rc.1"));
        assert!(meets_baseline("0.1.7-rc.1", ""));
    }

    #[test]
    fn missing_baseline_keeps_local_core_usable() {
        assert!(core_supports_bundled_plugins("0.1.6", None));
        assert!(!core_supports_bundled_plugins("0.1.6", Some("0.1.7-rc.1")));
    }
}
