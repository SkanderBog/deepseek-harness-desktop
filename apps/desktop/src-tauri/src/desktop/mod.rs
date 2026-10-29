pub mod activation;
pub mod autostart;
pub mod builder;
pub mod deep_link;
pub mod payload;
pub mod window;
#[cfg(unix)]
pub mod login_path;
#[cfg(all(windows, any(not(debug_assertions), test)))]
pub(crate) mod single_instance;
pub mod startup;

pub use builder::{builder, tray};

pub fn product_name<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> &str {
    app.config()
        .product_name
        .as_deref()
        .unwrap_or(&app.package_info().name)
}
