//! ## 读取（文件列表，issue #1160）
//! 的文件条目；而图片回退桥「无图无文本即吞事件」的行为（issue #610 修复引入）

use std::path::{Path, PathBuf};

use crate::AppHandle;
use base64::{Engine, engine::general_purpose::STANDARD};

#[derive(serde::Serialize)]
pub struct ClipboardImageResponse {
    pub data_url: String,
    pub mime: String,
    pub filename: String,
}

#[tauri::command]
pub async fn read_clipboard_image(
    _app: AppHandle,
) -> Result<Option<ClipboardImageResponse>, String> {
    let result =
        tokio::task::spawn_blocking(move || -> Result<Option<ClipboardImageResponse>, String> {
            const MAX_PIXELS: u64 = 50_000_000;

            let mut clipboard =
                arboard::Clipboard::new().map_err(|e| format!("CLIPBOARD_IMAGE_ACCESS: {e}"))?;

            let image_data = match clipboard.get_image() {
                Ok(data) => data,
                Err(arboard::Error::ContentNotAvailable) => return Ok(None),
                Err(e) => return Err(format!("CLIPBOARD_IMAGE_READ: {e}")),
            };

            if image_data.width == 0 || image_data.height == 0 {
                return Ok(None);
            }
            let pixel_count = image_data.width as u64 * image_data.height as u64;
            if pixel_count > MAX_PIXELS {
                return Err(format!(
                    "CLIPBOARD_IMAGE_TOO_LARGE: {}x{} ({} px)",
                    image_data.width, image_data.height, pixel_count
                ));
            }

            let rgba = image::RgbaImage::from_raw(
                image_data.width as u32,
                image_data.height as u32,
                image_data.bytes.into_owned(),
            )
            .ok_or_else(|| "CLIPBOARD_IMAGE_DECODE: invalid rgba buffer".to_string())?;

            let mut cursor = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(rgba)
                .write_to(&mut cursor, image::ImageFormat::Png)
                .map_err(|e| format!("CLIPBOARD_IMAGE_ENCODE: {e}"))?;

            let b64 = STANDARD.encode(cursor.into_inner());
            Ok(Some(ClipboardImageResponse {
                data_url: format!("data:image/png;base64,{b64}"),
                mime: "image/png".to_string(),
                filename: "clipboard-image.png".to_string(),
            }))
        })
        .await
        .map_err(|e| format!("CLIPBOARD_IMAGE_TASK: {e}"))??;

    Ok(result)
}

#[tauri::command]
pub async fn read_clipboard_text() -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || -> Result<Option<String>, String> {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| format!("CLIPBOARD_TEXT_ACCESS: {e}"))?;
        match clipboard.get_text() {
            Ok(text) => Ok(Some(text)),
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(e) => Err(format!("CLIPBOARD_TEXT_READ: {e}")),
        }
    })
    .await
    .map_err(|e| format!("CLIPBOARD_TEXT_TASK: {e}"))?
}

#[tauri::command]
pub async fn write_clipboard_text(text: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| format!("CLIPBOARD_TEXT_ACCESS: {e}"))?;
        clipboard
            .set_text(text)
            .map_err(|e| format!("CLIPBOARD_TEXT_WRITE: {e}"))
    })
    .await
    .map_err(|e| format!("CLIPBOARD_TEXT_TASK: {e}"))?
}

const MAX_CLIPBOARD_FILE_BYTES: u64 = 20 * 1024 * 1024;

const MAX_CLIPBOARD_FILES: usize = 32;

const MAX_CLIPBOARD_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, serde::Serialize)]
pub struct ClipboardFileEntry {
    pub data_url: String,
    pub mime: String,
    pub filename: String,
    pub size: u64,
}

#[derive(Debug, serde::Serialize)]
pub struct ClipboardFileRejection {
    pub filename: String,
    pub reason: String,
}

#[derive(Debug, serde::Serialize)]
pub struct ClipboardFilesResponse {
    pub files: Vec<ClipboardFileEntry>,
    pub rejected: Vec<ClipboardFileRejection>,
}

