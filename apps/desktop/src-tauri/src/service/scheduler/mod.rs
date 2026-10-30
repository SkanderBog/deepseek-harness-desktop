use crate::AppHandle;
use crate::service::workflow::{
    status::{self, Status},
    utils,
};
use std::time::Duration;
use tokio::time;

pub fn start(app_handle: &AppHandle) {
    log::info!("Starting dsh process monitor");
    let app_handle_clone = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        scheduler_permanent_loop(app_handle_clone).await;
    });
}

async fn scheduler_permanent_loop(app_handle: AppHandle) {
    let mut interval = time::interval(Duration::from_secs(5));

    loop {
        for profile in crate::service::workflow::instances::profiles() {
            crate::service::workflow::instances::scope(profile, async {
                let current_status = status::get_status();
                let port = crate::service::workflow::instances::port(&app_handle);
                let is_dsh_running = crate::service::workflow::has_owned_process()
                    && utils::is_dsh_running(port).await;
                log::trace!("DSH status check: dsh_running={}", is_dsh_running);
                if is_dsh_running && current_status != Status::Running {
                    status::set_status(Status::Running);
                    status::emit_status(&app_handle);
                }
                if !crate::service::workflow::has_owned_process()
                    && current_status == Status::Running
                {
                    log::warn!(
                "DSH status check: no owned process yet status Running; resetting to Stopped"
            );
                    status::set_status(Status::Stopped);
                    status::emit_status(&app_handle);
                }

                crate::service::plugin::watch::check_and_emit(&app_handle);
            })
            .await;
        }
        crate::config::check_and_emit_theme(&app_handle);
        interval.tick().await;
    }
}
