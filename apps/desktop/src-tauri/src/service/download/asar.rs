//! Minimal read-only asar support.
//!
//! The dsh package ships as a single `.asar`; Electron patches `fs` so the page and
//! child processes can read inside it, but Rust cannot. The only thing this
//! process needs from the archive is metadata (today: `package.json`), so read the
//! JSON header and pull that one entry instead of depending on a bundler.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MAX_HEADER_SIZE: u64 = 64 * 1024 * 1024;
const MAX_ENTRY_SIZE: u64 = 64 * 1024 * 1024;

pub fn read_entry(archive: &Path, entry: &str) -> Result<Vec<u8>, String> {
    let mut file = File::open(archive)
        .map_err(|error| format!("ASAR_OPEN: {}: {error}", archive.display()))?;
    let (header, data_start, file_size) = read_header(&mut file)?;
    let node = find_entry(&header, entry)?;
    let size = node["size"].as_u64().unwrap();
    if size > MAX_ENTRY_SIZE {
        return Err(format!("ASAR_ENTRY_TOO_LARGE: {entry}"));
    }
    let offset = node["offset"].as_str().unwrap().parse::<u64>().unwrap();
    if data_start
        .checked_add(offset)
        .and_then(|start| start.checked_add(size))
        .is_none_or(|end| end > file_size)
    {
        return Err(format!("ASAR_ENTRY_BOUNDS: {entry}"));
    }
    file.seek(SeekFrom::Start(data_start + offset))
        .map_err(|error| format!("ASAR_SEEK: {error}"))?;
    let mut bytes = vec![0; size as usize];
    file.read_exact(&mut bytes)
        .map_err(|error| format!("ASAR_READ: {error}"))?;
    Ok(bytes)
}

fn read_header(file: &mut File) -> Result<(serde_json::Value, u64, u64), String> {
    let file_size = file
        .metadata()
        .map_err(|error| format!("ASAR_STAT: {error}"))?
        .len();
    let mut prefix = [0u8; 16];
    file.read_exact(&mut prefix)
        .map_err(|error| format!("ASAR_HEADER: {error}"))?;
    let pickle_size = u32::from_le_bytes(prefix[0..4].try_into().unwrap()) as u64;
    let header_block = u32::from_le_bytes(prefix[4..8].try_into().unwrap()) as u64;
    let header_pickle_size = u32::from_le_bytes(prefix[8..12].try_into().unwrap()) as u64;
    let json_size = u32::from_le_bytes(prefix[12..16].try_into().unwrap()) as u64;
    let data_start = 8u64
        .checked_add(header_block)
        .ok_or("ASAR_HEADER_INVALID: overflow")?;
    let json_end = 16u64
        .checked_add(json_size)
        .ok_or("ASAR_HEADER_INVALID: overflow")?;
    let padded_json_end = json_end
        .checked_add((4 - json_end % 4) % 4)
        .ok_or("ASAR_HEADER_INVALID: overflow")?;
    if pickle_size != 4
        || header_block < 8
        || header_pickle_size.checked_add(4) != Some(header_block)
        || json_size > MAX_HEADER_SIZE
        || data_start != padded_json_end
        || data_start > file_size
    {
        return Err("ASAR_HEADER_INVALID: inconsistent or out-of-bounds pickle lengths".into());
    }
    let mut bytes = vec![0; json_size as usize];
    file.read_exact(&mut bytes)
        .map_err(|error| format!("ASAR_HEADER_BODY: {error}"))?;
    let header =
        serde_json::from_slice(&bytes).map_err(|error| format!("ASAR_HEADER_JSON: {error}"))?;
    Ok((header, data_start, file_size))
}

fn find_entry<'a>(
    header: &'a serde_json::Value,
    entry: &str,
) -> Result<&'a serde_json::Value, String> {
    if entry.is_empty()
        || entry.starts_with('/')
        || entry
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("ASAR_ENTRY_PATH_INVALID: {entry}"));
    }
    let mut node = header;
    for segment in entry.split('/') {
        node = node
            .get("files")
            .and_then(|files| files.get(segment))
            .ok_or_else(|| format!("ASAR_ENTRY_MISSING: {entry}"))?;
    }
    if node.get("files").is_some()
        || node.get("unpacked").and_then(serde_json::Value::as_bool) == Some(true)
    {
        return Err(format!("ASAR_ENTRY_NOT_PACKED_FILE: {entry}"));
    }
    node.get("size")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("ASAR_ENTRY_SIZE: {entry}"))?;
    node.get("offset")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| format!("ASAR_ENTRY_OFFSET: {entry}"))?;
    Ok(node)
}

