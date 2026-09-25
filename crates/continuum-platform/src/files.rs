use std::fs;
use std::path::{Path, PathBuf};

use continuum_core::Representation;

pub const FILE_MIME: &str = "application/x-continuum-file";
pub const URI_LIST_MIME: &str = "text/uri-list";
pub const GNOME_COPIED_FILES_MIME: &str = "x-special/gnome-copied-files";
pub const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;

const HEADER_LEN: usize = 4;

#[must_use]
pub fn is_file(representation: &Representation) -> bool {
    representation.mime == FILE_MIME
}

#[must_use]
pub fn encode(name: &str, content: &[u8]) -> Representation {
    let name_bytes = name.as_bytes();
    let mut bytes = Vec::with_capacity(HEADER_LEN + name_bytes.len() + content.len());
    bytes.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
    bytes.extend_from_slice(name_bytes);
    bytes.extend_from_slice(content);
    Representation::new(FILE_MIME, bytes)
}

#[must_use]
pub fn decode(representation: &Representation) -> Option<(String, Vec<u8>)> {
    if representation.mime != FILE_MIME || representation.bytes.len() < HEADER_LEN {
        return None;
    }
    let len = u32::from_le_bytes(representation.bytes[..HEADER_LEN].try_into().ok()?) as usize;
    if representation.bytes.len() < HEADER_LEN + len {
        return None;
    }
    let name =
        String::from_utf8(representation.bytes[HEADER_LEN..HEADER_LEN + len].to_vec()).ok()?;
    let content = representation.bytes[HEADER_LEN + len..].to_vec();
    Some((name, content))
}

/// Writes `content` under `~/Downloads/Continuum`, never overwriting, and returns the path.
pub fn save(name: &str, content: &[u8]) -> std::io::Result<PathBuf> {
    let dir = destination_dir();
    fs::create_dir_all(&dir)?;
    let path = unique_path(&dir, &sanitize(name));
    fs::write(&path, content)?;
    Ok(path)
}

#[must_use]
pub fn destination_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join("Downloads")))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("Continuum")
}

pub fn read_file(path: &Path) -> Option<(String, Vec<u8>)> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file")
        .to_string();
    Some((name, bytes))
}

#[must_use]
pub fn file_url(path: &Path) -> String {
    format!("file://{}", percent_encode(&path.to_string_lossy()))
}

#[must_use]
pub fn uri_list_bytes(paths: &[PathBuf]) -> Vec<u8> {
    let mut out = String::new();
    for path in paths {
        out.push_str("file://");
        out.push_str(&percent_encode(&path.to_string_lossy()));
        out.push_str("\r\n");
    }
    out.into_bytes()
}

#[must_use]
pub fn gnome_copied_files_bytes(paths: &[PathBuf]) -> Vec<u8> {
    let mut out = String::from("copy\n");
    for path in paths {
        out.push_str("file://");
        out.push_str(&percent_encode(&path.to_string_lossy()));
        out.push('\n');
    }
    out.into_bytes()
}

#[must_use]
pub fn parse_uri_list(bytes: &[u8]) -> Vec<PathBuf> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.strip_prefix("file://"))
        .map(|encoded| PathBuf::from(percent_decode(encoded)))
        .collect()
}

fn sanitize(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c == '\0' {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "file".to_string()
    } else {
        trimmed.to_string()
    }
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|s| s.to_str());
    for n in 1.. {
        let next = match ext {
            Some(ext) => format!("{stem} ({n}).{ext}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = dir.join(next);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("unique file name search is unbounded")
}

fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(value) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
