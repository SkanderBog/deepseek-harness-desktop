mod bridge;
mod config;
pub mod desktop;
mod logger;
mod service;
mod utils;

#[cfg(target_os = "macos")]
use tauri::Manager;

pub type AppHandle<R = tauri_runtime_electron::ElectronRuntime> = tauri::AppHandle<R>;
pub type Webview<R = tauri_runtime_electron::ElectronRuntime> = tauri::Webview<R>;
pub type WebviewWindow<R = tauri_runtime_electron::ElectronRuntime> = tauri::WebviewWindow<R>;

pub fn patch_core_dir(core_dir: &std::path::Path) -> Result<(), String> {
    service::patch::apply_all_at(core_dir)
}

pub fn run() -> Result<(), String> {
    let started = std::time::Instant::now();
    #[cfg(unix)]
    if let Err(error) = std::env::current_exe()
        .map_err(|error| error.to_string())
        .and_then(|executable| desktop::login_path::restore(&executable))
    {
        eprintln!("STARTUP_PATH_RESTORE_FAILED: {error}");
    }
    desktop::startup::verify()?;
    logger::init();
    log::info!(
        "STARTUP_BOOT: name=logger_ready elapsed_ms={}",
        started.elapsed().as_millis()
    );
    service::data_dir::restore_process_env();

    let mut context = tauri::generate_context!();
    if let Some(directory) = config::e2e_app_data_dir()? {
        context.config_mut().app.app_directories_override = Some(
            tauri::utils::config::AppDirectoriesOverride::Root(directory),
        );
    }
    log::info!(
        "STARTUP_BOOT: name=context_ready elapsed_ms={}",
        started.elapsed().as_millis()
    );
    #[cfg(all(windows, not(debug_assertions)))]
    let single_instance = match desktop::single_instance::claim(&context.config().identifier)? {
        Some(instance) => std::sync::Arc::new(instance),
        None => return Ok(()),
    };
    let startup = service::workflow::early::Startup::begin(
        &context,
        #[cfg(all(windows, not(debug_assertions)))]
        single_instance.clone(),
    );
    desktop::builder()
        .plugin(tauri_runtime_electron::init())
        .invoke_handler(desktop::handler())
        .build(context)
        .map_err(|error| format!("STARTUP_FAILED: {error}"))?
        .run(move |app_handle, event| match event {
            tauri::RunEvent::Ready => {
                startup.attach(app_handle);
                #[cfg(all(windows, not(debug_assertions)))]
                if let Err(error) = single_instance.attach(app_handle) {
                    log::error!("SINGLE_INSTANCE_ATTACH_FAILED: {error}");
                    app_handle.exit(1);
                }
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::WindowEvent { event, .. } => {
                desktop::builder::on_macos_titlebar_event(&event);
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::MainEventsCleared => {
                desktop::builder::sync_macos_titlebars(|| {
                    app_handle.webview_windows().into_values()
                });
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                crate::utils::show_main_window(&app_handle);
            }
            tauri::RunEvent::ExitRequested { .. } => {
                config::save_main_window_geometry(app_handle);
            }
            tauri::RunEvent::Exit => {
                startup.shutdown();
                service::plugin::terminate_active_installs_blocking();
                let setting = config::get_store_dat_setting(app_handle);
                if setting.installed {
                    service::workflow::stop_on_exit(app_handle);
                }
                service::update::launch_pending_installer(app_handle);
            }
            _ => {}
        });
    Ok(())
}
