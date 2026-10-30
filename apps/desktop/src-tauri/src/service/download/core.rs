use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::AppHandle;
use crate::service::download::ProgressTracker;
use tauri::{Manager, Runtime};

pub async fn download_file<'a, R: Runtime>(
    tracker: &'a ProgressTracker<'a, R>,
    url: String,
) -> Result<Vec<u8>, String> {
    download_file_from_sources(tracker, vec![url]).await
}

pub async fn download_file_from_sources<'a, R: Runtime>(
    tracker: &'a ProgressTracker<'a, R>,
    urls: Vec<String>,
) -> Result<Vec<u8>, String> {
    if urls.is_empty() {
        return Err("DOWNLOAD_URL_EMPTY: no download source provided".to_string());
    }
    let mut last_err = String::new();
    for (index, url) in urls.iter().enumerate() {
        if index > 0 {
            let host = reqwest::Url::parse(url)
                .ok()
                .and_then(|parsed| parsed.host_str().map(|h| h.to_string()))
                .unwrap_or_else(|| url.clone());
            log::warn!(
                "Primary download source failed, switching to fallback source: {}",
                url
            );
            tracker.update(
                0.0,
                format!("主下载源不可用，已切换镜像源重试（{host}）"),
                format!("Switch to fallback download source: {url}"),
            );
        }
        match download_with_retry(tracker, url).await {
            Ok(buffer) => return Ok(buffer),
            Err(e) => last_err = e,
        }
    }
    Err(if urls.len() > 1 {
        format!("{last_err}（已尝试 {} 个下载源）", urls.len())
    } else {
        last_err
    })
}

async fn download_with_retry<'a, R: Runtime>(
    tracker: &'a ProgressTracker<'a, R>,
    url: &str,
) -> Result<Vec<u8>, String> {
    log::info!("Starting file download: {}", url);
    validate_download_url(url)?;

    tracker.update(0.0, String::new(), format!("Download {}", url));

    let client = crate::config::proxy::http_client_builder(tracker.window.app_handle())?
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (deepseek-harness-desktop)")
        .connect_timeout(std::time::Duration::from_secs(20))
        .read_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| {
            log::error!("Failed to create HTTP client: {}", e);
            e.to_string()
        })?;

    const MAX_DOWNLOAD_ATTEMPTS: usize = 5;
    let mut buffer: Vec<u8> = Vec::new();
    for attempt in 1..=MAX_DOWNLOAD_ATTEMPTS {
        if attempt > 1 {
            let delay = Duration::from_secs((1 << (attempt - 1)).min(8));
            log::warn!(
                "Download attempt {}/{} failed, retrying in {}s (resume from {} bytes)",
                attempt - 1,
                MAX_DOWNLOAD_ATTEMPTS,
                delay.as_secs(),
                buffer.len()
            );
            tokio::time::sleep(delay).await;
        }
        match download_attempt(
            &client,
            tracker,
            url,
            attempt,
            MAX_DOWNLOAD_ATTEMPTS,
            &mut buffer,
        )
        .await
        {
            Ok(()) => {
                log::info!("Download completed, {} bytes total", buffer.len());
                return Ok(buffer);
            }
            Err(e) => {
                log::warn!(
                    "Download attempt {}/{} failed: {}",
                    attempt,
                    MAX_DOWNLOAD_ATTEMPTS,
                    e
                );
            }
        }
    }
    Err(format!(
        "DOWNLOAD_INTERRUPTED: 下载中断（网络传输被重置），已自动重试 {MAX_DOWNLOAD_ATTEMPTS} 次仍失败，已下载约 {:.1} MB，请检查网络后重试",
        buffer.len() as f64 / 1_000_000.0
    ))
}

fn download_progress_percent(received_total: u64, total_size: u64) -> f64 {
    if total_size > 0 {
        (received_total as f64 / total_size as f64) * 100.0
    } else {
        -1.0
    }
}

