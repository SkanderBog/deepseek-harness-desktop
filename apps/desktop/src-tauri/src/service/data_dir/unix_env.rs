//! Unix（macOS / Linux）下 `DSH_HOME` 的持久化机制（issue #871 的跨平台部分）。

use std::path::PathBuf;

#[cfg(target_os = "macos")]
use crate::config::APP_IDENTIFIER;
use crate::service::cli::{strip_rc_block, upsert_rc_block, write_rc_with_backup};

use super::env::DATA_DIR_ENV;

const PROFILE_MARK_START: &str = "# >>> deepseek-harness dsh home >>>";
const PROFILE_MARK_END: &str = "# <<< deepseek-harness dsh home <<<";

#[cfg(target_os = "linux")]
const ENVIRONMENT_D_FILE: &str = ".config/environment.d/dsh-tauri.conf";

#[cfg(target_os = "macos")]
fn launch_agent_path(home: &std::path::Path) -> PathBuf {
    home.join(format!("Library/LaunchAgents/{APP_IDENTIFIER}.env.plist"))
}

pub(crate) fn profile_block(value: &str) -> String {
    format!(
        "{PROFILE_MARK_START}\nexport {DATA_DIR_ENV}=\"{}\"\n{PROFILE_MARK_END}\n",
        quote_sh(value)
    )
}

