use std::fs;
use std::io::Cursor;
use std::io::copy;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::service::download::ProgressTracker;
use tauri::Runtime;

fn create_file_with_retry(outpath: &Path) -> Result<fs::File, String> {
    const MAX_ATTEMPTS: u32 = 40;
    const RETRY_DELAY: Duration = Duration::from_millis(250);

    for attempt in 1..=MAX_ATTEMPTS {
        match fs::File::create(outpath) {
            Ok(file) => return Ok(file),
            Err(e) if matches!(e.raw_os_error(), Some(32 | 5)) => {
                log::warn!(
                    "File {:?} is locked by another process (attempt {}/{})",
                    outpath,
                    attempt,
                    MAX_ATTEMPTS
                );
                std::thread::sleep(RETRY_DELAY);
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Err(format!(
        "File {:?} is still locked after {} attempts",
        outpath, MAX_ATTEMPTS
    ))
}

pub fn extract_zip<'a, R: Runtime>(
    tracker: &ProgressTracker<'a, R>,
    buffer: &[u8],
    dest: &PathBuf,
) -> Result<(), String> {
    log::debug!("Starting ZIP extraction to: {:?}", dest);
    let mut archive = zip::ZipArchive::new(Cursor::new(buffer)).map_err(|e| {
        log::error!("Invalid ZIP format: {}", e);
        format!("Invalid ZIP format: {}", e)
    })?;

    let total_files = archive.len();
    log::debug!("ZIP file contains {} files", total_files);

    for i in 0..total_files {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("ZIP_ENTRY_INVALID: {e}"))?;

        let path = file
            .enclosed_name()
            .ok_or_else(|| format!("ZIP_PATH_INVALID: {}", file.name()))?;
        let outpath = dest.join(path);

        let relative_path = outpath
            .strip_prefix(dest)
            .ok()
            .and_then(|p| p.to_str())
            .map(|s| s.replace('\\', "/"))
            .unwrap_or_else(|| {
                outpath
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            });
        let progress_pct = ((i + 1) as f64 / total_files as f64) * 100.0;
        tracker.update(
            progress_pct,
            format!("已解压 {:.1}%", progress_pct),
            format!("Extract {}", relative_path),
        );

        if file.is_dir() {
            fs::create_dir_all(&outpath).map_err(|e| {
                log::error!("Failed to create directory {:?}: {}", outpath, e);
                e.to_string()
            })?;
        } else {
            if let Some(p) = outpath.parent() {
                fs::create_dir_all(p).map_err(|e| {
                    log::error!(
                        "Failed to create parent directory {:?}: {} (file: {:?})",
                        p,
                        e,
                        outpath
                    );
                    format!("Failed to create parent directory {:?}: {}", p, e)
                })?;
            }
            let mut outfile = create_file_with_retry(&outpath).map_err(|e| {
                log::error!(
                    "Failed to create file {:?}: {} (parent exists: {})",
                    outpath,
                    e,
                    outpath.parent().map(|p| p.exists()).unwrap_or(false)
                );
                format!("Failed to create file {:?}: {}", outpath, e)
            })?;
            copy(&mut file, &mut outfile).map_err(|e| {
                log::error!("Failed to copy file {:?}: {}", outpath, e);
                e.to_string()
            })?;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = file.unix_mode() {
                fs::set_permissions(&outpath, fs::Permissions::from_mode(mode)).ok();
            }
        }
    }
    log::info!("ZIP extraction completed, {} files total", total_files);
    Ok(())
}

pub fn extract_tgz<'a, R: Runtime>(
    tracker: &ProgressTracker<'a, R>,
    buffer: &[u8],
    dest: &PathBuf,
) -> Result<(), String> {
    log::debug!("Starting TGZ extraction to: {:?}", dest);
    use flate2::read::GzDecoder;
    use tar::Archive;

    let tar_gz = GzDecoder::new(Cursor::new(buffer));
    let mut archive = Archive::new(tar_gz);

    let entries = archive.entries().map_err(|e| {
        log::error!("Failed to read TGZ entries: {}", e);
        e.to_string()
    })?;

    let mut file_count = 0;
    for entry_result in entries {
        let mut entry = entry_result.map_err(|e| {
            log::error!("Failed to read TGZ entry: {}", e);
            e.to_string()
        })?;
        let path = entry.path().map(|p| p.to_path_buf()).map_err(|e| {
            log::error!("Failed to get entry path: {}", e);
            e.to_string()
        })?;

        let relative_path = path.to_string_lossy().replace('\\', "/");
        let estimated_pct = (file_count as f64 / (file_count + 1) as f64) * 100.0;
        tracker.update(
            -1.0,
            format!("已解压 {:.1}%", estimated_pct),
            format!("Extract {}", relative_path),
        );

        entry.unpack_in(dest).map_err(|e| {
            log::error!("Failed to unpack entry {:?}: {}", path, e);
            e.to_string()
        })?;
        file_count += 1;
    }

    log::info!("TGZ extraction completed, {} files total", file_count);
    Ok(())
}