async fn download_attempt<'a, R: Runtime>(
    client: &reqwest::Client,
    tracker: &'a ProgressTracker<'a, R>,
    url: &str,
    attempt: usize,
    max_attempts: usize,
    buffer: &mut Vec<u8>,
) -> Result<(), String> {
    let resume_from = buffer.len() as u64;
    log::debug!(
        "Download attempt {}/{}: {} (resume from {})",
        attempt,
        max_attempts,
        url,
        resume_from
    );

    let mut req = client.get(url);
    if resume_from > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
    }
    let res = req.send().await.map_err(|e| {
        log::error!("Download request failed: {}", e);
        e.to_string()
    })?;
    validate_download_url(res.url().as_str())?;

    if res.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        log::warn!("Server returned 416 for range request, restarting from zero");
        buffer.clear();
        return Err(format!("Download failed: HTTP {}", res.status()));
    }
    if !res.status().is_success() {
        log::error!("Download failed with HTTP status: {}", res.status());
        return Err(format!("Download failed: HTTP {}", res.status()));
    }

    let range_accepted = res.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if !range_accepted && resume_from > 0 {
        log::warn!("Server ignored Range request, restarting download from zero");
        buffer.clear();
    }

    let total_size = if range_accepted {
        resume_from + res.content_length().unwrap_or(0)
    } else {
        res.content_length().unwrap_or(0)
    };
    log::debug!("File size: {} bytes", total_size);
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            log::error!("Download stream read error: {}", e);
            e.to_string()
        })?;
        buffer.extend_from_slice(&chunk);
        let received_total = buffer.len() as u64;
        let progress_pct = download_progress_percent(received_total, total_size);
        tracker.update(
            progress_pct,
            format!(
                "已下载 {:.1} MB / {:.1} MB",
                received_total as f64 / 1_000_000.0,
                total_size as f64 / 1_000_000.0
            ),
            String::new(),
        );
    }

    log::info!(
        "Download attempt {}/{} succeeded, {} bytes in buffer",
        attempt,
        max_attempts,
        buffer.len()
    );
    Ok(())
}

fn validate_download_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("DOWNLOAD_URL_INVALID: {e}"))?;
    let trusted_host = matches!(
        parsed.host_str(),
        Some(
            "nodejs.org"
                | "registry.npmjs.org"
                | "github.com"
                | "release-assets.githubusercontent.com"
                | "objects.githubusercontent.com"
                | "npmmirror.com"
                | "cdn.npmmirror.com"
                | "registry.npmmirror.com"
                | "gh-proxy.com"
                | "gh.llkk.cc"
                | "ghfast.top"
                | "ghproxy.net"
        )
    );
    if parsed.scheme() != "https" || !trusted_host {
        return Err(format!("DOWNLOAD_SOURCE_UNTRUSTED: {url}"));
    }
    Ok(())
}

