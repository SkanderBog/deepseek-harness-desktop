use std::time::Duration;

pub const NODE_VERSION: &str = "v22.22.0";

pub const NODE_BASE_URL: &str = "https://nodejs.org/dist/";

pub const NODE_MIRROR_BASE_URL: &str = "https://npmmirror.com/mirrors/node/";

pub const DSH_CORE_URL: &str =
    "https://github.com/dsh-tauri-desk/deepseek-harness-pkg/releases/latest/download/";

pub const DSH_MIRROR_PREFIXES: &[&str] = &[
    "https://gh-proxy.com/",
    "https://gh.llkk.cc/",
    "https://ghfast.top/",
    "https://ghproxy.net/",
];

pub const PNPM_VERSION: &str = "11.7.0";
pub const PNPM_SHA256: &str = "deafa7ec98a1218b6a047289b92fbe2395c1e22d3495bb711653013218ee15ee";

pub const PNPM_BASE_URL: &str = "https://registry.npmjs.org/pnpm/-/";

#[cfg_attr(all(not(windows), not(test)), allow(dead_code))] // 仅 Windows 的 MinGit 任务与单测使用
pub const MINGIT_VERSION: &str = "2.53.0.2";
#[cfg_attr(all(not(windows), not(test)), allow(dead_code))] // 仅 Windows 的 MinGit 任务与单测使用
pub const MINGIT_X64_SHA256: &str =
    "d4bf83d6a860ccae9af44e508e1e00a39f09db6fa78a9ba5543b94d87ca22a29";
#[cfg_attr(all(not(windows), not(test)), allow(dead_code))] // 仅 Windows 的 MinGit 任务与单测使用
pub const MINGIT_ARM64_SHA256: &str =
    "842d50edc6bbcf39693e60a8ebb9dabb89b96b932b63aae12d218522b3e497f3";
#[cfg_attr(all(not(windows), not(test)), allow(dead_code))] // 仅 Windows 的 MinGit 任务与单测使用
pub const MINGIT_BASE_URL: &str =
    "https://github.com/git-for-windows/git/releases/download/v2.53.0.windows.2/";

pub const PNPM_MIRROR_BASE_URL: &str = "https://registry.npmmirror.com/pnpm/-/";

pub const DSH_HOST: &str = "http://127.0.0.1";

pub fn get_dsh_service_url(port: u16) -> String {
    format!("{DSH_HOST}:{port}")
}

pub const DSH_PORT: u16 = 3080;
pub const DSH_DEV_PORT: u16 = 3081;

pub const DSH_HOME_DIR_NAME: &str = ".dsh";
pub const DSH_HOME_DEV_DIR_NAME: &str = ".dsh.dev";

pub const APP_IDENTIFIER: &str = env!("DSH_APP_IDENTIFIER");

pub const HARNESS_PID_MARKER_NAMES: [&str; 2] = [".harness.pid", ".harness-nightly.pid"];

pub const APP_DATA_DEV_DIR_NAME: &str = "dev";

pub const DSH_MANIFEST_RELATIVE: &str = "package.json";

#[cfg_attr(not(windows), allow(dead_code))] // 仅 Windows 的 bin 目录计算使用
pub const CLI_ROOT_DEV_DIR_NAME: &str = "dev-dsh";

pub const STORE_DAT_FILE: &str = ".store.dat";
pub const STORE_DAT_DEV_FILE: &str = ".store.dev.dat";
pub const STORE_DAT_TEST_FILE: &str = ".store.test.dat";
pub const E2E_PORT_ENV_VAR: &str = "TAURI_WEBDRIVER_PORT";

pub const DOWNLOAD_CACHE_ENV_VAR: &str = "DSH_DOWNLOAD_CACHE_DIR";

#[cfg(target_os = "windows")]
pub const E2E_WEBVIEW_DATA_DIR_ENV_VAR: &str = "DSH_E2E_WEBVIEW_DATA_DIR";
pub const STORE_SETTING_KEY: &str = "setting";
pub const STORE_WINDOW_STATE_KEY: &str = "window_state";
pub const STORE_PET_WINDOW_STATE_KEY: &str = "pet_window_state";
pub const STORE_PENDING_INSTALLER_KEY: &str = "desktop_pending_installer";

pub const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

pub const LOOPBACK_CONNECT_TIMEOUT: Duration = Duration::from_millis(250);

pub const SLOW_STARTUP_THRESHOLD: Duration = Duration::from_secs(15);

pub const PROXY_TEST_URL: &str = "https://github.com/hairyf/deepseek-harness-desktop/releases.atom";

pub const PROXY_TEST_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests {
    use super::get_dsh_service_url;

    #[test]
    fn service_url_preserves_loopback_origin_and_entire_port_range() {
        for (port, expected) in [
            (0, "http://127.0.0.1:0"),
            (3080, "http://127.0.0.1:3080"),
            (3081, "http://127.0.0.1:3081"),
            (65535, "http://127.0.0.1:65535"),
        ] {
            assert_eq!(get_dsh_service_url(port), expected);
        }
    }
}
