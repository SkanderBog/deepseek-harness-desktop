use crate::config;
use serde::Serialize;
use std::fs;

use super::path::{get_bin_dir, get_shim_path, path_registered, register_path, unregister_path};
use super::shim::{user_dsh_preserved, write_shims};
use crate::AppHandle;

#[derive(Debug, Clone, Serialize)]
pub struct CliLinkStatus {
    pub enabled: bool,
    pub shim_exists: bool,
    pub path_registered: bool,
    pub user_dsh_preserved: bool,
    pub bin_dir: String,
    pub shim_path: String,
}

pub fn get_status(app_handle: &AppHandle) -> CliLinkStatus {
    let setting = config::get_store_dat_setting(app_handle);
    let shim_path = get_shim_path(app_handle);
    let bin_dir = get_bin_dir(app_handle);
    CliLinkStatus {
        enabled: setting.cli_link_enabled,
        shim_exists: shim_path.is_file(),
        path_registered: path_registered(app_handle),
        user_dsh_preserved: user_dsh_preserved(&bin_dir),
        bin_dir: bin_dir.to_string_lossy().into_owned(),
        shim_path: shim_path.to_string_lossy().into_owned(),
    }
}

pub fn ensure(app_handle: &AppHandle) -> Result<CliLinkStatus, String> {
    let dsh_home = config::get_dsh_data_path(app_handle);
    fs::create_dir_all(&dsh_home).map_err(|e| format!("create dsh home failed: {e}"))?;

    let bin_dir = get_bin_dir(app_handle);
    write_shims(app_handle, &bin_dir)?;
    if cfg!(debug_assertions) {
        log::info!(
            "dsh/pnpm CLI shims ensured at {} (debug build: PATH registration skipped)",
            bin_dir.display()
        );
        return Ok(get_status(app_handle));
    }
    register_path(app_handle)?;

    log::info!("dsh/pnpm CLI links ensured at {}", bin_dir.display());
    Ok(get_status(app_handle))
}

pub fn ensure_shims(app_handle: &AppHandle) -> Result<(), String> {
    let bin_dir = get_bin_dir(app_handle);
    write_shims(app_handle, &bin_dir)?;
    log::info!("dsh/pnpm shims ensured at {}", bin_dir.display());
    Ok(())
}

pub fn remove(app_handle: &AppHandle) -> Result<CliLinkStatus, String> {
    if cfg!(debug_assertions) {
        log::info!("cli link removal skipped in debug build (shared user state kept)");
        return Ok(get_status(app_handle));
    }
    let bin_dir = get_bin_dir(app_handle);

    #[cfg(windows)]
    {
        use super::shim::{
            NPM_SHIM_CMD_NAME, NPM_SHIM_PS1_NAME, NPX_SHIM_CMD_NAME, NPX_SHIM_PS1_NAME,
            PNPM_SHIM_CMD_NAME, PNPM_SHIM_PS1_NAME, SHIM_CMD_NAME, SHIM_PS1_NAME,
        };
        let _ = fs::remove_file(bin_dir.join(SHIM_CMD_NAME));
        let _ = fs::remove_file(bin_dir.join(SHIM_PS1_NAME));
        let _ = fs::remove_file(bin_dir.join(PNPM_SHIM_CMD_NAME));
        let _ = fs::remove_file(bin_dir.join(PNPM_SHIM_PS1_NAME));
        let _ = fs::remove_file(bin_dir.join(NPX_SHIM_CMD_NAME));
        let _ = fs::remove_file(bin_dir.join(NPX_SHIM_PS1_NAME));
        let _ = fs::remove_file(bin_dir.join(NPM_SHIM_CMD_NAME));
        let _ = fs::remove_file(bin_dir.join(NPM_SHIM_PS1_NAME));
    }
    #[cfg(not(windows))]
    {
        use super::shim::{PNPM_SHIM_SH_NAME, SHIM_SH_NAME};
        let _ = fs::remove_file(bin_dir.join(SHIM_SH_NAME));
        let _ = fs::remove_file(bin_dir.join(PNPM_SHIM_SH_NAME));
    }

    unregister_path(app_handle)?;

    log::info!("dsh/pnpm CLI links removed");
    Ok(get_status(app_handle))
}
