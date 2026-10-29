
use std::path::Path;

use serde::{Deserialize, Serialize};
use tauri_plugin_store::StoreExt;

use crate::config::{store_dat_file_name, STORE_PENDING_INSTALLER_KEY};
use crate::service::workflow;
use crate::{AppHandle};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingInstaller {
    path: String,
    version: String,
}

fn get(app_handle: &AppHandle) -> Option<PendingInstaller> {
    let store = app_handle.store(store_dat_file_name()).ok()?;
    let value = store.get(STORE_PENDING_INSTALLER_KEY)?;
    let pending: PendingInstaller = serde_json::from_value(value).ok()?;
    if pending.path.is_empty() {
        return None;
    }
    Some(pending)
}

pub(super) fn installer_path(app_handle: &AppHandle) -> Option<std::path::PathBuf> {
    get(app_handle).map(|pending| pending.path.into())
}

pub(super) fn set(app_handle: &AppHandle, pending: Option<(&Path, &str)>) {
    let Ok(store) = app_handle.store(store_dat_file_name()) else {
        log::warn!("Failed to load store for pending installer marker");
        return;
    };
    match pending {
        Some((path, version)) => {
            let value = serde_json::json!({
                "path": path.to_string_lossy(),
                "version": version,
            });
            store.set(STORE_PENDING_INSTALLER_KEY, value);
        }
        None => {
            store.delete(STORE_PENDING_INSTALLER_KEY);
        }
    }
    if let Err(error) = store.save() {
        log::warn!("Failed to save pending installer marker: {error}");
    }
}

pub fn launch_pending_installer(app_handle: &AppHandle) {
    if !super::version::supports_stable_updates(&app_handle.config().identifier)
        || !super::auto_update_enabled(app_handle)
    {
        set(app_handle, None);
        return;
    }
    let Some(pending) = get(app_handle) else {
        return;
    };
    set(app_handle, None);

    let current = super::version::current_version();
    if !super::version::is_newer(&pending.version, &current) {
        log::info!(
            "Pending installer {} is not newer than running {current}, skipping auto update",
            pending.version
        );
        return;
    }

    log::info!(
        "Launching pending desktop installer on exit: {} ({})",
        pending.path,
        pending.version
    );
    if let Err(error) = workflow::stop_for_installer(app_handle) {
        log::error!("UPDATE_STOP_FAILED: refusing installer handoff: {error}");
        return;
    }
    if let Err(error) = super::install::open_installer_now(app_handle, &pending.path) {
        log::warn!("UPDATE_OPEN: failed to launch pending installer on exit: {error}");
    }
}
