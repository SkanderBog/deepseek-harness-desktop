
use semver::Version;

pub(super) fn current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// AppImage（issue #1230）。
pub(super) const DISTRO_MANAGED: bool = option_env!("DSH_DISTRO_MANAGED").is_some();

pub(super) fn supports_stable_updates(identifier: &str) -> bool {
    stable_updates_allowed(identifier, DISTRO_MANAGED)
}

fn stable_updates_allowed(identifier: &str, distro_managed: bool) -> bool {
    !distro_managed && identifier != "dsh-tauri-nightly"
}

pub(super) fn parse_version(v: &str) -> Option<Version> {
    Version::parse(v.trim().trim_start_matches('v')).ok()
}

pub(super) fn is_stable(version: &Version) -> bool {
    version.pre.is_empty() && version.build.is_empty()
}

pub(super) fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// Intel 安装包（issue #576）。macOS 因此必须运行时探测宿主。
#[cfg(target_os = "macos")]
fn host_is_aarch64() -> bool {
    cfg!(target_arch = "aarch64") || apple_silicon_host()
}

#[cfg(not(target_os = "macos"))]
fn host_is_aarch64() -> bool {
    cfg!(target_arch = "aarch64")
}

#[cfg(target_os = "macos")]
fn apple_silicon_host() -> bool {
    static CACHE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| {
        let mut value: i32 = 0;
        let mut size = std::mem::size_of::<i32>();
        let name = b"hw.optional.arm64\0";
        let code = unsafe {
            libc::sysctlbyname(
                name.as_ptr() as *const libc::c_char,
                &mut value as *mut i32 as *mut libc::c_void,
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        code == 0 && value == 1
    })
}

fn arch_rank(name: &str) -> i8 {
    arch_rank_for(name, host_is_aarch64())
}

fn arch_rank_for(name: &str, host_aarch64: bool) -> i8 {
    let lower = name.to_lowercase();
    if lower.contains("universal") {
        return 1;
    }
    let markers: &[&str] = if host_aarch64 {
        &["aarch64", "arm64", "apple-silicon", "-arm", "_arm"]
    } else {
        &["x86_64", "amd64", "x64", "intel", "-x86", "_x86"]
    };
    if markers.iter().any(|k| lower.contains(k)) {
        2
    } else {
        0
    }
}

#[cfg(target_os = "linux")]
fn linux_package_family() -> &'static str {
    let debianish = std::path::Path::new("/etc/debian_version").exists()
        || std::path::Path::new("/usr/bin/dpkg").exists();
    let rpmish = std::path::Path::new("/etc/redhat-release").exists()
        || std::path::Path::new("/etc/fedora-release").exists()
        || std::path::Path::new("/usr/bin/rpm").exists();
    if debianish {
        "deb"
    } else if rpmish {
        "rpm"
    } else {
        "unknown"
    }
}

#[cfg(target_os = "linux")]
fn linux_prefs(family: &str) -> &'static [&'static str] {
    match family {
        "rpm" => &[".rpm", ".AppImage", ".deb"],
        "deb" => &[".deb", ".AppImage", ".rpm"],
        _ => &[".AppImage", ".deb", ".rpm"],
    }
}

