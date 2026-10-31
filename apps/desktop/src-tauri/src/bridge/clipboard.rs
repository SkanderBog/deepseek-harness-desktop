//! 原生剪贴板读写（Linux/WebKitGTK 贴图/贴文件回退 + Wayland 安全文本写入）。
//!
//! ## 读取（图片）
//! WebKitGTK 不通过 Web API（`ClipboardEvent.clipboardData.items/files`）暴露
//! `image/*` 剪贴板条目，导致桌面端内嵌的 dsh iframe 中输入框「贴图」无效
//! （浏览器里却正常）。本命令在 Rust 侧用 `arboard` 读取系统剪贴板图片、编码为
//! PNG data URL 返回；注入到 iframe 的桥脚本（`desktop::paste::PASTE_SHIM_JS`）
//! 拿到该 data URL 后重新派发 `paste` 事件，让 dsh 聊天框按正常贴图路径处理。
//!
//! ## 读取（文件列表，issue #1160）
//! 同一族限制：文件管理器复制文件时，剪贴板里是 `text/uri-list` /
//! `x-special/gnome-copied-files`，WebKitGTK 同样不把它映射成 `clipboardData`
//! 的文件条目；而图片回退桥「无图无文本即吞事件」的行为（issue #610 修复引入）
//! 让贴文件比不修更沉默——事件被 `preventDefault`，图片读取又拿不到图。
//! `read_clipboard_files` 在 Rust 侧用 `arboard` 的 `clipboard.get().file_list()`
//! 读出绝对路径，逐个读成带 MIME 的 data URL 回传；桥脚本把结果全部塞进同一个
//! `DataTransfer` 重发合成 `paste`，dsh 输入框的 `intakeFiles` 原样命中。
//!
//! ## 写入（文本）
//! 前端的「复制日志 / 复制服务地址」等操作统一走本模块的写入命令，而不是
//! `tauri-plugin-clipboard-manager`：后者在**应用启动时**就在主线程上创建并持有
//! 一个长生命周期 `arboard::Clipboard`（`init()` 内 `Clipboard::new()`），在
//! Linux Wayland 会话里一旦合成器不支持 `ext-data-control`/`wlr-data-control`
//! （`arboard` 只会打印「Falling back to the X11 clipboard protocol」警告并回退），
//! 该持有实例要么进入脆弱状态、要么在交互式 `set_text` 时阻塞/崩溃（用户反馈
//! 「复制运行日志软件崩溃」）。这里的实现改为**每次操作**在 `spawn_blocking`
//! 中**惰性新建**一个短期 `arboard::Clipboard`，用完即弃：
//! - 不阻塞异步运行时与 UI（Linux 上创建 / 写入需连显示服务器）；
//! - 不复用跨操作的长生命周期句柄，规避 Wayland 回退后的挂死 / 崩溃；
//! - Wayland 不支持时 `arboard` 自动回退 X11，仍是尽力而为，不抛出。

use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine};

/// 剪贴板图片读取结果：自包含的 PNG data URL（可直接作为 Blob/File 来源）。
#[derive(serde::Serialize)]
pub struct ClipboardImageResponse {
    /// 形如 `data:image/png;base64,...`
    pub data_url: String,
    pub mime: String,
    pub filename: String,
}

