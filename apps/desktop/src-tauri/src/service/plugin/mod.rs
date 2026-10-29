
mod atomic_write;
mod cancel;
pub mod compat;
pub mod disable;
pub mod errors;
mod hmr;
mod install;
mod installed;
mod internal;
mod patch_entries;
mod patch_guard;
mod patch_yaml;
mod preset;
mod process;
pub mod recovery;
mod safe;
pub mod snapshot;
pub mod update;
pub mod verify;
pub mod watch;

pub(crate) use crate::service::profile::ensure_profile_pnpm_policy;
pub use cancel::cancel;
pub(crate) use cancel::terminate_active_installs_blocking;
pub use disable::{disable, enable};
pub(crate) use hmr::sync_layer_at as local_plugin_hmr_sync_at;
pub use hmr::LocalHmrStatus;
pub(crate) use hmr::{status as local_plugin_hmr_status, sync_layer as local_plugin_hmr_sync};
pub(crate) use install::harness_prefer_bundled_pnpm_from;
pub(crate) use install::local_spec_from_path;
pub(crate) use install::packument;
pub use install::{
    allow_policy_versions, allow_version_exemptions, inspect_specs, install, install_specs,
    remove_many, update_many, IncompatibleVersion, PolicyBlockedVersion,
};
pub(crate) use installed::{
    declared_packages, ensure_npmrc_at, ensure_profile_npmrc, installed_name, list_installed,
    profile_dir, ProfilePackageJson,
};
pub use installed::{list, PreinstallPlugin};
pub(crate) use internal::cancel as cancel_internal_plugins;
pub(crate) use internal::ensure as ensure_internal_plugins;
pub(crate) use internal::{prepare_early_plugins, EarlyPluginState};
pub(crate) use patch_entries::{strip_active_unresolved_entries, PatchEntryStripReport};
pub(crate) use patch_guard::preflight_at as preflight_patch_entries_at;
pub(crate) use patch_guard::{
    patch_layer_paths, quarantine_active_patch_layers, quarantine_failure_message,
    quarantine_patch_layers_in, PatchQuarantineReport,
};
pub(crate) use patch_yaml::{parse_patch_layer, render_patch_layer};
pub use preset::repo_url_of;
pub(crate) use preset::{
    bundled_plugin_dir, bundled_plugin_dir_from, current_preset_hash, load_presets,
    preinstall_pending, presets_from_manifest, remove_legacy_bundled_plugins,
};
pub(crate) use process::{acquire_operation_lock, is_process_cleanup_failure};
pub use recovery::{
    detect as detect_recovery, uninstall as uninstall_recovery, PluginRecoveryInfo,
};
pub(crate) use safe::purge_user_plugins_in_safe_profile;
pub(crate) use verify::ensure_preset_plugins;
pub use watch::DshPlugin;
