use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, Runtime};

pub fn allowed_roots<R: Runtime>(app_handle: &AppHandle<R>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(download_dir) = app_handle.path().download_dir() {
        roots.push(download_dir);
    }
    if let Ok(data_dir) = app_handle.path().app_data_dir() {
        roots.push(data_dir);
    }
    roots
}

fn existing_roots<R: Runtime>(app_handle: &AppHandle<R>) -> Vec<PathBuf> {
    allowed_roots(app_handle)
        .into_iter()
        .filter(|root| root.is_dir())
        .collect()
}

pub fn is_allowed_path<R: Runtime>(app_handle: &AppHandle<R>, path: &Path) -> bool {
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
