use std::path::{Path, PathBuf};

pub fn file_stamp(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0)
}

/// （issue #116）。低于该推荐值时在启动日志给出明确提示，引导用户调高参数。
#[cfg(unix)]
pub const MIN_INOTIFY_MAX_USER_WATCHES: u64 = 524_288;

#[cfg(unix)]
pub fn linux_inotify_max_user_watches() -> Option<u64> {
    let content = std::fs::read_to_string("/proc/sys/fs/inotify/max_user_watches").ok()?;
    content.trim().parse::<u64>().ok()
}

pub fn unique_download_path(destination: &Path) -> PathBuf {
    use std::env;

    let dir = match destination.parent() {
        Some(parent) if parent.is_dir() => parent.to_path_buf(),
        _ => env::var("USERPROFILE")
            .map(PathBuf::from)
            .map(|home| home.join("Downloads"))
            .unwrap_or_else(|_| PathBuf::from(".")),
    };
    let name = destination
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download");
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) => (stem.to_string(), format!(".{ext}")),
        None => (name.to_string(), String::new()),
    };
    let mut counter = 0usize;
    loop {
        let candidate = if counter == 0 {
            dir.join(name)
        } else {
            dir.join(format!("{stem} ({counter}){ext}"))
        };
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}

pub fn search_node_binary(dir: &PathBuf, target: &str) -> Option<PathBuf> {
    use std::fs;

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = search_node_binary(&path, target) {
                    return Some(found);
                }
            } else if path.file_name().and_then(|n| n.to_str()) == Some("node")
                || path.file_name().and_then(|n| n.to_str()) == Some("node.exe")
            {
                return Some(path);
            }
        }
    }

    let candidate = dir.join(target);
    if candidate.exists() {
        return Some(candidate);
    }

    None
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn inotify_watch_limit_is_below_recommended_minimum() {
        assert!(
            MIN_INOTIFY_MAX_USER_WATCHES > 65_536,
            "recommended minimum {} should exceed common defaults",
            MIN_INOTIFY_MAX_USER_WATCHES
        );
    }

    #[test]
    fn inotify_watch_limit_reads_procfs_when_present() {
        match linux_inotify_max_user_watches() {
            Some(v) => assert!(v > 0),
            None => {
                println!("inotify max_user_watches not available; skipping");
            }
        }
    }
}
