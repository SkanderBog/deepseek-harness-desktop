#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    #[cfg(unix)]
    if args.get(1).is_some_and(|arg| arg == "--print-login-path") {
        if let Err(error) = main::desktop::login_path::print() {
            eprintln!("STARTUP_PATH_RESTORE_FAILED: {error}");
            std::process::exit(1);
        }
        return;
    }
    if args.get(1).is_some_and(|arg| arg == "--patch-core") {
        let Some(dir) = args.get(2) else {
            eprintln!("usage: deepseek-harness-desktop --patch-core <core-dir>");
            std::process::exit(2);
        };
        if let Err(e) = main::patch_core_dir(std::path::Path::new(dir)) {
            eprintln!("patch-core failed: {e}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(error) = main::run() {
        main::desktop::startup::report(&error);
        std::process::exit(1);
    }
}
