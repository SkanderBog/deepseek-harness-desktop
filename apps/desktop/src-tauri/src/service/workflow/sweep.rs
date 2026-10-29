//! 孤儿 Harness 清扫：崩溃/强杀残留实例的识别与回收（issue #34 关联现象），

use crate::config;
use std::fs;
use std::process::Command;

use super::process::{
    has_owned_process, kill_pid_tree, take_startup_sweep, terminate_stale_harness_processes,
};
use super::utils::is_port_in_use;
use crate::{AppHandle};

pub(super) fn harness_pid_path<R: tauri::Runtime>(
    app_handle: &AppHandle<R>,
) -> std::path::PathBuf {
    let channel = usize::from(app_handle.config().identifier == "dsh-tauri-nightly");
    let root = config::get_dsh_data_path(app_handle);
    let name = config::HARNESS_PID_MARKER_NAMES[channel];
    match super::instances::current_profile() {
        Some(profile) => root.join("profiles").join(profile).join(name),
        None => root.join(name),
    }
}

#[cfg(test)]
mod tests {
    use super::harness_pid_path;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    #[test]
    fn harness_markers_are_isolated_by_profile() {
        let app = mock_builder().build(mock_context(noop_assets())).unwrap();
        let a = super::super::instances::scope_sync("marker-profile-a".into(), || {
            harness_pid_path(app.handle())
        });
        let b = super::super::instances::scope_sync("marker-profile-b".into(), || {
            harness_pid_path(app.handle())
        });
        assert_ne!(a, b);
        assert_eq!(a.parent().unwrap().file_name().unwrap(), "marker-profile-a");
        assert_eq!(a.file_name(), b.file_name());
    }

    #[test]
    fn harness_pid_marker_is_isolated_by_channel() {
        for (identifier, marker) in [
            ("dsh-tauri", ".harness.pid"),
            ("dsh-tauri-nightly", ".harness-nightly.pid"),
        ] {
            let mut context = mock_context(noop_assets());
            context.config_mut().identifier = identifier.into();
            let app = mock_builder().build(context).unwrap();
            let path = harness_pid_path(app.handle());
            assert_eq!(path.file_name().unwrap(), marker);
        }
    }
}

pub fn sweep_orphan_harness(app_handle: &AppHandle) {
    if super::instances::current_profile().is_none() {
        let root = config::get_dsh_data_path(app_handle).join("profiles");
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten().filter(|entry| entry.path().is_dir()) {
                if let Some(profile) = entry.file_name().to_str() {
                    super::instances::scope_sync(profile.into(), || {
                        sweep_orphan_harness(app_handle)
                    });
                }
            }
        }
    }
    if has_owned_process() {
        return;
    }
    if take_startup_sweep() {
        if let Err(error) = terminate_stale_harness_processes(app_handle) {
            super::process::retry_startup_sweep();
            log::warn!("Startup Harness sweep failed: {error}");
        }
    } else {
        log::debug!("Skipping orphan Harness sweep: already swept during this launch");
    }
    if cfg!(windows) {
        return;
    }
    let pid_file = harness_pid_path(app_handle);
    let Ok(text) = fs::read_to_string(&pid_file) else {
        return;
    };
    let mut lines = text.lines();
    let (Some(pid), Some(port)) = (
        lines.next().and_then(|l| l.trim().parse::<u32>().ok()),
        lines.next().and_then(|l| l.trim().parse::<u16>().ok()),
    ) else {
        let _ = fs::remove_file(&pid_file);
        return;
    };
    if !is_port_in_use(port) {
        let _ = fs::remove_file(&pid_file);
        return;
    }
    if port_owner_pid(port) != Some(pid) {
        return;
    }
    log::warn!("Sweeping orphaned Harness process {pid} (port {port}) left by a previous session");
    kill_pid_tree(pid);
    let _ = fs::remove_file(&pid_file);
}

pub(super) fn port_owner_pid(port: u16) -> Option<u32> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let output = Command::new("netstat")
            .arg("-ano")
            .creation_flags(0x08000000)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        let needle = format!(":{port}");
        for line in text.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 5 || fields[0] != "TCP" {
                continue;
            }
            if !fields[1].ends_with(&needle) {
                continue;
            }
            if fields[3] == "LISTENING" {
                return fields[4].parse().ok();
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        let output = Command::new("lsof")
            .args(["-ti", &format!("tcp:{port}")])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .and_then(|l| l.trim().parse().ok())
    }
}

#[cfg(windows)]
pub(super) fn relaunch_marker_path(app_handle: &AppHandle) -> std::path::PathBuf {
    config::get_base_dir(app_handle).join(".dsh-relaunch-448")
}

#[cfg(windows)]
pub(super) fn dsh_bin_open_error(app_handle: &AppHandle) -> Option<i32> {
    std::fs::File::open(config::get_dsh_binary_path(app_handle))
        .err()
        .and_then(|e| e.raw_os_error())
}

/// 永不自行恢复（issue #35）。应用无法在运行时关闭继承的策略，只能脱离被污染的进程树：
#[cfg(windows)]
pub(super) fn relaunch_via_shell_escape(app_handle: &AppHandle) {
    let marker = relaunch_marker_path(app_handle);
    if marker.exists() {
        let _ = std::fs::remove_file(&marker);
        log::warn!("RedirectionGuard(448) relaunch did not escape, falling back to normal missing handling");
        return;
    }
    let _ = std::fs::write(&marker, b"1");
    let Ok(exe) = std::env::current_exe() else {
        log::warn!("RedirectionGuard(448) detected but current_exe unavailable, falling back");
        return;
    };
    match std::process::Command::new("explorer.exe").arg(&exe).spawn() {
        Ok(_) => {
            log::warn!(
                "RedirectionGuard(448) detected, relaunching via explorer to escape enforced context: {}",
                exe.display()
            );
            std::thread::sleep(std::time::Duration::from_millis(300));
            std::process::exit(0);
        }
        Err(e) => {
            log::warn!(
                "RedirectionGuard(448) detected but explorer spawn failed ({e}), falling back"
            );
        }
    }
}
