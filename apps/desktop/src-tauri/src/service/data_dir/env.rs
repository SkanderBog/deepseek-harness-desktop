
use std::path::PathBuf;

use crate::config::{DSH_HOME_DEV_DIR_NAME, DSH_HOME_DIR_NAME};

pub(super) const DATA_DIR_ENV: &str = "DSH_HOME";

pub(super) fn default_home() -> PathBuf {
    let name = if cfg!(debug_assertions) {
        DSH_HOME_DEV_DIR_NAME
    } else {
        DSH_HOME_DIR_NAME
    };
    crate::config::user_home_dir()
        .map(|home| home.join(name))
        .unwrap_or_else(|| PathBuf::from(name))
}

#[cfg(windows)]
pub(super) fn read_user_home() -> Result<Option<String>, String> {
    Ok(normalize(crate::service::cli::read_user_env(DATA_DIR_ENV)?))
}

#[cfg(windows)]
pub(super) fn restore_process_env(persisted: Result<Option<String>, String>) {
    match persisted {
        Ok(Some(value)) => unsafe {
            std::env::set_var(DATA_DIR_ENV, crate::service::cli::expand_env(&value))
        },
        Ok(None) => {}
        Err(error) => log::warn!("DATA_DIR_ENV_READ: {error}"),
    }
}

#[cfg(not(windows))]
pub(super) fn read_user_home() -> Result<Option<String>, String> {
    Ok(normalize(super::unix_env::read_value()))
}

#[cfg(windows)]
pub(super) fn write_user_home(value: &str) -> Result<(), String> {
    crate::service::cli::write_user_env(DATA_DIR_ENV, value)?;
    crate::service::cli::notify_environment_change();
    Ok(())
}

#[cfg(not(windows))]
pub(super) fn write_user_home(value: &str) -> Result<(), String> {
    super::unix_env::write_value(value)
}

#[cfg(windows)]
pub(super) fn clear_user_home() -> Result<(), String> {
    crate::service::cli::delete_user_env(DATA_DIR_ENV)?;
    crate::service::cli::notify_environment_change();
    Ok(())
}

#[cfg(not(windows))]
pub(super) fn clear_user_home() -> Result<(), String> {
    super::unix_env::clear_value()
}

#[cfg(windows)]
pub(super) fn now_stamp() -> String {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;

    let mut time: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut time) };
    format!(
        "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}",
        time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute, time.wSecond
    )
}

#[cfg(not(windows))]
pub(super) fn now_stamp() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as libc::time_t)
        .unwrap_or_default();
    let mut local: libc::tm = unsafe { std::mem::zeroed() };
    let filled = unsafe {
        libc::localtime_r(
            &seconds as *const libc::time_t,
            &mut local as *mut libc::tm,
        )
    };
    if !filled.is_null() {
        return format!(
            "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}",
            local.tm_year + 1900,
            local.tm_mon + 1,
            local.tm_mday,
            local.tm_hour,
            local.tm_min,
            local.tm_sec
        );
    }
    let utc = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}",
        utc.year(),
        u8::from(utc.month()),
        utc.day(),
        utc.hour(),
        utc.minute(),
        utc.second()
    )
}

#[cfg(windows)]
pub(super) fn free_bytes(path: &std::path::Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let mut probe = path.to_path_buf();
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }
    let wide: Vec<u16> = probe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free = 0u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut free,
        )
    };
    (ok != 0).then_some(free)
}

#[cfg(not(windows))]
pub(super) fn free_bytes(path: &std::path::Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let mut probe = path.to_path_buf();
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }
    let c_path = CString::new(probe.as_os_str().as_bytes()).ok()?;
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stats) } != 0 {
        return None;
    }
    Some((stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64))
}

fn normalize(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn persisted_home_restores_in_an_isolated_process() {
        use std::os::windows::process::CommandExt;
        for mode in [
            "missing", "stale", "expanded", "unset", "empty", "blank", "default", "error",
        ] {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "service::data_dir::env::tests::persisted_home_restore_child",
                    "--nocapture",
                ])
                .env("DSH_TEST_HOME_RESTORE", mode)
                .env_remove(DATA_DIR_ENV)
                .creation_flags(0x08000000)
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success(), "home restoration failed: {mode}");
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("home restoration timed out: {mode}");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn persisted_home_restore_child() {
        let Ok(mode) = std::env::var("DSH_TEST_HOME_RESTORE") else {
            return;
        };
        let migrated = "E:\\DSH 数据\\DSHHome";
        let inherited = "D:\\Old Home";
        unsafe { std::env::set_var("DSH_TEST_EXPAND_ROOT", "E:\\DSH 数据"); }
        let persisted = match mode.as_str() {
            "missing" | "stale" => Ok(Some(migrated.to_string())),
            "expanded" => Ok(Some("%DSH_TEST_EXPAND_ROOT%\\DSHHome".to_string())),
            "empty" => Ok(normalize(Some(String::new()))),
            "blank" => Ok(normalize(Some("   ".to_string()))),
            "error" => Err("REG_QUERY_FAILED: isolated failure".to_string()),
            "unset" | "default" => Ok(None),
            _ => panic!("unknown home restoration mode: {mode}"),
        };
        if mode == "expanded" {
            unsafe { std::env::set_var(DATA_DIR_ENV, migrated); }
        } else if mode != "missing" && mode != "default" {
            unsafe { std::env::set_var(DATA_DIR_ENV, inherited); }
        }
        restore_process_env(persisted);
        let expected = match mode.as_str() {
            "missing" | "stale" | "expanded" => Some(migrated.into()),
            "default" => None,
            _ => Some(inherited.into()),
        };
        assert_eq!(std::env::var_os(DATA_DIR_ENV), expected, "mode: {mode}");
    }

    #[test]
    fn normalize_treats_blank_as_unset() {
        assert_eq!(normalize(Some("  ".to_string())), None);
        assert_eq!(normalize(Some(String::new())), None);
        assert_eq!(normalize(None), None);
        assert_eq!(
            normalize(Some(" D:\\DSHHome ".to_string())),
            Some("D:\\DSHHome".to_string())
        );
    }

    #[test]
    fn default_home_matches_the_build_mode() {
        let home = default_home();
        let expected = if cfg!(debug_assertions) {
            DSH_HOME_DEV_DIR_NAME
        } else {
            DSH_HOME_DIR_NAME
        };
        assert_eq!(home.file_name().unwrap().to_str().unwrap(), expected);
    }

    #[test]
    fn stamp_has_the_script_compatible_shape() {
        let stamp = now_stamp();
        assert_eq!(stamp.len(), 19);
        assert!(stamp
            .chars()
            .all(|c| c.is_ascii_digit() || c == 'T' || c == '-'));
        assert_eq!(&stamp[10..11], "T");
    }

    #[test]
    fn free_bytes_probes_the_nearest_existing_ancestor() {
        let missing = std::env::temp_dir()
            .join("dsh-data-dir-not-created")
            .join("deep");
        let free = free_bytes(&missing);
        assert!(free.is_some_and(|bytes| bytes > 0));
        assert_eq!(free_bytes(std::path::Path::new("relative-only")), None);
    }
}