fn guess_mime_by_extension(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "txt" | "md" | "log" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn clipboard_filename(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("clipboard-file")
        .to_string()
}

/// 并静默失败（issue #1160 实测复现）。这里剥掉行尾 `\r`（以及可能的空白）后再用。
fn sanitize_uri_path(path: PathBuf) -> PathBuf {
    let Some(raw) = path.to_str() else {
        return path;
    };
    let trimmed = raw.trim_end_matches(['\r', '\n']);
    if trimmed.len() == raw.len() {
        path
    } else {
        PathBuf::from(trimmed)
    }
}

fn read_one_clipboard_file(
    path: &Path,
    remaining: &mut u64,
) -> Result<ClipboardFileEntry, ClipboardFileRejection> {
    let reject = |reason: &str| ClipboardFileRejection {
        filename: clipboard_filename(path),
        reason: reason.to_string(),
    };

    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return Err(reject("unreadable")),
    };
    if !metadata.is_file() {
        return Err(reject("not_a_file"));
    }
    if metadata.len() > MAX_CLIPBOARD_FILE_BYTES {
        return Err(reject("too_large"));
    }
    if metadata.len() > *remaining {
        return Err(reject("total_exceeded"));
    }

    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return Err(reject("unreadable")),
    };
    *remaining -= metadata.len();

    let mime = guess_mime_by_extension(path);
    let b64 = STANDARD.encode(bytes);
    Ok(ClipboardFileEntry {
        data_url: format!("data:{mime};base64,{b64}"),
        mime,
        size: metadata.len(),
        filename: clipboard_filename(path),
    })
}

fn collect_clipboard_files(paths: Vec<PathBuf>) -> ClipboardFilesResponse {
    let mut files = Vec::new();
    let mut rejected = Vec::new();
    let mut remaining = MAX_CLIPBOARD_TOTAL_BYTES;
    for path in paths.into_iter().take(MAX_CLIPBOARD_FILES) {
        match read_one_clipboard_file(&path, &mut remaining) {
            Ok(entry) => files.push(entry),
            Err(deny) => rejected.push(deny),
        }
    }
    ClipboardFilesResponse { files, rejected }
}

