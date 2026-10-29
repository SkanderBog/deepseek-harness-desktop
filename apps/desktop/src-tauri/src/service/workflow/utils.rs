use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use crate::utils::decode_process_line;

const DSH_MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
const DSH_MAX_BACKUPS: usize = 3;
static DSH_LOG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
fn dsh_log_lock() -> &'static Mutex<()> {
    DSH_LOG_LOCK.get_or_init(|| Mutex::new(()))
}

pub(super) fn loopback_http_client(timeout: Duration) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(crate::config::LOOPBACK_CONNECT_TIMEOUT)
        .timeout(timeout)
        .build()
}

pub(super) fn health_probe_plugin_urls(port: u16) -> Vec<String> {
    vec![
        format!("http://127.0.0.1:{port}/plugins/@deepseek-ai/dsh-client-ui-layout/client.js"),
        format!("http://127.0.0.1:{port}/plugins/@deepseek-ai/dsh-client-modules/client.js"),
    ]
}

pub(super) fn client_urls_from_boot_html(port: u16, html: &str) -> Option<Vec<String>> {
    let marker = "globalThis[\"__DSH_BOOT__\"] = ";
    let start = html.find(marker)? + marker.len();
    let end = html[start..].find("</script>")? + start;
    let json = html[start..end].trim().trim_end_matches(';').trim();
    let boot: serde_json::Value = serde_json::from_str(json).ok()?;
    let entries = boot.get("entries")?.as_array()?;
    let mut paths = Vec::new();

    for script in html.split("<script").skip(1) {
        let Some(src_start) = script.find("src=\"") else {
            continue;
        };
        let rest = &script[src_start + 5..];
        let Some(src_end) = rest.find('\"') else {
            continue;
        };
        let src = decode_html_attribute(&rest[..src_end]);
        if is_client_bundle_path(&src) {
            paths.push(src);
        }
    }
    for entry in entries {
        let Some(url) = entry.get("url").and_then(|v| v.as_str()) else {
            continue;
        };
        let url = decode_html_attribute(url);
        if is_client_bundle_path(&url) {
            paths.push(url);
        }
    }
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return None;
    }
    Some(
        paths
            .into_iter()
            .map(|path| format!("http://127.0.0.1:{port}/{}", path.trim_start_matches('/')))
            .collect(),
    )
}

