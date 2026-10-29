#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(unix)]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--print-login-path")) {
        if let Err(error) = tauri_app_lib::desktop::login_path::print() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(error) = tauri_app_lib::run() {
        tauri_app_lib::desktop::startup::report(&error);
        std::process::exit(1);
    }
}