/// 从系统剪贴板读取文件列表并逐个编码为 data URL（issue #1160，贴文件回退）。
#[tauri::command]
pub async fn read_clipboard_files() -> Result<ClipboardFilesResponse, String> {
    tokio::task::spawn_blocking(|| -> Result<ClipboardFilesResponse, String> {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| format!("CLIPBOARD_FILES_ACCESS: {e}"))?;

        let paths = match clipboard.get().file_list() {
            Ok(paths) => paths,
            Err(arboard::Error::ContentNotAvailable) => Vec::new(),
            Err(arboard::Error::ClipboardNotSupported) => Vec::new(),
            Err(e) => return Err(format!("CLIPBOARD_FILES_READ: {e}")),
        };

        // 诊断：剪贴板里到底有没有文件、arboard 解出什么（CRLF 残留是 issue #1160 的
        log::info!("[clipboard-files] arboard returned {} path(s)", paths.len());
        let paths: Vec<PathBuf> = paths.into_iter().map(sanitize_uri_path).collect();

        let response = collect_clipboard_files(paths);
        if !response.rejected.is_empty() {
            log::warn!(
                "[clipboard-files] {} file(s) rejected: {:?}",
                response.rejected.len(),
                response
                    .rejected
                    .iter()
                    .map(|r| format!("{}={}", r.filename, r.reason))
                    .collect::<Vec<_>>()
            );
        }

        Ok(response)
    })
    .await
    .map_err(|e| format!("CLIPBOARD_FILES_TASK: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_one_clipboard_file_encodes_with_guessed_mime() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        const PAYLOAD: &[u8] = b"\xff\xd8\xff\xe0 fake jpeg";
        std::fs::write(&path, PAYLOAD).unwrap();

        let mut remaining = MAX_CLIPBOARD_TOTAL_BYTES;
        let entry = read_one_clipboard_file(&path, &mut remaining).unwrap();
        assert_eq!(entry.filename, "photo.jpg");
        assert_eq!(entry.mime, "image/jpeg");
        assert!(entry.data_url.starts_with("data:image/jpeg;base64,"));
        assert_eq!(entry.size, PAYLOAD.len() as u64);
        assert_eq!(remaining, MAX_CLIPBOARD_TOTAL_BYTES - PAYLOAD.len() as u64);
    }

    #[test]
    fn read_one_clipboard_file_rejects_directory_as_not_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut remaining = MAX_CLIPBOARD_TOTAL_BYTES;
        let deny = read_one_clipboard_file(dir.path(), &mut remaining).unwrap_err();
        assert_eq!(deny.reason, "not_a_file");
    }

    #[test]
    fn read_one_clipboard_file_rejects_missing_path_as_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone.bin");
        let mut remaining = MAX_CLIPBOARD_TOTAL_BYTES;
        let deny = read_one_clipboard_file(&missing, &mut remaining).unwrap_err();
        assert_eq!(deny.reason, "unreadable");
    }

    #[test]
    fn read_one_clipboard_file_rejects_oversize_without_reading_body() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_CLIPBOARD_FILE_BYTES + 1).unwrap();

        let mut remaining = MAX_CLIPBOARD_TOTAL_BYTES;
        let deny = read_one_clipboard_file(&path, &mut remaining).unwrap_err();
        assert_eq!(deny.reason, "too_large");
        assert_eq!(deny.filename, "big.bin");
        assert_eq!(remaining, MAX_CLIPBOARD_TOTAL_BYTES);
    }

    #[test]
    fn read_one_clipboard_file_rejects_when_total_quota_exhausted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ok.txt");
        std::fs::write(&path, b"content").unwrap();

        let mut remaining = 0;
        let deny = read_one_clipboard_file(&path, &mut remaining).unwrap_err();
        assert_eq!(deny.reason, "total_exceeded");
        assert_eq!(deny.filename, "ok.txt");
        assert_eq!(remaining, 0);
    }

    #[test]
    fn collect_clipboard_files_keeps_order_and_quotas_across_batch() {
        let dir = tempfile::tempdir().unwrap();
        let small = dir.path().join("a.txt");
        std::fs::write(&small, b"12345").unwrap();
        let missing = dir.path().join("gone.txt");

        let response = collect_clipboard_files(vec![small.clone(), missing.clone(), small]);
        assert_eq!(response.files.len(), 2);
        assert_eq!(response.files[0].filename, "a.txt");
        assert_eq!(response.rejected.len(), 1);
        assert_eq!(response.rejected[0].reason, "unreadable");
    }

    #[test]
    fn collect_clipboard_files_caps_entry_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.txt");
        std::fs::write(&path, b"x").unwrap();
        let paths = vec![path; MAX_CLIPBOARD_FILES + 10];

        let response = collect_clipboard_files(paths);
        assert_eq!(response.files.len(), MAX_CLIPBOARD_FILES);
        assert!(response.rejected.is_empty());
    }

    #[test]
    fn sanitize_uri_path_strips_crlf_trailer() {
        // 路径尾部会带 \r；不清洗则 metadata() 必然失败（issue #1160 实测根因）
        let raw = PathBuf::from("/media/x/工作簿1.xlsx\r");
        assert_eq!(
            sanitize_uri_path(raw),
            PathBuf::from("/media/x/工作簿1.xlsx")
        );
    }

    #[test]
    fn sanitize_uri_path_leaves_clean_path_untouched() {
        let clean = PathBuf::from("/media/x/工作簿1.xlsx");
        assert_eq!(sanitize_uri_path(clean.clone()), clean);
    }

    #[test]
    fn sanitize_uri_path_keeps_file_readable_after_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.xlsx");
        std::fs::write(&path, b"fake xlsx").unwrap();

        let mut remaining = MAX_CLIPBOARD_TOTAL_BYTES;
        let dirty = PathBuf::from(format!("{}\r", path.display()));
        assert!(read_one_clipboard_file(&dirty, &mut remaining).is_err());

        let mut remaining = MAX_CLIPBOARD_TOTAL_BYTES;
        let entry = read_one_clipboard_file(&sanitize_uri_path(dirty), &mut remaining).unwrap();
        assert_eq!(entry.filename, "report.xlsx");
        assert_eq!(entry.mime, "application/octet-stream");
    }

    #[test]
    fn guess_mime_covers_common_types_and_falls_back() {
        assert_eq!(guess_mime_by_extension(Path::new("a.PNG")), "image/png");
        assert_eq!(guess_mime_by_extension(Path::new("b.md")), "text/plain");
        assert_eq!(
            guess_mime_by_extension(Path::new("c.pdf")),
            "application/pdf"
        );
        assert_eq!(
            guess_mime_by_extension(Path::new("d.unknownext")),
            "application/octet-stream"
        );
        assert_eq!(
            guess_mime_by_extension(Path::new("noext")),
            "application/octet-stream"
        );
    }
}