pub fn verify_sha256(buffer: &[u8], expected: &str) -> Result<(), String> {
    let expected = expected
        .strip_prefix("sha256:")
        .unwrap_or(expected)
        .trim()
        .to_ascii_lowercase();
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("INTEGRITY_METADATA_INVALID: expected SHA-256 is invalid".to_string());
    }
    let actual = format!("{:x}", Sha256::digest(buffer));
    if actual != expected {
        return Err(format!(
            "INTEGRITY_CHECK_FAILED: SHA-256 mismatch, expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

pub async fn fetch_node_sha256(
    app_handle: &AppHandle,
    download_url: &str,
) -> Result<String, String> {
    let (base, filename) = download_url.rsplit_once('/').ok_or_else(|| {
        "INTEGRITY_METADATA_INVALID: Node.js download URL has no filename".to_string()
    })?;
    let checksums_url = format!("{base}/SHASUMS256.txt");
    let checksums = crate::config::proxy::http_client_builder(app_handle)?
        .user_agent("deepseek-harness-desktop")
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| format!("INTEGRITY_METADATA_FAILED: {e}"))?
        .get(&checksums_url)
        .send()
        .await
        .map_err(|e| format!("INTEGRITY_METADATA_FAILED: {e}"))?
        .error_for_status()
        .map_err(|e| format!("INTEGRITY_METADATA_FAILED: {e}"))?
        .text()
        .await
        .map_err(|e| format!("INTEGRITY_METADATA_FAILED: {e}"))?;

    checksums
        .lines()
        .filter_map(|line| line.split_once(char::is_whitespace))
        .find_map(|(digest, name)| {
            (name.trim_start_matches([' ', '*']) == filename).then(|| digest.to_string())
        })
        .ok_or_else(|| format!("INTEGRITY_METADATA_MISSING: no checksum for {filename}"))
}

pub(crate) async fn remove_dir_with_retry(dest: &Path) -> bool {
    const MAX_ATTEMPTS: u32 = 40;
    const RETRY_DELAY: Duration = Duration::from_millis(250);

    for attempt in 1..=MAX_ATTEMPTS {
        match fs::remove_dir_all(dest) {
            Ok(()) => return true,
            Err(e) => {
                if attempt < MAX_ATTEMPTS {
                    log::warn!(
                        "Failed to clean {:?} (attempt {}/{}), file may be locked: {}",
                        dest,
                        attempt,
                        MAX_ATTEMPTS,
                        e
                    );
                    tokio::time::sleep(RETRY_DELAY).await;
                } else {
                    log::error!(
                        "Failed to clean {:?} after {} attempts: {}",
                        dest,
                        MAX_ATTEMPTS,
                        e
                    );
                }
            }
        }
    }
    false
}

async fn remove_path_if_exists(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        if remove_dir_with_retry(path).await {
            Ok(())
        } else {
            Err(format!(
                "INSTALL_PATH_LOCKED: cannot remove {}",
                path.display()
            ))
        }
    } else {
        fs::remove_file(path)
            .map_err(|e| format!("INSTALL_PATH_REMOVE_FAILED: {}: {e}", path.display()))
    }
}

pub(crate) async fn rename_with_retry(from: &Path, to: &Path) -> Result<(), std::io::Error> {
    const MAX_ATTEMPTS: u32 = 60;
    const RETRY_DELAY: Duration = Duration::from_millis(500);

    for attempt in 1..=MAX_ATTEMPTS {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if attempt < MAX_ATTEMPTS => {
                log::warn!(
                    "Rename {:?} -> {:?} failed (attempt {attempt}/{MAX_ATTEMPTS}), file may be locked: {e}",
                    from,
                    to
                );
                tokio::time::sleep(RETRY_DELAY).await;
            }
            Err(e) if e.raw_os_error() == Some(32) => {
                return Err(std::io::Error::new(
                    e.kind(),
                    format!(
                        "{e}; end node.exe processes still using the core directory in Task Manager or restart Windows, then retry"
                    ),
                ));
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!("rename_with_retry loop always returns")
}

async fn commit_staged_install(staging: &Path, dest: &Path, backup: &Path) -> Result<(), String> {
    if !dest.exists() && backup.exists() {
        rename_with_retry(backup, dest).await.map_err(|e| {
            format!(
                "INSTALL_RECOVERY_FAILED: {} -> {}: {e}",
                backup.display(),
                dest.display()
            )
        })?;
    }
    remove_path_if_exists(backup).await?;
    let had_previous = dest.exists();
    if had_previous {
        rename_with_retry(dest, backup).await.map_err(|e| {
            format!(
                "INSTALL_BACKUP_FAILED: {} -> {}: {e}",
                dest.display(),
                backup.display()
            )
        })?;
    }
    if let Err(e) = rename_with_retry(staging, dest).await {
        if had_previous {
            let _ = fs::rename(backup, dest);
        }
        return Err(format!(
            "INSTALL_COMMIT_FAILED: {} -> {}: {e}",
            staging.display(),
            dest.display()
        ));
    }
    if had_previous {
        if let Err(e) = remove_path_if_exists(backup).await {
            log::warn!("Failed to remove previous installation backup: {e}");
        }
    }
    Ok(())
}

pub async fn ensure_extract<'a, R: Runtime>(
    tracker: &'a ProgressTracker<'a, R>,
    name: String,
    buffer: Vec<u8>,
    dest: PathBuf,
) -> Result<(), String> {
    log::info!("Starting file extraction: {} -> {:?}", name, dest);
    use super::extractor::{extract_tgz, extract_zip};
    use super::utils::flatten_directory;

    let parent = dest.parent().unwrap_or(Path::new("."));
    let leaf = dest
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("package");
    let staging = parent.join(format!(".{leaf}.installing-{}", std::process::id()));
    let backup = parent.join(format!(".{leaf}.backup"));
    remove_path_if_exists(&staging).await?;

    let pure_name = name.split('?').next().unwrap_or(&name).to_lowercase();
    let is_tgz = pure_name.ends_with(".tar.gz") || pure_name.ends_with(".tgz");
    let is_asar_zip = pure_name.ends_with(".asar.zip");
    let is_zip = pure_name.ends_with(".zip");
    log::debug!(
        "File type: tgz={}, zip={}, asar_zip={}",
        is_tgz,
        is_zip,
        is_asar_zip
    );

    if !is_tgz && !is_zip {
        log::debug!("Non-compressed file, writing directly");
        if let Some(parent) = staging.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                log::error!("Failed to create parent directory: {}", e);
                e.to_string()
            })?;
        }
        fs::write(&staging, &buffer).map_err(|e| {
            log::error!("Failed to write file: {}", e);
            e.to_string()
        })?;
        tracker.update(
            100.0,
            format!("已写入: {}", "100%"),
            format!("File written: {}", staging.display()),
        );
        commit_staged_install(&staging, &dest, &backup).await?;
        log::info!("File write completed: {}", dest.display());
        return Ok(());
    }

    fs::create_dir_all(&staging).map_err(|e| {
        log::error!("Failed to create destination directory: {}", e);
        e.to_string()
    })?;

    if is_tgz {
        log::debug!("Using tgz extractor");
        extract_tgz(tracker, &buffer, &staging)?;
    } else {
        log::debug!("Using zip extractor");
        extract_zip(tracker, &buffer, &staging)?;
    }

    log::debug!("Flattening directory structure");
    flatten_directory(&staging).map_err(|e| {
        log::error!("Failed to flatten directory: {}", e);
        e.to_string()
    })?;

    if is_asar_zip {
        let archive = super::utils::normalize_asar_name(&staging)?;
        super::asar::validate_dsh_archive(&archive)?;
    }

    #[cfg(unix)]
    {
        use super::utils::fix_recursive_permissions;
        log::debug!("Fixing file permissions");
        fix_recursive_permissions(&staging).map_err(|e| {
            log::error!("Failed to fix permissions: {}", e);
            format!("Failed to fix permissions: {}", e)
        })?;

        #[cfg(target_os = "macos")]
        {
            use std::process::Command;
            log::debug!("Removing macOS quarantine attribute");
            if let Some(path_str) = staging.to_str() {
                let _ = Command::new("xattr").args(["-cr", path_str]).output();
            }
        }
    }

    commit_staged_install(&staging, &dest, &backup).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_percent_never_emits_nan_or_inf() {
        assert_eq!(download_progress_percent(50, 100), 50.0);
        assert_eq!(download_progress_percent(0, 100), 0.0);
        assert_eq!(download_progress_percent(50, 0), -1.0);
        assert_eq!(download_progress_percent(0, 0), -1.0);
        for pct in [
            download_progress_percent(50, 0),
            download_progress_percent(0, 0),
        ] {
            assert!(pct.is_finite(), "progress must be finite, got {pct}");
        }
    }

    #[test]
    fn sha256_verification_accepts_only_matching_digest() {
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify_sha256(b"abc", expected).is_ok());
        assert!(verify_sha256(b"changed", expected).is_err());
        assert!(verify_sha256(b"abc", "not-a-digest").is_err());
    }

    #[test]
    fn download_sources_are_https_and_allowlisted() {
        assert!(validate_download_url("https://nodejs.org/dist/v22/file.zip").is_ok());
        assert!(validate_download_url("https://registry.npmjs.org/pnpm/-/pnpm.tgz").is_ok());
        assert!(validate_download_url(
            "https://github.com/git-for-windows/git/releases/download/v2.53.0.windows.2/MinGit-2.53.0.2-64-bit.zip"
        )
        .is_ok());
        assert!(validate_download_url("http://nodejs.org/dist/file.zip").is_err());
        assert!(validate_download_url("https://example.com/file.zip").is_err());
        assert!(validate_download_url("https://npmmirror.com/mirrors/node/v22/file.zip").is_ok());
        assert!(
            validate_download_url("https://cdn.npmmirror.com/binaries/node/v22/file.zip").is_ok()
        );
        assert!(validate_download_url("https://registry.npmmirror.com/pnpm/-/pnpm.tgz").is_ok());
        for prefix in crate::config::DSH_MIRROR_PREFIXES {
            assert!(
                validate_download_url(&format!(
                    "{prefix}https://github.com/dsh-tauri-desk/deepseek-harness-pkg/releases/latest/download/deepseek-harness-pkg-windows.zip"
                ))
                .is_ok(),
                "镜像前缀应被放行: {prefix}"
            );
        }
    }

    #[tokio::test]
    async fn staged_install_replaces_previous_version_and_cleans_backup() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("dsh-atomic-install-{unique}"));
        let dest = root.join("package");
        let staging = root.join("package.installing");
        let backup = root.join("package.backup");
        fs::create_dir_all(&dest).expect("create previous install");
        fs::create_dir_all(&staging).expect("create staged install");
        fs::write(dest.join("version.txt"), "old").expect("write previous version");
        fs::write(staging.join("version.txt"), "new").expect("write staged version");

        commit_staged_install(&staging, &dest, &backup)
            .await
            .expect("commit staged install");

        assert_eq!(fs::read_to_string(dest.join("version.txt")).unwrap(), "new");
        assert!(!staging.exists());
        assert!(!backup.exists());

        fs::rename(&dest, &backup).expect("simulate interrupted switch");
        fs::create_dir_all(&staging).expect("create next staged install");
        fs::write(staging.join("version.txt"), "next").expect("write next version");
        commit_staged_install(&staging, &dest, &backup)
            .await
            .expect("recover and commit");
        assert_eq!(
            fs::read_to_string(dest.join("version.txt")).unwrap(),
            "next"
        );
        assert!(!backup.exists());
        fs::remove_dir_all(root).ok();
    }
}