/// 从系统剪贴板读取图片并编码为 PNG data URL。
///
/// 剪贴板无图片时返回 `Ok(None)`；读取/编码失败返回 `Err`（前缀 `CLIPBOARD_IMAGE_`）。
#[tauri::command]
pub async fn read_clipboard_image(
    _app: tauri::AppHandle,
) -> Result<Option<ClipboardImageResponse>, String> {
    // arboard 的 Clipboard::new()/get_image() 是阻塞调用（Linux 上需连接显示服务器），
    // 放到 blocking 线程避免阻塞异步运行时与 UI。
    let result =
        tokio::task::spawn_blocking(move || -> Result<Option<ClipboardImageResponse>, String> {
            // 超过约 50MP 的剪贴板图片（≈200MB RGBA）直接拒绝，避免撑爆内存
            const MAX_PIXELS: u64 = 50_000_000;

            let mut clipboard =
                arboard::Clipboard::new().map_err(|e| format!("CLIPBOARD_IMAGE_ACCESS: {e}"))?;

            let image_data = match clipboard.get_image() {
                Ok(data) => data,
                // 剪贴板里没有图片（普通文本/文件等），不是错误
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

            // arboard 返回 RGBA8 像素，直接包装为 RgbaImage 后用 image 编码成 PNG
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

/// 从系统剪贴板读取纯文本。
///
/// WKWebView 里的 `navigator.clipboard.readText()` 拿不到授权（macOS 桌面端右键菜单
/// 「粘贴」因此失败），本命令给 iframe 侧一条原生读取路径：与写入一致，在
/// `spawn_blocking` 里惰性新建短期 `arboard::Clipboard` 读取，用完即弃。
/// 剪贴板无文本返回 `Ok(None)`；读取失败返回 `Err`（前缀 `CLIPBOARD_TEXT_`）。
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

/// 把纯文本写入系统剪贴板。
///
/// 实现与 [`read_clipboard_image`] 一致：在 `spawn_blocking` 里**惰性新建**短期
/// `arboard::Clipboard` 完成 `set_text`，用完即弃。避免重复 `tauri-plugin-clipboard-manager`
/// 在启动时持有单例 `arboard::Clipboard` 所造成的 Linux Wayland 崩溃/挂死
/// （详见本模块头注释）。Wayland 合成器不支持 data-control 时 `arboard` 自行回退
/// X11；仅当两者都不可用才返回 `Err`（前缀 `CLIPBOARD_TEXT_`），由前端提示复制失败。
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

/// 单个剪贴板附件的字节上限：超过则拒绝（提示降级），避免整份大文件 base64
/// 后经 `postMessage` 塞爆 WebView（≈27MB 文本，与贴图 50MP 上限同量级）。
const MAX_CLIPBOARD_FILE_BYTES: u64 = 20 * 1024 * 1024;

/// 单次粘贴最多读取的附件数：防御剪贴板里成百上千个文件的极端场景。
const MAX_CLIPBOARD_FILES: usize = 32;

/// 单次粘贴累计读取的字节上限：单文件上限挡不住「多个刚好不超限」的极端组合
/// （32×20MB=640MB 仍会撑爆 WebView），累计越线即整批拒绝，提示用户改用引用路径。
const MAX_CLIPBOARD_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

/// 剪贴板文件读取结果：成功附件 + 被拒条目（前端据此提示降级，不静默丢弃）。
#[derive(Debug, serde::Serialize)]
pub struct ClipboardFileEntry {
    /// 形如 `data:<mime>;base64,...`
    pub data_url: String,
    pub mime: String,
    pub filename: String,
    pub size: u64,
}

/// 被拒绝条目的原因码：`too_large` / `unreadable` / `not_a_file` / `total_exceeded`。
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

/// 按扩展名推断 MIME，未知统一 `application/octet-stream`。
///
/// 只覆盖聊天框常见附件类型；其余按通用二进制，输入框据此走普通文件上传路径。
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

/// 剪贴板文件的展示名：取路径末段，异常（空/非 UTF-8）落回通用名。
fn clipboard_filename(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("clipboard-file")
        .to_string()
}

/// 清洗 arboard 从 `text/uri-list` 解出的路径。
///
/// arboard 3.6.1 的 `paths_from_uri_list` 只按 `\n` 切分 URI 列表，而 deepin 文件
/// 管理器（DFM）等把每条 URI 写成 `\r\n` 结尾，于是路径末尾残留 `\r`：
/// `/path/工作簿1.xlsx\r`。该路径 `metadata()` 必然失败，文件会被拒成 `unreadable`
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

/// 读取单个剪贴板文件并编码为 data URL；不可粘时返回拒绝原因。
///
/// `remaining` 是本次粘贴剩余的总量配额。**所有判定都在 `metadata` 阶段完成**
/// （目录 / 单文件超限 / 总量不足都在读文件内容之前拒绝），避免「整份读进内存
/// 再丢弃」的峰值；成功时从 `remaining` 扣掉本文件字节数。
fn read_one_clipboard_file(
    path: &Path,
    remaining: &mut u64,
) -> Result<ClipboardFileEntry, ClipboardFileRejection> {
    // 闭包只捕获 `path` 引用（各分支现场提取文件名），不在别处持有 String 借用
    let reject = |reason: &str| ClipboardFileRejection {
        filename: clipboard_filename(path),
        reason: reason.to_string(),
    };

    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return Err(reject("unreadable")),
    };
    // 目录 / 符号链接指向的非普通文件：聊天框没有目录附件语义，拒绝而非吞掉
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

/// 把剪贴板路径列表读成附件响应（纯同步、可单测；由 `read_clipboard_files` 在
/// `spawn_blocking` 内调用）。
///
/// 三级防御：条数（`take`，最多 32）、单文件（20MB）、累计总量（64MB）。后两级
/// 判定都在读内容之前（见 [`read_one_clipboard_file`]），被拒条目随响应回传，
/// 由宿主提示降级，不静默丢弃。
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
///
/// 路径完全来自**系统剪贴板**（本命令读取），不接受 iframe 传入的路径——iframe
/// 无法借这座桥读取本机任意文件。剪贴板无文件时返回空列表（不是错误）。
///
/// Linux 两个后端（X11 `URI_LIST`/`GNOME_COPIED_FILES`、Wayland `text/uri-list`）
/// arboard 3.6.1 均已实现；Wayland 合成器不支持 data-control 时 arboard 自行回退
/// X11，读不到则返回 `ContentNotAvailable` / `ClipboardNotSupported`，一律降级为
/// 空列表（行为与修复前一致，不抛出）。
///
/// 与 [`read_clipboard_image`] 一致：`spawn_blocking` 内惰性新建短期
/// `arboard::Clipboard`，用完即弃。失败返回 `Err`（前缀 `CLIPBOARD_FILES_`）。
#[tauri::command]
pub async fn read_clipboard_files() -> Result<ClipboardFilesResponse, String> {
    tokio::task::spawn_blocking(|| -> Result<ClipboardFilesResponse, String> {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| format!("CLIPBOARD_FILES_ACCESS: {e}"))?;

        let paths = match clipboard.get().file_list() {
            Ok(paths) => paths,
            // 剪贴板里没有文件（普通文本 / 图片等），不是错误
            Err(arboard::Error::ContentNotAvailable) => Vec::new(),
            // 后端不支持 / 无法访问文件列表（如 Wayland 无 data-control 且 X11 回退失败）：
            // 优雅降级为空，与修复前行为一致
            Err(arboard::Error::ClipboardNotSupported) => Vec::new(),
            Err(e) => return Err(format!("CLIPBOARD_FILES_READ: {e}")),
        };

        // 诊断：剪贴板里到底有没有文件、arboard 解出什么（CRLF 残留是 issue #1160 的
        // 实测根因，见 sanitize_uri_path）。仅 info 级别，正常路径也会留一行。
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
        // JPEG magic + 占位内容：长度一律由 PAYLOAD 推导，避免手数字节数漂移
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
        // 稀疏文件：只记长度，不落真实磁盘块，测试仍快速
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_CLIPBOARD_FILE_BYTES + 1).unwrap();

        let mut remaining = MAX_CLIPBOARD_TOTAL_BYTES;
        let deny = read_one_clipboard_file(&path, &mut remaining).unwrap_err();
        assert_eq!(deny.reason, "too_large");
        assert_eq!(deny.filename, "big.bin");
        // 拒绝发生在 metadata 阶段：配额未消耗
        assert_eq!(remaining, MAX_CLIPBOARD_TOTAL_BYTES);
    }

    #[test]
    fn read_one_clipboard_file_rejects_when_total_quota_exhausted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ok.txt");
        std::fs::write(&path, b"content").unwrap();

        // 文件本身没超单文件上限，但剩余配额为 0 → total_exceeded，且不消耗、不读取
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
        // deepin 文件管理器（DFM）写出的 uri-list 以 \r\n 结尾，arboard 只按 \n 切分，
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
        // 端到端语义：带 \r 的路径读不到，清洗后能正常读成附件
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
        assert_eq!(guess_mime_by_extension(Path::new("c.pdf")), "application/pdf");
        assert_eq!(guess_mime_by_extension(Path::new("d.unknownext")), "application/octet-stream");
        assert_eq!(guess_mime_by_extension(Path::new("noext")), "application/octet-stream");
    }
}