pub fn entry_exists(archive: &Path, entry: &str) -> Result<bool, String> {
    let mut file = File::open(archive)
        .map_err(|error| format!("ASAR_OPEN: {}: {error}", archive.display()))?;
    let (header, data_start, file_size) = read_header(&mut file)?;
    match find_entry(&header, entry) {
        Ok(node) => {
            let offset = node["offset"].as_str().unwrap().parse::<u64>().unwrap();
            let size = node["size"].as_u64().unwrap();
            if data_start
                .checked_add(offset)
                .and_then(|start| start.checked_add(size))
                .is_none_or(|end| end > file_size)
            {
                return Err(format!("ASAR_ENTRY_BOUNDS: {entry}"));
            }
            Ok(true)
        }
        Err(error) if error.starts_with("ASAR_ENTRY_MISSING:") => Ok(false),
        Err(error) => Err(error),
    }
}

pub fn validate_dsh_archive(archive: &Path) -> Result<(), String> {
    const PACKAGE: &str = "node_modules/@deepseek-ai/dsh/package.json";
    const BIN: &str = "node_modules/@deepseek-ai/dsh/lib/bin.js";
    if !entry_exists(archive, BIN)? {
        return Err(format!("ASAR_DSH_BIN_MISSING: {BIN}"));
    }
    let package = read_entry(archive, PACKAGE)?;
    serde_json::from_slice::<serde_json::Value>(&package)
        .map_err(|error| format!("ASAR_DSH_PACKAGE_INVALID: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Writes the smallest archive the reader accepts: one entry, no payload tail.
    ///
    /// Mirrors the real layout: pickle size, header block size, then a block whose
    /// JSON length sits at offset 12 and JSON at offset 16 after 4-byte padding.
    fn write_archive(path: &Path, entries: &[(&str, &[u8])]) {
        fn insert(
            files: &mut serde_json::Map<String, serde_json::Value>,
            path: &[&str],
            value: serde_json::Value,
        ) {
            if path.len() == 1 {
                files.insert(path[0].into(), value);
            } else {
                let node = files
                    .entry(path[0])
                    .or_insert_with(|| serde_json::json!({"files": {}}));
                insert(node["files"].as_object_mut().unwrap(), &path[1..], value);
            }
        }
        let mut files = serde_json::Map::new();
        let mut content = Vec::new();
        for (name, bytes) in entries {
            insert(
                &mut files,
                &name.split('/').collect::<Vec<_>>(),
                serde_json::json!({"size": bytes.len(), "offset": content.len().to_string()}),
            );
            content.extend_from_slice(bytes);
        }
        let json = serde_json::to_vec(&serde_json::json!({"files": files})).unwrap();
        let pad = (4 - (16 + json.len()) % 4) % 4;
        let block = 8 + json.len() + pad;
        let mut file = File::create(path).unwrap();
        file.write_all(&4u32.to_le_bytes()).unwrap();
        file.write_all(&(block as u32).to_le_bytes()).unwrap();
        file.write_all(&((block - 4) as u32).to_le_bytes()).unwrap();
        file.write_all(&(json.len() as u32).to_le_bytes()).unwrap();
        file.write_all(&json).unwrap();
        file.write_all(&vec![0; pad]).unwrap();
        file.write_all(&content).unwrap();
    }

    #[test]
    fn reads_a_file_out_of_the_archive() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("test.asar");
        let manifest = br#"{"version":"1.2.3"}"#;
        write_archive(
            &archive,
            &[
                ("node_modules/@deepseek-ai/dsh/package.json", manifest),
                ("node_modules/@deepseek-ai/dsh/lib/bin.js", b"bin"),
            ],
        );
        assert_eq!(
            read_entry(&archive, "node_modules/@deepseek-ai/dsh/package.json").unwrap(),
            manifest
        );
        validate_dsh_archive(&archive).unwrap();
        assert!(entry_exists(&archive, "node_modules/@deepseek-ai/dsh/lib/bin.js").unwrap());
        assert!(!entry_exists(&archive, "missing").unwrap());
        assert!(entry_exists(&archive, "../missing").is_err());
        let mut bytes = std::fs::read(&archive).unwrap();
        bytes.truncate(bytes.len() - 1);
        std::fs::write(&archive, bytes).unwrap();
        assert!(entry_exists(&archive, "node_modules/@deepseek-ai/dsh/lib/bin.js").is_err());
    }
}
