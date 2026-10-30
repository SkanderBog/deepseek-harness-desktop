use crate::AppHandle;
use crate::config;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginError {
    pub message: String,
    pub action: String,
    pub at: String,
}

fn errors_path(app_handle: &AppHandle) -> PathBuf {
    config::get_base_dir(app_handle).join("plugin-errors.json")
}

pub(crate) fn load(app_handle: &AppHandle) -> HashMap<String, PluginError> {
    let Ok(content) = std::fs::read_to_string(errors_path(app_handle)) else {
        return HashMap::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

fn save(app_handle: &AppHandle, map: &HashMap<String, PluginError>) -> Result<(), String> {
    let path = errors_path(app_handle);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("PLUGIN_ERRORS_DIR: {e}"))?;
    }
    let json =
        serde_json::to_string_pretty(map).map_err(|e| format!("PLUGIN_ERRORS_RENDER: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("PLUGIN_ERRORS_WRITE: {e}"))
}

pub fn record(app_handle: &AppHandle, id: &str, action: &str, message: &str) -> Result<(), String> {
    let mut map = load(app_handle);
    map.insert(
        id.to_string(),
        PluginError {
            message: message.trim().to_string(),
            action: action.to_string(),
            at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs().to_string())
                .unwrap_or_default(),
        },
    );
    save(app_handle, &map)
}

pub fn clear(app_handle: &AppHandle, id: &str) -> Result<(), String> {
    let mut map = load(app_handle);
    if map.remove(id).is_some() {
        save(app_handle, &map)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_clear_round_trip() {
        let map = HashMap::from([(
            "dshmarket".to_string(),
            PluginError {
                message: "ERR_PNPM_IGNORED_BUILDS".to_string(),
                action: "install".to_string(),
                at: "1700000000".to_string(),
            },
        )]);
        let json = serde_json::to_string(&map).unwrap();
        let back: HashMap<String, PluginError> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.get("dshmarket").unwrap().action, "install");
        assert_eq!(
            back.get("dshmarket").unwrap().message,
            "ERR_PNPM_IGNORED_BUILDS"
        );
    }
}
