use std::ffi::OsString;

const AUTO_HEAP_MIN_MB: u64 = 2048;

const AUTO_HEAP_MAX_MB: u64 = 16384;

const INHERITED_HEAP_LIMIT_MB: u32 = 0;

const HEAP_LIMIT_FLAGS: [&str; 2] = ["--max-old-space-size", "--max_old_space_size"];

const HEAP_PERCENTAGE_FLAGS: [&str; 2] = [
    "--max-old-space-size-percentage",
    "--max_old_space_size_percentage",
];

fn heap_flags() -> impl Iterator<Item = &'static str> {
    HEAP_LIMIT_FLAGS
        .iter()
        .chain(HEAP_PERCENTAGE_FLAGS.iter())
        .copied()
}

fn for_each_heap_flag(node_options: &str, mut visit: impl FnMut(&'static str, &str, bool)) {
    let mut tokens = node_options.split_whitespace();
    while let Some(token) = tokens.next() {
        let token = token.trim_matches(['"', '\'']);
        for flag in heap_flags() {
            let attached = token
                .strip_prefix(flag)
                .and_then(|tail| tail.strip_prefix('='));
            if token != flag && attached.is_none() {
                continue;
            }
            let value = match attached {
                Some(value) => value.trim_matches(['"', '\'']),
                None => tokens
                    .next()
                    .map(|value| value.trim_matches(['"', '\'']))
                    .unwrap_or_default(),
            };
            let is_limit = if HEAP_LIMIT_FLAGS.contains(&flag) {
                value.parse::<u32>().is_ok_and(|mb| mb > 0)
            } else {
                value.parse::<f64>().is_ok_and(|percent| percent > 0.0)
            };
            visit(flag, value, is_limit);
            break;
        }
    }
}

fn node_options_heap_limit(node_options: &str) -> Option<u32> {
    let mut limit = None;
    let mut has_percentage = false;
    for_each_heap_flag(node_options, |flag, value, is_limit| {
        if !is_limit {
            return;
        }
        if !HEAP_LIMIT_FLAGS.contains(&flag) {
            has_percentage = true;
            return;
        }
        if let Ok(mb) = value.parse::<u32>() {
            limit = Some(mb);
        }
    });
    if has_percentage {
        return None;
    }
    limit
}

pub(super) fn node_options_has_heap_flags(node_options: &str) -> bool {
    let mut found = false;
    for_each_heap_flag(node_options, |_, _, is_limit| found |= is_limit);
    found
}

pub(super) fn node_options_without_heap_flags(node_options: &str) -> Option<String> {
    let mut kept: Vec<&str> = Vec::new();
    let mut tokens = node_options.split_whitespace();
    while let Some(token) = tokens.next() {
        let bare = token.trim_matches(['"', '\'']);
        let is_heap_flag =
            heap_flags().any(|flag| bare == flag || bare.starts_with(&format!("{flag}=")));
        if !is_heap_flag {
            kept.push(token);
            continue;
        }
        if !bare.contains('=') {
            tokens.next();
        }
    }
    (!kept.is_empty()).then(|| kept.join(" "))
}

pub(super) fn node_options_heap_limit_mb() -> Option<u32> {
    std::env::var("NODE_OPTIONS")
        .ok()
        .as_deref()
        .and_then(node_options_heap_limit)
}

pub(super) fn node_options_has_heap_flags_env() -> bool {
    std::env::var("NODE_OPTIONS").is_ok_and(|value| node_options_has_heap_flags(&value))
}

pub(super) fn auto_heap_limit_mb(total_mb: u64) -> u32 {
    (total_mb / 2).clamp(AUTO_HEAP_MIN_MB, AUTO_HEAP_MAX_MB) as u32
}

pub(super) fn resolve_heap_limit_mb(
    configured: Option<u32>,
    inherited: bool,
    total_mb: Option<u64>,
) -> Option<u32> {
    if let Some(configured) = configured {
        return Some(configured);
    }
    if inherited {
        return Some(INHERITED_HEAP_LIMIT_MB);
    }
    total_mb.filter(|total| *total > 0).map(auto_heap_limit_mb)
}

pub(super) fn effective_limit_mb(heap_mb: Option<u32>, inherited: Option<u32>) -> Option<u32> {
    match heap_mb {
        Some(INHERITED_HEAP_LIMIT_MB) => inherited,
        other => other,
    }
}

pub(super) fn heap_option_arg(heap_mb: Option<u32>) -> Option<OsString> {
    heap_mb
        .filter(|mb| *mb > 0)
        .map(|mb| OsString::from(format!("--max-old-space-size={mb}")))
}

pub(crate) fn effective_heap_limit_mb(configured: Option<u32>) -> Option<u32> {
    let inherited = node_options_heap_limit_mb();
    effective_limit_mb(
        resolve_heap_limit_mb(
            configured,
            node_options_has_heap_flags_env(),
            physical_memory_mb(),
        ),
        inherited,
    )
}

#[cfg(windows)]
pub(super) fn physical_memory_mb() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status.ullTotalPhys / 1024 / 1024)
}

