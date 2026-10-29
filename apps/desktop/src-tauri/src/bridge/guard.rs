
use std::path::{Path, PathBuf};

use tauri::{Manager};
use crate::{AppHandle};

pub fn allowed_roots(app_handle: &AppHandle) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(download_dir) = app_handle.path().download_dir() {
        roots.push(download_dir);
    }
    if let Ok(data_dir) = app_handle.path().app_data_dir() {
        roots.push(data_dir);
    }
    let dsh_home = crate::config::get_dsh_data_path(app_handle);
    roots.push(dsh_home);
    roots.push(crate::config::get_base_dir(app_handle).join("recovery-backups"));
    if let Some(dir) = crate::service::core::local_core_package_dir(app_handle) {
        roots.push(dir);
    }
    for key in [
        crate::config::dependencies::DEP_PNPM,
        crate::config::dependencies::DEP_DSH,
    ] {
        roots.push(crate::config::dependencies::active_root(app_handle, key));
    }
    roots
}

fn existing_roots(app_handle: &AppHandle) -> Vec<PathBuf> {
    allowed_roots(app_handle)
        .into_iter()
        .filter(|root| root.is_dir())
        .collect()
}

pub fn is_allowed_path(app_handle: &AppHandle, path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    let Ok(real) = dunce::canonicalize(path) else {
        return false;
    };
    existing_roots(app_handle)
        .iter()
        .filter_map(|root| dunce::canonicalize(root).ok())
        .any(|root| real.starts_with(&root))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[test]
    fn prefix_match_respects_component_boundary() {
        let inside = PathBuf::from("/data/app/dependencies/dsh");
        let root = PathBuf::from("/data/app");
        assert!(inside.starts_with(&root));
        let sibling = PathBuf::from("/data/app2/x");
        assert!(!sibling.starts_with(&root));
    }
}
