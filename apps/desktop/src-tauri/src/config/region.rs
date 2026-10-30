use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Domestic,
    Overseas,
}

static REGION: OnceLock<Region> = OnceLock::new();

pub fn detect_region() -> Region {
    *REGION.get_or_init(detect_region_uncached)
}

fn detect_region_uncached() -> Region {
    let locale = current_locale();
    let locale_zh = locale_is_china(&locale);
    let tz_china = is_china_timezone();
    let region = region_for(locale_zh, tz_china);
    log::info!(
        "Download region detected: {:?} (locale={locale:?}, china_timezone={tz_china})",
        region
    );
    region
}

fn region_for(locale_zh: bool, tz_china: bool) -> Region {
    if locale_zh || tz_china {
        Region::Domestic
    } else {
        Region::Overseas
    }
}

fn locale_is_china(locale: &str) -> bool {
    let normalized = locale.to_ascii_lowercase().replace('_', "-");
    normalized.starts_with("zh-cn") || normalized.starts_with("zh-hans")
}

fn tz_name_is_china(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    normalized.contains("asia/shanghai")
        || normalized == "prc"
        || normalized.starts_with("prc/")
        || normalized.starts_with("china/")
        || normalized.contains("china standard time")
}

fn current_locale() -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
        let mut buf = [0u16; 85];
        let len = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
        if len <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize - 1])
    }
    #[cfg(not(windows))]
    {
        for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(value) = std::env::var(var) {
                let base = value
                    .split('.')
                    .next()
                    .unwrap_or(&value)
                    .split('@')
                    .next()
                    .unwrap_or(&value);
                if !base.is_empty() && base != "C" && base != "POSIX" {
                    return base.to_string();
                }
            }
        }
        String::new()
    }
}

fn is_china_timezone() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Time::{
            DYNAMIC_TIME_ZONE_INFORMATION, GetDynamicTimeZoneInformation, TIME_ZONE_ID_INVALID,
        };
        let mut info: DYNAMIC_TIME_ZONE_INFORMATION = unsafe { std::mem::zeroed() };
        let id = unsafe { GetDynamicTimeZoneInformation(&mut info) };
        if id == TIME_ZONE_ID_INVALID {
            return false;
        }
        let len = info
            .TimeZoneKeyName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(info.TimeZoneKeyName.len());
        tz_name_is_china(&String::from_utf16_lossy(&info.TimeZoneKeyName[..len]))
    }
    #[cfg(not(windows))]
    {
        if let Ok(tz) = std::env::var("TZ") {
            if tz_name_is_china(&tz) {
                return true;
            }
        }
        if let Ok(target) = std::fs::read_link("/etc/localtime") {
            if tz_name_is_china(&target.to_string_lossy()) {
                return true;
            }
        }
        if let Ok(content) = std::fs::read_to_string("/etc/timezone") {
            if tz_name_is_china(content.trim()) {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_matching_only_matches_mainland_chinese() {
        assert!(locale_is_china("zh-CN"));
        assert!(locale_is_china("zh_CN.UTF-8"));
        assert!(locale_is_china("zh-Hans-CN"));
        assert!(locale_is_china("ZH_cn"));
        assert!(!locale_is_china("zh-TW"));
        assert!(!locale_is_china("zh-HK"));
        assert!(!locale_is_china("zh-SG"));
        assert!(!locale_is_china("en-US"));
        assert!(!locale_is_china("ja-JP"));
        assert!(!locale_is_china(""));
    }

    #[test]
    fn timezone_matching_only_matches_china_tz() {
        assert!(tz_name_is_china("Asia/Shanghai"));
        assert!(tz_name_is_china("/usr/share/zoneinfo/Asia/Shanghai"));
        assert!(tz_name_is_china("China Standard Time"));
        assert!(tz_name_is_china("PRC"));
        assert!(!tz_name_is_china("Asia/Singapore"));
        assert!(!tz_name_is_china("Asia/Taipei"));
        assert!(!tz_name_is_china("Asia/Hong_Kong"));
        assert!(!tz_name_is_china("America/New_York"));
        assert!(!tz_name_is_china(""));
    }

    #[test]
    fn region_combination_is_or() {
        assert_eq!(region_for(true, true), Region::Domestic);
        assert_eq!(region_for(true, false), Region::Domestic);
        assert_eq!(region_for(false, true), Region::Domestic);
        assert_eq!(region_for(false, false), Region::Overseas);
    }
}