///   （aarch64）的安装包（issue #33）。
///   `.rpm`**，issue #79），AppImage 作为便携兜底，同样按架构匹配。优先原生包
pub(super) fn pick_asset(assets: &[String]) -> Option<String> {
    #[cfg(target_os = "windows")]
    let prefs: &[&str] = &[".exe", ".msi"];
    #[cfg(target_os = "macos")]
    let prefs: &[&str] = &[".dmg"];
    #[cfg(target_os = "linux")]
    let prefs: &[&str] = linux_prefs(linux_package_family());

    let mut best: Option<(usize, i8, String)> = None;
    for name in assets {
        let Some(idx) = prefs.iter().position(|p| name.ends_with(p)) else {
            continue;
        };
        let rank = prefs.len() - idx; // 扩展名优先级：越靠前越高
        let ar = arch_rank(name); // 架构匹配度：同扩展名下优先选匹配架构
        if best
            .as_ref()
            .is_none_or(|(r, a, _)| rank > *r || (rank == *r && ar > *a))
        {
            best = Some((rank, ar, name.clone()));
        }
    }
    best.map(|(_, _, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nightly_does_not_receive_stable_installers() {
        assert!(!stable_updates_allowed("dsh-tauri-nightly", false));
    }

    #[test]
    fn stable_and_legacy_identifiers_keep_stable_updates() {
        for identifier in ["dsh-tauri", "io.github.hairyf.deepseek-harness-desktop"] {
            assert!(stable_updates_allowed(identifier, false));
        }
    }

    #[test]
    fn distro_managed_builds_never_receive_installers() {
        for identifier in [
            "dsh-tauri",
            "dsh-tauri-nightly",
            "io.github.hairyf.deepseek-harness-desktop",
        ] {
            assert!(!stable_updates_allowed(identifier, true));
        }
    }

    #[test]
    fn parse_version_strips_v_prefix() {
        assert_eq!(parse_version("v0.5.2").unwrap().to_string(), "0.5.2");
        assert_eq!(parse_version("0.5.2").unwrap().to_string(), "0.5.2");
        assert_eq!(
            parse_version("0.5.2-rc.1").unwrap().to_string(),
            "0.5.2-rc.1"
        );
        assert_eq!(parse_version("abc"), None);
        assert_eq!(parse_version("test-main-123"), None);
        assert_eq!(parse_version("0.5"), None);
    }

    #[test]
    fn is_newer_compares_semver() {
        assert!(is_newer("0.5.2", "0.5.1"));
        assert!(is_newer("1.0.0", "0.9.0"));
        assert!(is_newer("0.7.14-rc.1", "0.7.13"));
        assert!(is_newer("0.7.14", "0.7.14-rc.1"));
        assert!(is_newer("0.7.14-rc.2", "0.7.14-rc.1"));
        assert!(!is_newer("0.5.1", "0.5.2"));
        assert!(!is_newer("0.5.1", "0.5.1"));
        assert!(!is_newer("0.7.14-rc.1", "0.7.14"));
    }

    #[test]
    fn is_newer_ignores_unparseable() {
        assert!(!is_newer("abc", "0.5.1"));
        assert!(!is_newer("0.5.1", "abc"));
        assert!(!is_newer("test-main-123", "0.5.1"));
    }

    #[test]
    fn is_stable_only_pure_numeric() {
        assert!(is_stable(&parse_version("0.7.14").unwrap()));
        assert!(is_stable(&parse_version("0.7.0").unwrap()));
        assert!(!is_stable(&parse_version("0.7.14-rc.1").unwrap()));
        assert!(!is_stable(&parse_version("0.7.14-beta.2").unwrap()));
        assert!(!is_stable(&parse_version("0.7.14-alpha").unwrap()));
        assert!(!is_stable(&parse_version("0.7.14+build.5").unwrap()));
    }

    #[test]
    fn pick_asset_prefers_matching_suffix() {
        let mk = |name: &str| name.to_string();
        #[cfg(target_os = "windows")]
        {
            let assets: Vec<String> = vec![mk("app-x86_64-setup.exe"), mk("app-x64_en-US.msi")];
            assert_eq!(pick_asset(&assets).as_deref(), Some("app-x86_64-setup.exe"));
        }
        #[cfg(target_os = "macos")]
        {
            let assets: Vec<String> = vec![mk("app.dmg"), mk("app-x86_64.tar.gz")];
            assert_eq!(pick_asset(&assets).as_deref(), Some("app.dmg"));
        }
        let no_match: Vec<String> = vec![mk("README.md")];
        assert!(pick_asset(&no_match).is_none());
        assert!(pick_asset(&[]).is_none());
    }

    /// 架构匹配度按**宿主 CPU** 判定而非编译目标：issue #576 现场是 Intel 版应用在
    #[test]
    fn arch_rank_for_matches_host_and_universal() {
        assert_eq!(
            arch_rank_for("Deepseek.Harness.Desktop_0.15.3_aarch64.dmg", true),
            2
        );
        assert_eq!(
            arch_rank_for("Deepseek.Harness.Desktop_0.15.3_x64.dmg", true),
            0
        );
        assert_eq!(
            arch_rank_for("Deepseek.Harness.Desktop_0.15.3_x64.dmg", false),
            2
        );
        assert_eq!(
            arch_rank_for("Deepseek.Harness.Desktop_0.15.3_aarch64.dmg", false),
            0
        );
        assert_eq!(
            arch_rank_for("Deepseek.Harness.Desktop_0.15.3_amd64.AppImage", false),
            2
        );
        assert_eq!(
            arch_rank_for("Deepseek.Harness.Desktop-0.15.3-1.x86_64.rpm", false),
            2
        );
        assert_eq!(
            arch_rank_for("Deepseek.Harness.Desktop-0.15.3-1.aarch64.rpm", true),
            2
        );
        assert_eq!(
            arch_rank_for("Deepseek.Harness.Desktop-universal.dmg", true),
            1
        );
        assert_eq!(
            arch_rank_for("Deepseek.Harness.Desktop-universal.dmg", false),
            1
        );
        assert_eq!(arch_rank_for("app.dmg", true), 0);
        assert_eq!(arch_rank_for("app.dmg", false), 0);
    }

    /// Linux 资产优先级：包管理家族决定原生格式最优先（issue #79），未知家族落回 deb。
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_prefs_prefers_native_format_per_family() {
        let deb = linux_prefs("deb");
        let deb_rank = deb.iter().position(|p| *p == ".deb").unwrap();
        let appimage_rank = deb.iter().position(|p| *p == ".AppImage").unwrap();
        let rpm_rank = deb.iter().position(|p| *p == ".rpm").unwrap();
        assert!(
            deb_rank < appimage_rank && deb_rank < rpm_rank,
            "deb 系应优先 .deb，实际 {deb:?}"
        );

        let rpm = linux_prefs("rpm");
        let rpm_rank = rpm.iter().position(|p| *p == ".rpm").unwrap();
        let deb_rank = rpm.iter().position(|p| *p == ".deb").unwrap();
        let appimage_rank = rpm.iter().position(|p| *p == ".AppImage").unwrap();
        assert!(
            rpm_rank < appimage_rank && rpm_rank < deb_rank,
            "rpm 系应优先 .rpm，实际 {rpm:?}"
        );

        let unknown = linux_prefs("unknown");
        let appimage_rank = unknown.iter().position(|p| *p == ".AppImage").unwrap();
        let deb_rank = unknown.iter().position(|p| *p == ".deb").unwrap();
        let rpm_rank = unknown.iter().position(|p| *p == ".rpm").unwrap();
        assert!(
            appimage_rank < deb_rank && appimage_rank < rpm_rank,
            "未知家族应优先 AppImage，实际 {unknown:?}"
        );
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn host_arch_is_apple_silicon_for_aarch64_build() {
        assert!(host_is_aarch64());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn pick_asset_prefers_host_arch_dmg() {
        let mk = |name: &str| name.to_string();
        let aarch64 = "Deepseek.Harness.Desktop_0.6.6_aarch64.dmg";
        let x64 = "Deepseek.Harness.Desktop_0.6.6_x64.dmg";
        let universal = "Deepseek.Harness.Desktop_0.6.6-universal.dmg";
        let picked = pick_asset(&[mk(aarch64), mk(x64)]).unwrap();
        assert_eq!(picked, if host_is_aarch64() { aarch64 } else { x64 });
        let wrong = if host_is_aarch64() { x64 } else { aarch64 };
        let picked = pick_asset(&[mk(wrong), mk(universal)]).unwrap();
        assert_eq!(picked, universal);
    }
}
