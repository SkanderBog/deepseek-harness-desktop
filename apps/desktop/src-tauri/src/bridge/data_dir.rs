//! 数据目录（`DSH_HOME`）迁移 / 回滚的 Tauri 命令（issue #871）。

use std::path::{Path, PathBuf};

use crate::service::data_dir;
use crate::{AppHandle, WebviewWindow};

fn reject_unreadable_markers(
    recorded: impl IntoIterator<Item = data_dir::HarnessMarker>,
) -> Result<(), String> {
    use data_dir::HarnessMarker;
    recorded
        .into_iter()
        .enumerate()
        .try_for_each(|(index, marker)| match marker {
            HarnessMarker::Invalid => Err(format!(
                "DATA_DIR_HARNESS_MARKER_INVALID: {} 读不出来，无法确认 Harness 是否已退出，请先退出所有实例再重试",
                crate::config::HARNESS_PID_MARKER_NAMES[index % 2]
            )),
            HarnessMarker::Missing | HarnessMarker::Pid(_) => Ok(()),
        })
}

fn confirm_harness_stopped(
    recorded: impl IntoIterator<Item = data_dir::HarnessMarker>,
) -> Result<(), String> {
    let recorded: Vec<_> = recorded.into_iter().collect();
    reject_unreadable_markers(recorded.clone())?;
    for marker in recorded {
        if let data_dir::HarnessMarker::Pid(pid) = marker {
            if data_dir::process_alive(pid) {
                return Err(format!(
                    "DATA_DIR_HARNESS_RUNNING: {pid} 仍在使用数据目录，请先退出那个实例再重试"
                ));
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn get_data_dir_status(app_handle: AppHandle) -> data_dir::DataDirStatus {
    data_dir::status(&app_handle)
}

#[tauri::command]
pub async fn pick_data_dir(app_handle: AppHandle) -> Result<Option<String>, String> {
    let start = crate::config::get_dsh_data_path(&app_handle);
    let picked = rfd::AsyncFileDialog::new()
        .set_title("选择数据存放目录")
        .set_directory(start.parent().unwrap_or(&start))
        .pick_folder()
        .await;
    Ok(picked.map(|handle| handle.path().to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn list_data_dir_entries(
    app_handle: AppHandle,
) -> Result<Vec<data_dir::DataDirEntry>, String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || data_dir::entries(&app))
        .await
        .map_err(|e| format!("DATA_DIR_TASK: {e}"))?
}

#[tauri::command]
pub async fn preview_data_dir_migration(
    app_handle: AppHandle,
    parent: String,
    leaf: String,
) -> Result<data_dir::MigrationPlan, String> {
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || data_dir::preview(&app, Path::new(&parent), &leaf))
        .await
        .map_err(|e| format!("DATA_DIR_TASK: {e}"))?
}

#[tauri::command]
pub async fn migrate_data_dir(
    window: WebviewWindow,
    app_handle: AppHandle,
    parent: String,
    leaf: String,
) -> Result<data_dir::MigrationOutcome, String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {
        let transition = crate::service::workflow::acquire_core_transition().await?;
        crate::service::workflow::instances::require_exclusive_profile()?;
        let operation = crate::service::plugin::acquire_operation_lock().await;
        let recorded =
            data_dir::all_harness_markers(&crate::config::get_dsh_data_path(&app_handle));
        reject_unreadable_markers(recorded.clone())?;
        crate::service::workflow::stop(app_handle.clone()).await?;
        confirm_harness_stopped(recorded)?;
        let app = app_handle.clone();
        let result = data_dir::migrate(app, PathBuf::from(parent), leaf).await;
        drop(transition);
        drop(operation);
        result
    })
    .await
}

#[tauri::command]
pub async fn rollback_data_dir(
    window: WebviewWindow,
    app_handle: AppHandle,
    backup: Option<String>,
) -> Result<data_dir::MigrationOutcome, String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {
        let transition = crate::service::workflow::acquire_core_transition().await?;
        crate::service::workflow::instances::require_exclusive_profile()?;
        let operation = crate::service::plugin::acquire_operation_lock().await;
        let recorded =
            data_dir::all_harness_markers(&crate::config::get_dsh_data_path(&app_handle));
        reject_unreadable_markers(recorded.clone())?;
        crate::service::workflow::stop(app_handle.clone()).await?;
        confirm_harness_stopped(recorded)?;
        let app = app_handle.clone();
        let result = data_dir::rollback(app, backup.map(PathBuf::from)).await;
        drop(transition);
        drop(operation);
        result
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use data_dir::HarnessMarker::{Invalid, Missing, Pid};

    #[test]
    fn recorded_markers_prevent_live_channel_migration_after_marker_removal() {
        assert!(confirm_harness_stopped([Missing; 2]).is_ok());
        assert!(confirm_harness_stopped([Pid(u32::MAX); 2]).is_ok());
        for (index, name) in [".harness.pid", ".harness-nightly.pid"]
            .into_iter()
            .enumerate()
        {
            let mut recorded = [Missing; 2];
            recorded[index] = Pid(std::process::id());
            assert!(reject_unreadable_markers(recorded).is_ok());
            assert!(confirm_harness_stopped(recorded)
                .unwrap_err()
                .starts_with("DATA_DIR_HARNESS_RUNNING:"));
            recorded[index] = Invalid;
            let error = reject_unreadable_markers(recorded).unwrap_err();
            assert!(error.starts_with("DATA_DIR_HARNESS_MARKER_INVALID:"));
            assert!(error.contains(name));
            assert!(confirm_harness_stopped(recorded).is_err());
        }
    }
}
