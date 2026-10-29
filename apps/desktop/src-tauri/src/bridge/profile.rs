
use crate::config;
use crate::service::profile;
use crate::service::profile::migrate::{MigrateReport, MigrationAnalysis, MigrationDataKind};
use crate::{AppHandle, WebviewWindow};

#[tauri::command]
pub fn get_profiles(window: WebviewWindow, app_handle: AppHandle) -> Vec<profile::Profile> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope_sync(profile, || profile::list(&app_handle))
}

#[tauri::command]
pub fn create_profile(app_handle: AppHandle, name: String) -> Result<profile::Profile, String> {
    profile::create(&app_handle, &name)
}

#[tauri::command]
pub async fn set_active_profile(
    app_handle: AppHandle,
    window: WebviewWindow,
    id: String,
) -> Result<profile::Profile, String> {
    let _transition = crate::service::workflow::acquire_core_transition().await?;
    let old = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    let mut selected = if window.label() == crate::desktop::builder::MAIN_WINDOW_LABEL {
        profile::set_active(&app_handle, &id)?
    } else {
        profile::select(&app_handle, &id)?
    };
    selected.active = true;
    crate::service::workflow::instances::bind_window(window.label(), &id);
    if old != id && !crate::service::workflow::instances::is_attached(&old) {
        crate::service::workflow::instances::scope(old, crate::service::workflow::stop(app_handle))
            .await?;
    }
    Ok(selected)
}

#[tauri::command]
pub async fn remove_profile(
    window: WebviewWindow,
    app_handle: AppHandle,
    id: String,
) -> Result<(), String> {
    crate::service::fs_guard::validate_id(&id)?;
    if id == profile::DEFAULT_PROFILE {
        return Err("PROFILE_DEFAULT_NOT_REMOVABLE: the default profile cannot be removed".into());
    }
    let _transition = crate::service::workflow::acquire_core_transition().await?;
    let current = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    if id == current || crate::service::workflow::instances::is_attached(&id) {
        return Err("PROFILE_ACTIVE_NOT_REMOVABLE: the active profile cannot be removed".into());
    }
    crate::service::workflow::instances::scope(id.clone(), async {
        crate::service::workflow::stop(app_handle.clone()).await
    })
    .await?;
    crate::service::workflow::instances::scope_sync(current, || profile::remove(&app_handle, &id))
}

#[tauri::command]
pub async fn reset_profile(
    app_handle: AppHandle,
    window: WebviewWindow,
    id: String,
) -> Result<(), String> {
    crate::service::fs_guard::validate_id(&id)?;
    let _transition = crate::service::workflow::acquire_core_transition().await?;
    if crate::service::workflow::instances::attached_elsewhere(&id, window.label()) {
        return Err("PROFILE_IN_USE: stop and detach profile before reset".into());
    }
    crate::service::workflow::instances::scope(id.clone(), async {
        crate::service::workflow::stop(app_handle.clone()).await?;
        profile::reset(&app_handle, &id)
    })
    .await
}

#[tauri::command]
pub async fn clone_profile(
    app_handle: AppHandle,
    source_id: String,
    name: Option<String>,
) -> Result<profile::Profile, String> {
    let profiles_root = config::get_dsh_data_path(&app_handle).join("profiles");
    tauri::async_runtime::spawn_blocking(move || {
        profile::clone_with_root(&profiles_root, &source_id, name.as_deref())
    })
    .await
    .map_err(|e| format!("PROFILE_CLONE_TASK: {e}"))?
}

#[tauri::command]
pub async fn analyze_profile_migration(
    window: WebviewWindow,
    app_handle: AppHandle,
    source_id: String,
) -> Result<MigrationAnalysis, String> {
    let profile = crate::service::workflow::instances::window_profile(&app_handle, window.label());
    crate::service::workflow::instances::scope(profile, async move {
        profile::migrate::analyze(&app_handle, &source_id).await
    })
    .await
}

#[tauri::command]
pub async fn migrate_profile_data(
    window: WebviewWindow,
    app_handle: AppHandle,
    source_id: String,
    items: Vec<MigrationDataKind>,
) -> Result<MigrateReport, String> {
    let handle = app_handle.clone();
    let profile_id =
        crate::service::workflow::instances::window_profile(&app_handle, window.label());
    tauri::async_runtime::spawn_blocking(move || {
        crate::service::workflow::instances::scope_sync(profile_id, || {
            profile::migrate::apply(&handle, &source_id, &items)
        })
    })
    .await
    .map_err(|e| format!("PROFILE_MIGRATION_TASK: {e}"))?
}

#[cfg(test)]
mod tests {
    #[test]
    fn profile_removal_waits_for_stop_before_deleting_detached_data() {
        let source = include_str!("profile.rs");
        let removal = source
            .split("pub async fn remove_profile(")
            .nth(1)
            .unwrap()
            .split("pub async fn reset_profile(")
            .next()
            .unwrap();
        let lock = removal.find("acquire_core_transition().await?").unwrap();
        let attached = removal.find("instances::is_attached(&id)").unwrap();
        let scope = removal.find("instances::scope(id.clone(),").unwrap();
        let stop = removal
            .find("workflow::stop(app_handle.clone()).await")
            .unwrap();
        let confirmed = stop + removal[stop..].find(".await?;").unwrap();
        let delete = removal.find("profile::remove(&app_handle, &id)").unwrap();
        assert!(lock < attached && attached < scope && scope < stop);
        assert!(stop < confirmed && confirmed < delete);
    }

    #[test]
    fn profile_switch_persists_the_startup_profile_only_from_the_main_window() {
        let source = include_str!("profile.rs");
        let switch = source
            .split("pub async fn set_active_profile(")
            .nth(1)
            .unwrap()
            .split("pub async fn remove_profile(")
            .next()
            .unwrap();
        let guard = switch
            .find("window.label() == crate::desktop::builder::MAIN_WINDOW_LABEL")
            .unwrap();
        let persist = switch
            .find("profile::set_active(&app_handle, &id)")
            .unwrap();
        let transient = switch.find("profile::select(&app_handle, &id)").unwrap();
        assert!(
            guard < persist && guard < transient,
            "切换档案必须先按窗口区分：只有主窗口持久化下次启动档案"
        );
    }

    #[test]
    fn profile_binding_entrypoints_share_the_destructive_transition_lock() {
        for (source, entry, end) in [
            (
                include_str!("../desktop/window.rs"),
                "pub async fn create_app_window(",
                "pub fn quit_app(",
            ),
            (
                include_str!("remote.rs"),
                "pub async fn remote_open_window(",
                "#[cfg(test)]",
            ),
            (
                include_str!("lifecycle.rs"),
                "pub async fn enter_safe_mode(",
                "pub async fn quarantine_broken_patch_layers(",
            ),
            (
                include_str!("profile.rs"),
                "pub async fn set_active_profile(",
                "pub async fn remove_profile(",
            ),
        ] {
            let body = source
                .split(entry)
                .nth(1)
                .unwrap()
                .split(end)
                .next()
                .unwrap();
            let lock = body.find("acquire_core_transition().await?").unwrap();
            let binding = body
                .find("instances::bind_window(")
                .or_else(|| body.find("build_extra_window("))
                .unwrap();
            assert!(
                lock < binding,
                "{entry} must serialize bindings with reset/restore/removal"
            );
        }
    }
}
