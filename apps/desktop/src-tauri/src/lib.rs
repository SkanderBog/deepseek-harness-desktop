mod bridge;
mod config;
pub mod desktop;
mod logger;
mod utils;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> Result<(), String> {
    #[cfg(unix)]
    if let Err(error) = std::env::current_exe()
        .map_err(|error| error.to_string())
        .and_then(|executable| desktop::login_path::restore(&executable))
    {
        eprintln!("STARTUP_PATH_RESTORE_FAILED: {error}");
    }
    desktop::startup::verify()?;
    logger::init();
    let mut context = tauri::generate_context!();
    if let Some(directory) = config::e2e_app_data_dir()? {
        context.config_mut().app.app_directories_override = Some(
            tauri::utils::config::AppDirectoriesOverride::Root(directory),
        );
    }
    for window in &mut context.config_mut().app.windows {
        window.create = false;
    }
    #[cfg(all(windows, not(debug_assertions)))]
    let single_instance = match desktop::single_instance::claim(&context.config().identifier)? {
        Some(instance) => std::sync::Arc::new(instance),
        None => return Ok(()),
    };
    desktop::builder()
        .invoke_handler(tauri::generate_handler![
            bridge::log_frontend,
            bridge::read_clipboard_text,
            bridge::write_clipboard_text,
            bridge::open_external_url,
            bridge::reveal_in_folder,
            bridge::open_dir,
            bridge::get_app_config,
            bridge::update_app_config,
            bridge::set_language,
            bridge::is_dev_build,
            desktop::window::toggle_devtools,
            bridge::get_launch_on_login,
            bridge::set_launch_on_login,
        ])
        .build(context)
        .map_err(|error| error.to_string())?
        .run(move |app, event| {
            #[cfg(all(windows, not(debug_assertions)))]
            if matches!(event, tauri::RunEvent::Ready) {
                if let Err(error) = single_instance.attach(app) {
                    log::error!("{error}");
                    app.exit(1);
                }
            }
            if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
                config::save_main_window_geometry(app);
            }
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                utils::show_main_window(app);
            }
        });
    Ok(())
}
