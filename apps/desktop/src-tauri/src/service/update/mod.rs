//!   「自动更新」开关约束（issue #1132），关闭后只保留用户显式发起的检查/安装。

mod about;
mod install;
mod meta;
mod pending;
mod version;

pub use about::{about, DesktopAboutInfo};
#[allow(unused_imports)]
pub use install::{check, download, open_installer, DesktopDownloadProgress, DesktopUpdateInfo};
pub use pending::launch_pending_installer;
use crate::{AppHandle};

pub fn distro_managed() -> bool {
    version::DISTRO_MANAGED
}

/// 与对话框里的「立即更新」），退出时不会拿已下载的安装包替换本应用（issue #1132）。
pub(super) fn auto_update_enabled(app_handle: &AppHandle) -> bool {
    crate::config::get_store_dat_setting(app_handle).auto_update
}

pub fn clear_pending_installer(app_handle: &AppHandle) {
    pending::set(app_handle, None);
}

const REPO_URL: &str = "https://github.com/hairyf/deepseek-harness-desktop";
const COPYRIGHT: &str = "Copyright © 2026 Deepseek Harness Desktop contributors";
const POWERED_BY: &str = "DeepSeek Harness";
const UPDATES_DIR: &str = "updates";
const DOWNLOAD_TIMEOUT_SECS: u64 = 1800;