fn decode_html_attribute(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn is_client_bundle_path(path: &str) -> bool {
    let path = path.trim_start_matches("./");
    (path.starts_with("plugins/") || path.starts_with("/plugins/")) && path.contains("client.js")
}

pub(super) fn looks_like_plugin_bundle(ok_status: bool, body: &str) -> bool {
    if !ok_status {
        return false;
    }
    let trimmed = body.trim_start();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("<!doctype") || lower.starts_with("<html") {
        return false;
    }
    true
}

pub async fn is_dsh_running(port: u16) -> bool {
    let client = loopback_http_client(Duration::from_secs(2)).ok(); // 将 Result 转为 Option

    let client = match client {
        Some(c) => c,
        None => return false,
    };

    let url = format!("{}/", crate::config::get_dsh_service_url(port));

    let check_status = async {
        let resp = client.get(&url).send().await.ok()?;
        if resp.status() != reqwest::StatusCode::OK {
            return None;
        }
        Some(true)
    };

    check_status.await.unwrap_or(false)
}

pub fn wait_for_port_release(port: u16, budget: Duration) -> bool {
    let deadline = std::time::Instant::now() + budget;
    loop {
        if !is_port_in_use(port) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// `0.0.0.0`（局域网访问开关，issue #1021）时桌面端永远判定“未监听”，启动
pub fn is_port_in_use(port: u16) -> bool {
    [Ipv4Addr::LOCALHOST, Ipv4Addr::UNSPECIFIED]
        .iter()
        .any(|ip| TcpListener::bind(SocketAddr::new(IpAddr::V4(*ip), port)).is_err())
}

#[cfg(test)]
thread_local! {
    static OUTPUT_READER_FAILURE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn with_output_reader_failure<T>(reader: usize, task: impl FnOnce() -> T) -> T {
    struct Reset(usize);
    impl Drop for Reset {
        fn drop(&mut self) {
            OUTPUT_READER_FAILURE.set(self.0);
        }
    }
    let _reset = Reset(OUTPUT_READER_FAILURE.replace(reader));
    task()
}

#[cfg(test)]
fn check_output_reader_spawn() -> io::Result<()> {
    if OUTPUT_READER_FAILURE.with(|failure| {
        let remaining = failure.get();
        failure.set(remaining.saturating_sub(1));
        remaining == 1
    }) {
        return Err(io::Error::other("injected output reader spawn failure"));
    }
    Ok(())
}

pub fn spawn_output_readers<R1, R2>(
    stdout: Option<R1>,
    stderr: Option<R2>,
    log_path: PathBuf,
) -> io::Result<mpsc::Receiver<bool>>
where
    R1: Read + Send + 'static,
    R2: Read + Send + 'static,
{
    if let Some(stdout) = stdout {
        let log_path = log_path.clone();
        #[cfg(test)]
        check_output_reader_spawn()?;
        thread::Builder::new().spawn(move || {
            drain_subprocess_output(BufReader::new(stdout), log_path, log::Level::Info, |_| {});
        })?;
    }

    let (signature, receiver) = mpsc::channel();
    if let Some(stderr) = stderr {
        #[cfg(test)]
        check_output_reader_spawn()?;
        thread::Builder::new().spawn(move || {
            let mut reported = false;
            drain_subprocess_output(BufReader::new(stderr), log_path, log::Level::Warn, |line| {
                if !reported && line.contains("duplicate loader entry") {
                    reported = true;
                    let _ = signature.send(true);
                }
            });
            let _ = signature.send(false);
        })?;
    }
    Ok(receiver)
}

fn drain_subprocess_output<R: Read + Send + 'static>(
    mut reader: BufReader<R>,
    log_path: PathBuf,
    level: log::Level,
    mut on_line: impl FnMut(&str),
) {
    loop {
        let mut bytes = Vec::new();
        match reader.read_until(b'\n', &mut bytes) {
            Ok(0) => break, // EOF
            Ok(_) => {
                let bytes = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
                let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
                let line = decode_process_line(bytes);
                on_line(&line);
                match level {
                    log::Level::Warn => log::warn!(target: "dsh", "{}", line),
                    _ => log::info!(target: "dsh", "{}", line),
                }
                append_log(&log_path, &line);
            }
            Err(e) => {
                log::error!("Failed to read dsh {}: {}", log_level_name(level), e);
                break;
            }
        }
    }
}

fn log_level_name(level: log::Level) -> &'static str {
    match level {
        log::Level::Warn => "stderr",
        _ => "stdout",
    }
}

pub(super) fn append_log(log_path: &PathBuf, line: &str) {
    let _guard = dsh_log_lock().lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(meta) = std::fs::metadata(log_path) {
        if meta
            .len()
            .saturating_add(line.len() as u64)
            .saturating_add(1)
            > DSH_MAX_LOG_BYTES
        {
            let _ = std::fs::remove_file(indexed_log_path(log_path, DSH_MAX_BACKUPS));
            for i in (1..DSH_MAX_BACKUPS).rev() {
                let from = indexed_log_path(log_path, i);
                let to = indexed_log_path(log_path, i + 1);
                if from.exists() {
                    let _ = std::fs::remove_file(&to);
                    let _ = std::fs::rename(&from, &to);
                }
            }
            if log_path.exists() {
                let _ = std::fs::rename(log_path, indexed_log_path(log_path, 1));
            }
        }
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let _ = writeln!(file, "{}", line);
        let _ = file.flush();
    }
}

fn indexed_log_path(log_path: &Path, index: usize) -> PathBuf {
    if index == 0 {
        return log_path.to_path_buf();
    }
    let mut name = log_path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}", index));
    log_path.with_file_name(name)
}

pub fn rotate_service_log(log_path: &PathBuf, keep: usize) {
    if keep == 0 {
        let _ = std::fs::remove_file(log_path);
        return;
    }
    let _ = std::fs::remove_file(indexed_log_path(log_path, keep - 1));
    for i in (1..keep).rev() {
        let from = indexed_log_path(log_path, i);
        let to = indexed_log_path(log_path, i + 1);
        if from.exists() {
            let _ = std::fs::remove_file(&to);
            let _ = std::fs::rename(&from, &to);
        }
    }
    if log_path.exists() {
        let _ = std::fs::rename(log_path, indexed_log_path(log_path, 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &PathBuf, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn drain_keeps_reading_after_invalid_utf8_lines() {
        let dir = std::env::temp_dir().join(format!("dsh_drain_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let log = dir.join("out.log");

        let mut data = b"first line\n".to_vec();
        data.extend_from_slice(b"\xd2\xf2\xce\xaa\xb2\xe2\xca\xd4\n");
        data.extend_from_slice(b"second line\n");

        drain_subprocess_output(
            std::io::BufReader::new(std::io::Cursor::new(data)),
            log.clone(),
            log::Level::Warn,
            |_| {},
        );

        let content = fs::read_to_string(&log).unwrap();
        assert!(
            content.contains("first line"),
            "first line must be logged, got: {content:?}"
        );
        assert!(
            content.contains("second line"),
            "reader must NOT stop at invalid UTF-8 (old code broke and closed the pipe); got: {content:?}"
        );
        #[cfg(windows)]
        assert!(
            !content.contains('\u{FFFD}'),
            "Windows 上非法 UTF-8 必须走 ANSI 代码页解码，got: {content:?}"
        );
        #[cfg(not(windows))]
        assert!(content.contains('\u{FFFD}'));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn log_level_name_maps_streams() {
        assert_eq!(log_level_name(log::Level::Warn), "stderr");
        assert_eq!(log_level_name(log::Level::Info), "stdout");
        assert_eq!(log_level_name(log::Level::Debug), "stdout");
    }

    #[test]
    fn drain_strips_line_endings() {
        let dir = std::env::temp_dir().join(format!("dsh_drain_crlf_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let log = dir.join("out.log");

        drain_subprocess_output(
            std::io::BufReader::new(std::io::Cursor::new(b"a\r\nb\n".to_vec())),
            log.clone(),
            log::Level::Info,
            |_| {},
        );

        let content = fs::read_to_string(&log).unwrap();
        assert_eq!(content, "a\nb\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn output_readers_report_only_this_attempts_stderr_signature() {
        let root = tempfile::tempdir().unwrap();
        let log = root.path().join("dsh-web.log");
        let mut data = b"duplicate loader entry\n".to_vec();
        data.extend_from_slice(&vec![b'x'; 256 * 1024]);
        data.extend_from_slice(b"\nlast stderr line\n");
        let duplicate = spawn_output_readers(
            Some(std::io::Cursor::new(b"stdout ready\n".to_vec())),
            Some(std::io::Cursor::new(data)),
            log.clone(),
        )
        .unwrap();
        let timeout = Duration::from_secs(5);
        assert!(duplicate.recv_timeout(timeout).unwrap());
        assert!(!duplicate.recv_timeout(timeout).unwrap());
        let content = fs::read_to_string(&log).unwrap();
        assert!(content.contains("last stderr line\n"));

        let clean = spawn_output_readers(
            Some(std::io::Cursor::new(b"duplicate loader entry\n".to_vec())),
            Some(std::io::Cursor::new(b"unrelated error\n".to_vec())),
            log,
        )
        .unwrap();
        assert!(!clean.recv_timeout(timeout).unwrap());
    }

    #[test]
    fn append_keeps_the_new_line_in_current_log_when_size_limit_is_reached() {
        let root = tempfile::tempdir().unwrap();
        let log = root.path().join("dsh-web.log");
        let previous = vec![b'x'; 5 * 1024 * 1024];
        fs::write(&log, &previous).unwrap();
        let marker = "ERROR HARNESS_FATAL_REPORT: C:/logs/harness-fatal-42.json";

        append_log(&log, marker);

        assert_eq!(fs::read_to_string(&log).unwrap(), format!("{marker}\n"));
        assert_eq!(
            fs::read(root.path().join("dsh-web.log.1")).unwrap(),
            previous
        );
        append_log(&log, "next output");
        assert_eq!(
            fs::read_to_string(&log).unwrap(),
            format!("{marker}\nnext output\n")
        );
    }

    #[test]
    fn rotate_keeps_only_last_three_starts() {
        let dir = std::env::temp_dir().join(format!("dsh_rotate_test_{}", std::process::id()));
        let log = dir.join("dsh-web.log");
        let _ = fs::remove_dir_all(&dir);

        for i in 0..5 {
            write(&log, &format!("start {i} content\n"));
            rotate_service_log(&log, 3);
            assert_eq!(fs::read_to_string(&log).unwrap_or_default(), "");
            assert!(!dir.join("dsh-web.log.3").exists());
            assert!(!dir.join("dsh-web.log.4").exists());
        }

        assert_eq!(fs::read_to_string(&log).unwrap_or_default(), "");
        assert!(fs::read_to_string(dir.join("dsh-web.log.1"))
            .unwrap()
            .contains("start 4"));
        assert!(fs::read_to_string(dir.join("dsh-web.log.2"))
            .unwrap()
            .contains("start 3"));
        assert!(!dir.join("dsh-web.log.3").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn boot_html_uses_declared_client_graph() {
        let html = r#"<script src="/plugins/@deepseek-ai/dsh-client-modules/client.js?rev=one"></script><script>globalThis["__DSH_BOOT__"] = {"rev":"graph","entries":[{"id":"@deepseek-ai/dsh-client-ui-layout","url":"/plugins/@deepseek-ai/dsh-client-ui-layout/client.js?rev=two","rev":"two"}]}</script>"#;
        let urls = client_urls_from_boot_html(3099, html).expect("boot graph");
        assert_eq!(urls.len(), 2);
        assert!(urls
            .iter()
            .any(|url| url.contains("dsh-client-modules/client.js")));
        assert!(urls
            .iter()
            .any(|url| url.contains("dsh-client-ui-layout/client.js")));
        assert!(urls
            .iter()
            .all(|url| url.starts_with("http://127.0.0.1:3099/plugins/")));
    }

    #[test]
    fn boot_html_rejects_missing_or_external_graph_entries() {
        assert!(client_urls_from_boot_html(3099, "<html></html>").is_none());
        let html = r#"<script>globalThis[\"__DSH_BOOT__\"] = {\"entries\":[{\"url\":\"https://example.test/client.js\"}]}</script>"#;
        assert!(client_urls_from_boot_html(3099, html).is_none());
    }

    #[test]
    fn boot_html_decodes_combo_bundle_attribute_url() {
        let html = r#"<script src="/plugins/??@deepseek-ai/dsh-client-modules/client.js&amp;rev=cddf5581d5d5"></script><script>globalThis["__DSH_BOOT__"] = {"entries":[]}</script>"#;
        let urls = client_urls_from_boot_html(3081, html).expect("boot graph");
        assert_eq!(urls, vec!["http://127.0.0.1:3081/plugins/??@deepseek-ai/dsh-client-modules/client.js&rev=cddf5581d5d5"]);
    }

    #[test]
    fn boot_html_accepts_relative_bundle_paths() {
        let html = r#"<script src="plugins/??@deepseek-ai/dsh-client-modules/client.js&amp;rev=f584d378985b"></script><script>globalThis["__DSH_BOOT__"] = {"rev":"g","entries":[{"id":"@deepseek-ai/dsh-client-ui-layout","url":"plugins/??@deepseek-ai/dsh-client-ui-layout/client.js&rev=f26b875a92b6","rev":"f26b875a92b6"}]}</script>"#;
        let urls = client_urls_from_boot_html(3081, html).expect("boot graph");
        assert_eq!(urls.len(), 2);
        assert!(urls
            .iter()
            .all(|url| url.starts_with("http://127.0.0.1:3081/plugins/")));
        assert!(urls
            .iter()
            .any(|url| url.contains("dsh-client-modules/client.js&rev=f584d378985b")));
        assert!(urls
            .iter()
            .any(|url| url.contains("dsh-client-ui-layout/client.js&rev=f26b875a92b6")));
    }

    #[test]
    fn wait_for_port_release_bounds_the_wait_and_returns_as_soon_as_the_port_frees() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let started = std::time::Instant::now();
        assert!(!wait_for_port_release(port, Duration::from_millis(200)));
        assert!(
            started.elapsed() >= Duration::from_millis(200),
            "an occupied port must be waited on until the budget runs out"
        );
        drop(listener);
        let started = std::time::Instant::now();
        assert!(wait_for_port_release(port, Duration::from_millis(200)));
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "a released port must not be waited on"
        );
    }

    /// 回归（issue #1021）：Windows 上具体地址与通配地址的绑定互不冲突
    #[test]
    fn is_port_in_use_detects_wildcard_and_loopback_listeners() {
        for bind_addr in ["127.0.0.1:0", "0.0.0.0:0"] {
            let listener = TcpListener::bind(bind_addr).unwrap();
            let port = listener.local_addr().unwrap().port();
            assert!(
                is_port_in_use(port),
                "a listener on {bind_addr} must read as occupying port {port}"
            );
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(!is_port_in_use(port), "a released port must read as free");
    }

    #[test]
    fn health_probe_plugin_urls_target_client_bundles_not_spa_root() {
        let urls = health_probe_plugin_urls(3080);
        assert!(urls.iter().all(|u| u.contains("/plugins/")));
        assert!(urls
            .iter()
            .all(|u| !u.ends_with("3080/") && !u.ends_with("://127.0.0.1:3080")));
        assert!(urls
            .iter()
            .any(|u| u.contains("dsh-client-ui-layout/client.js")));
        assert!(urls.iter().all(|u| !u.contains("dsh-client-runtime")));
        assert!(urls
            .iter()
            .any(|u| u.contains("dsh-client-modules/client.js")));
    }

    #[test]
    fn spa_html_fallback_is_not_a_plugin_bundle() {
        assert!(!looks_like_plugin_bundle(
            true,
            "<!doctype html><html lang=\"en\"><body>HARNESS Loading plugins...</body></html>"
        ));
        assert!(!looks_like_plugin_bundle(
            true,
            "<html><head></head></html>"
        ));
        assert!(!looks_like_plugin_bundle(true, "   "));
        assert!(!looks_like_plugin_bundle(
            false,
            "window.__ModuleLoader__={}"
        ));
        assert!(looks_like_plugin_bundle(
            true,
            "window.__ModuleLoader__.load({id:\"@deepseek-ai/dsh-client-ui-layout\"})"
        ));
    }

    #[test]
    fn rotate_with_keep_zero_removes_all() {
        let dir = std::env::temp_dir().join(format!("dsh_rotate_zero_{}", std::process::id()));
        let log = dir.join("dsh-web.log");
        let _ = fs::remove_dir_all(&dir);
        write(&log, "x");
        write(&dir.join("dsh-web.log.1"), "x");
        rotate_service_log(&log, 0);
        assert!(!log.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