pub(crate) fn parse_profile_block(content: &str) -> Option<String> {
    let mut inside = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == PROFILE_MARK_START {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        if trimmed == PROFILE_MARK_END {
            break;
        }
        let Some(rest) = trimmed.strip_prefix("export ") else {
            continue;
        };
        let Some(rest) = rest.strip_prefix(DATA_DIR_ENV) else {
            continue;
        };
        let Some(raw) = rest.strip_prefix('=') else {
            continue;
        };
        let value = unquote_sh(raw.trim());
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

pub(crate) fn strip_profile_block(content: &str) -> String {
    strip_rc_block(content, PROFILE_MARK_START, PROFILE_MARK_END)
}

pub(crate) fn upsert_profile_block(content: &str, value: &str) -> String {
    upsert_rc_block(
        content,
        &profile_block(value),
        PROFILE_MARK_START,
        PROFILE_MARK_END,
    )
}

pub(crate) fn quote_sh(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if matches!(c, '\\' | '"' | '$' | '`') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub(crate) fn unquote_sh(value: &str) -> String {
    let inner = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(value);
    let mut out = String::with_capacity(inner.len());
    let mut escaped = false;
    for c in inner.chars() {
        if escaped {
            out.push(c);
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn quote_environment_d(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if c == '$' {
            out.push_str("$$");
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(target_os = "linux")]
pub(crate) fn environment_d_content(value: &str) -> String {
    format!("{DATA_DIR_ENV}=\"{}\"\n", quote_environment_d(value))
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn parse_environment_d(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix(DATA_DIR_ENV) else {
            continue;
        };
        let Some(raw) = rest.strip_prefix('=') else {
            continue;
        };
        let value = raw.trim().trim_matches('"').replace("$$", "$");
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

#[cfg(target_os = "macos")]
pub(crate) fn launch_agent_plist(value: &str) -> String {
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
            "<plist version=\"1.0\">\n",
            "<dict>\n",
            "  <key>Label</key>\n",
            "  <string>{0}.env</string>\n",
            "  <key>ProgramArguments</key>\n",
            "  <array>\n",
            "    <string>/bin/launchctl</string>\n",
            "    <string>setenv</string>\n",
            "    <string>{1}</string>\n",
            "    <string>{2}</string>\n",
            "  </array>\n",
            "  <key>RunAtLoad</key>\n",
            "  <true/>\n",
            "</dict>\n",
            "</plist>\n"
        ),
        APP_IDENTIFIER,
        DATA_DIR_ENV,
        xml_escape(value)
    )
}

#[cfg(target_os = "macos")]
pub(crate) fn xml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(target_os = "macos")]
pub(crate) fn parse_launch_agent(content: &str) -> Option<String> {
    let needle = format!("<string>{DATA_DIR_ENV}</string>");
    let mut lines = content.lines();
    while let Some(line) = lines.next() {
        if line.trim() != needle {
            continue;
        }
        let raw = lines.next()?.trim();
        let value = raw.strip_prefix("<string>")?.strip_suffix("</string>")?;
        let value = xml_unescape(value);
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

#[cfg(target_os = "macos")]
pub(crate) fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

pub(crate) fn home_dir() -> Result<PathBuf, String> {
    crate::config::user_home_dir()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| "DATA_DIR_ENV_HOME: $HOME 未设置".to_string())
}

pub(crate) fn read_profile() -> Result<String, String> {
    let path = home_dir()?.join(".profile");
    match std::fs::read_to_string(&path) {
        Ok(content) => Ok(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!("DATA_DIR_ENV_READ: {}: {error}", path.display())),
    }
}

pub(crate) fn write_profile(content: &str) -> Result<(), String> {
    let path = home_dir()?.join(".profile");
    write_rc_with_backup(&path, content)
        .map_err(|error| format!("DATA_DIR_ENV_WRITE: {}: {error}", path.display()))
}

pub(crate) fn write_value(value: &str) -> Result<(), String> {
    let profile = read_profile()?;
    write_profile(&upsert_profile_block(&profile, value))?;
    write_platform_extra(value)?;
    apply_to_process(Some(value));
    Ok(())
}

pub(crate) fn clear_value() -> Result<(), String> {
    let profile = read_profile()?;
    let cleaned = strip_profile_block(&profile);
    if cleaned != profile {
        write_profile(&cleaned)?;
    }
    clear_platform_extra()?;
    apply_to_process(None);
    Ok(())
}

fn apply_to_process(value: Option<&str>) {
    match value {
        Some(text) => unsafe { std::env::set_var(DATA_DIR_ENV, text) },
        None => unsafe { std::env::remove_var(DATA_DIR_ENV) },
    }
}

pub(crate) fn read_value() -> Option<String> {
    if let Some(value) = std::env::var(DATA_DIR_ENV)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
    {
        return Some(value);
    }
    if let Ok(profile) = read_profile() {
        if let Some(value) = parse_profile_block(&profile) {
            return Some(value);
        }
    }
    read_platform_extra()
}

pub(crate) fn restore_process_env() {
    if std::env::var(DATA_DIR_ENV)
        .map(|text| !text.trim().is_empty())
        .unwrap_or(false)
    {
        return;
    }
    let Some(value) = read_value() else {
        return;
    };
    log::info!("restored {DATA_DIR_ENV} from the persisted configuration: {value}");
    unsafe { std::env::set_var(DATA_DIR_ENV, value) };
}

#[cfg(target_os = "macos")]
fn write_platform_extra(value: &str) -> Result<(), String> {
    let plist = launch_agent_path(&home_dir()?);
    if let Some(parent) = plist.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("DATA_DIR_ENV_MKDIR: {}: {error}", parent.display()))?;
    }
    std::fs::write(&plist, launch_agent_plist(value))
        .map_err(|error| format!("DATA_DIR_ENV_WRITE: {}: {error}", plist.display()))?;
    launchctl(&["setenv", DATA_DIR_ENV, value])?;
    let domain = launch_domain();
    let text = plist.to_string_lossy().into_owned();
    let _ = launchctl(&["bootout", &domain, &text]);
    launchctl(&["bootstrap", &domain, &text])
}

#[cfg(target_os = "macos")]
fn clear_platform_extra() -> Result<(), String> {
    let plist = launch_agent_path(&home_dir()?);
    if plist.exists() {
        let _ = launchctl(&[
            "bootout",
            &launch_domain(),
            &plist.to_string_lossy().into_owned(),
        ]);
        std::fs::remove_file(&plist)
            .map_err(|error| format!("DATA_DIR_ENV_REMOVE: {}: {error}", plist.display()))?;
    }
    let _ = launchctl(&["unsetenv", DATA_DIR_ENV]);
    Ok(())
}

#[cfg(target_os = "macos")]
fn read_platform_extra() -> Option<String> {
    let plist = launch_agent_path(&home_dir().ok()?);
    parse_launch_agent(&std::fs::read_to_string(plist).ok()?)
}

#[cfg(target_os = "macos")]
fn launch_domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}

#[cfg(target_os = "macos")]
fn launchctl(args: &[&str]) -> Result<(), String> {
    let output = std::process::Command::new("/bin/launchctl")
        .args(args)
        .output()
        .map_err(|error| format!("DATA_DIR_ENV_LAUNCHCTL: launchctl {}: {error}", args[0]))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(format!(
        "DATA_DIR_ENV_LAUNCHCTL: launchctl {} failed: {stderr}",
        args[0]
    ))
}

#[cfg(target_os = "linux")]
fn write_platform_extra(value: &str) -> Result<(), String> {
    let path = home_dir()?.join(ENVIRONMENT_D_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("DATA_DIR_ENV_MKDIR: {}: {error}", parent.display()))?;
    }
    std::fs::write(&path, environment_d_content(value))
        .map_err(|error| format!("DATA_DIR_ENV_WRITE: {}: {error}", path.display()))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn clear_platform_extra() -> Result<(), String> {
    let path = home_dir()?.join(ENVIRONMENT_D_FILE);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("DATA_DIR_ENV_REMOVE: {}: {error}", path.display())),
    }
}

#[cfg(target_os = "linux")]
fn read_platform_extra() -> Option<String> {
    let path = home_dir().ok()?.join(ENVIRONMENT_D_FILE);
    parse_environment_d(&std::fs::read_to_string(path).ok()?)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn write_platform_extra(_value: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn clear_platform_extra() -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn read_platform_extra() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_block_round_trips_and_escapes_shell_metacharacters() {
        let value = "/Volumes/Data/D$SH Home`x`\"q\"";
        let block = profile_block(value);
        assert!(block.starts_with(PROFILE_MARK_START));
        assert!(block.contains("export DSH_HOME=\"/Volumes/Data/D\\$SH Home\\`x\\`\\\"q\\\"\""));
        assert_eq!(parse_profile_block(&block), Some(value.to_string()));
    }

    #[test]
    fn profile_block_upsert_is_idempotent_and_keeps_other_blocks() {
        const PATH_BLOCK: &str = "# >>> deepseek-harness dsh >>>\nexport PATH=\"$HOME/.local/bin:$PATH\"\n# <<< deepseek-harness dsh <<<\n";
        let once = upsert_profile_block(PATH_BLOCK, "/data/dsh");
        assert!(once.contains("export PATH="));
        assert_eq!(parse_profile_block(&once), Some("/data/dsh".to_string()));
        assert_eq!(upsert_profile_block(&once, "/data/dsh"), once);
        assert_eq!(strip_profile_block(&once), PATH_BLOCK);
    }

    #[test]
    fn parse_profile_block_ignores_foreign_exports() {
        let content = format!("{PROFILE_MARK_START}\nexport OTHER=x\n{PROFILE_MARK_END}\n");
        assert_eq!(parse_profile_block(&content), None);
        assert_eq!(parse_profile_block("export DSH_HOME=/data/dsh\n"), None);
    }

    #[test]
    fn quote_sh_only_escapes_what_a_double_quoted_shell_string_needs() {
        assert_eq!(quote_sh("/data/dsh"), "/data/dsh");
        assert_eq!(quote_sh("/data/a b"), "/data/a b");
        assert_eq!(quote_sh("a\"b"), "a\\\"b");
        assert_eq!(quote_sh("a$b"), "a\\$b");
        assert_eq!(quote_sh("a\\b"), "a\\\\b");
    }

    #[test]
    fn environment_d_quoting_matches_systemd_rules() {
        assert_eq!(quote_environment_d("/data/dsh"), "/data/dsh");
        assert_eq!(quote_environment_d("/data/D$SH"), "/data/D$$SH");
        assert_eq!(quote_environment_d("C:\\data"), "C:\\data");
        assert_eq!(parse_environment_d("# c\nDSH_HOME=\"/data/dsh\"\n"), Some("/data/dsh".to_string()));
        assert_eq!(parse_environment_d("DSH_HOME=\n"), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn launch_agent_plist_is_well_formed_xml_with_escaped_values() {
        let value = "/Volumes/a&b/<dsh>";
        let plist = launch_agent_plist(value);
        assert!(plist.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(plist.contains("<string>/bin/launchctl</string>"));
        assert!(plist.contains("<string>setenv</string>"));
        assert!(plist.contains("<string>/Volumes/a&amp;b/&lt;dsh&gt;</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.ends_with("</plist>\n"));
        assert_eq!(parse_launch_agent(&plist), Some(value.to_string()));
        let dir = std::env::temp_dir().join(format!("dsh-launch-agent-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("env.plist");
        std::fs::write(&file, &plist).unwrap();
        let lint = std::process::Command::new("/usr/bin/plutil")
            .args(["-lint", file.to_str().unwrap()])
            .output();
        if let Ok(output) = lint {
            assert!(output.status.success(), "plutil -lint rejected the plist");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