#[cfg(target_os = "macos")]
pub(super) fn physical_memory_mb() -> Option<u64> {
    use objc2_foundation::NSProcessInfo;
    let bytes = NSProcessInfo::processInfo().physicalMemory();
    (bytes > 0).then(|| bytes / 1024 / 1024)
}

#[cfg(target_os = "linux")]
pub(super) fn physical_memory_mb() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let total_kb = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    (total_kb > 0).then(|| total_kb / 1024)
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub(super) fn physical_memory_mb() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_limit_clamps_at_half_physical_memory() {
        assert_eq!(auto_heap_limit_mb(4096), 2048);
        assert_eq!(auto_heap_limit_mb(8192), 4096);
        assert_eq!(auto_heap_limit_mb(12288), 6144);
        assert_eq!(auto_heap_limit_mb(16384), 8192);
        assert_eq!(auto_heap_limit_mb(32768), 16384);
        assert_eq!(auto_heap_limit_mb(63231), AUTO_HEAP_MAX_MB as u32);
        assert_eq!(auto_heap_limit_mb(131072), AUTO_HEAP_MAX_MB as u32);
    }

    #[test]
    fn resolve_preserves_configured_value_and_falls_back_on_unknown_memory() {
        assert_eq!(
            resolve_heap_limit_mb(Some(12288), false, Some(16384)),
            Some(12288)
        );
        assert_eq!(resolve_heap_limit_mb(None, false, Some(16384)), Some(8192));
        assert_eq!(
            resolve_heap_limit_mb(None, false, Some(65536)),
            Some(AUTO_HEAP_MAX_MB as u32)
        );
        assert_eq!(resolve_heap_limit_mb(None, false, Some(0)), None);
        assert_eq!(resolve_heap_limit_mb(None, false, None), None);
    }

    #[test]
    fn configured_limit_wins_over_inherited_node_options() {
        assert_eq!(
            resolve_heap_limit_mb(Some(1600), true, Some(32768)),
            Some(1600)
        );
        assert_eq!(
            resolve_heap_limit_mb(Some(12288), true, Some(32768)),
            Some(12288)
        );
    }

    #[test]
    fn inherited_heap_limit_is_left_to_node_options() {
        for options in [
            "--max-old-space-size=8192",
            "--trace-warnings --max-old-space-size 8192",
            "--max-old-space-size \"8192\"",
            "--max_old_space_size=8192",
            "--max_old_space_size 8192",
            "--max-old-space-size-percentage=50",
            "--max_old_space_size_percentage 50",
        ] {
            assert!(node_options_has_heap_flags(options), "{options}");
            assert_eq!(
                resolve_heap_limit_mb(None, true, Some(32768)),
                Some(INHERITED_HEAP_LIMIT_MB),
                "{options}"
            );
        }
        for options in [
            "--max-old-space-size=8192",
            "--trace-warnings --max-old-space-size 8192",
            "--max-old-space-size \"8192\"",
            "--max_old_space_size=8192",
            "--max_old_space_size 8192",
        ] {
            assert_eq!(node_options_heap_limit(options), Some(8192), "{options}");
        }
        for options in [
            "--max-old-space-size-percentage=50",
            "--max_old_space_size_percentage 50",
        ] {
            assert_eq!(node_options_heap_limit(options), None, "{options}");
        }
    }

    #[test]
    fn unrelated_or_malformed_node_options_do_not_disable_adaptive_limit() {
        for options in [
            "--trace-warnings",
            "--no-max-old-space-size=8192",
            "--max-old-space-size-extra=8192",
            "--max-old-space-size=",
            "--max-old-space-size=not-a-number",
            "--max-old-space-size 0",
        ] {
            assert_eq!(node_options_heap_limit(options), None, "{options}");
            assert!(!node_options_has_heap_flags(options), "{options}");
            assert_eq!(
                resolve_heap_limit_mb(None, false, Some(16384)),
                Some(8192),
                "{options}"
            );
        }
    }

    #[test]
    fn the_last_heap_limit_in_node_options_wins() {
        assert_eq!(
            node_options_heap_limit("--max-old-space-size=4096 --max-old-space-size=8192"),
            Some(8192)
        );
        assert_eq!(
            node_options_heap_limit("--max-old-space-size=8192 --max-old-space-size=4096"),
            Some(4096)
        );
        assert_eq!(
            node_options_heap_limit("--max-old-space-size=4096 --max-old-space-size=oops"),
            Some(4096)
        );
    }

    #[test]
    fn strips_every_heap_flag_but_keeps_the_rest_of_node_options() {
        assert_eq!(
            node_options_without_heap_flags("--require C:/probe.cjs --max-old-space-size=8192"),
            Some("--require C:/probe.cjs".to_string())
        );
        assert_eq!(
            node_options_without_heap_flags("--max-old-space-size 8192 --trace-warnings"),
            Some("--trace-warnings".to_string())
        );
        assert_eq!(
            node_options_without_heap_flags("--max-old-space-size-percentage=50 --require a.cjs"),
            Some("--require a.cjs".to_string())
        );
        assert_eq!(
            node_options_without_heap_flags(
                "--max-old-space-size=8192 --max-old-space-size-percentage 50 --require C:/probe.cjs"
            ),
            Some("--require C:/probe.cjs".to_string())
        );
        assert_eq!(
            node_options_without_heap_flags("--max_old_space_size 8192"),
            None
        );
        assert_eq!(
            node_options_without_heap_flags("--trace-warnings"),
            Some("--trace-warnings".to_string())
        );
    }

    #[test]
    fn node_options_heap_limit_reads_the_inherited_value() {
        assert_eq!(
            node_options_heap_limit("--max-old-space-size=8192"),
            Some(8192)
        );
        assert_eq!(
            node_options_heap_limit("--max-old-space-size 1600"),
            Some(1600)
        );
        assert_eq!(
            node_options_heap_limit("--max_old_space_size=4096"),
            Some(4096)
        );
    }

    #[test]
    fn a_valid_percentage_flag_hides_the_size_flag() {
        for options in [
            "--max-old-space-size=8192 --max-old-space-size-percentage=50",
            "--max-old-space-size-percentage=50 --max-old-space-size=8192",
            "--max_old_space_size=8192 --max_old_space_size_percentage 50",
        ] {
            assert_eq!(node_options_heap_limit(options), None, "{options}");
            assert!(node_options_has_heap_flags(options), "{options}");
        }
        assert_eq!(
            node_options_heap_limit(
                "--max-old-space-size=8192 --max-old-space-size-percentage=abc"
            ),
            Some(8192)
        );
    }

    #[test]
    fn heap_option_arg_skips_the_inherited_sentinel() {
        assert_eq!(heap_option_arg(Some(INHERITED_HEAP_LIMIT_MB)), None);
        assert_eq!(heap_option_arg(None), None);
        assert_eq!(
            heap_option_arg(Some(2048)),
            Some(OsString::from("--max-old-space-size=2048"))
        );
    }

    #[test]
    fn effective_limit_reports_what_the_child_process_gets() {
        assert_eq!(effective_limit_mb(Some(1600), Some(8192)), Some(1600));
        assert_eq!(
            effective_limit_mb(Some(INHERITED_HEAP_LIMIT_MB), Some(8192)),
            Some(8192)
        );
        assert_eq!(
            effective_limit_mb(Some(INHERITED_HEAP_LIMIT_MB), None),
            None
        );
        assert_eq!(effective_limit_mb(None, Some(8192)), None);
    }
}
