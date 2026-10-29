//! Attachments of an outgoing message (#43 files and folders, #44 drag and drop).
//!
//! The UI keeps `{ name, size, path }` triples: `path` is a file sitting on disk, so the
//! message can be sent later without keeping anything in memory. A folder never travels as
//! it is: it becomes one `FolderName.zip`, written in a staging folder under the system temp
//! directory, with its inner structure kept.

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;
use zip::ZipWriter;

/// An attachment as the editor lists it under the message text.
#[derive(Serialize, Clone, Debug)]
pub struct AttachmentInfo {
    /// Name the recipient sees (a folder's is `FolderName.zip`).
    pub name: String,
    /// Size in bytes.
    pub size: u64,
    /// The file to read when the message is sent.
    pub path: String,
}

/// One file of a dropped item, base64-encoded; `path` is relative to the dropped item.
#[derive(Deserialize, Debug)]
pub struct DroppedFile {
    pub path: String,
    pub data: String,
}

/// One item dropped on the editor or the reply bar: a file (`dir` false, one entry) or a
/// folder (`dir` true, every file found inside it).
#[derive(Deserialize, Debug)]
pub struct DroppedItem {
    pub name: String,
    #[serde(default)]
    pub dir: bool,
    pub files: Vec<DroppedFile>,
}

/// A file ready to go into the multipart message.
pub struct Loaded {
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

/// MIME type by extension; anything unknown stays `application/octet-stream`.
const MIME: &[(&str, &str)] = &[
    ("txt", "text/plain"),
    ("md", "text/markdown"),
    ("csv", "text/csv"),
    ("tsv", "text/tab-separated-values"),
    ("json", "application/json"),
    ("xml", "application/xml"),
    ("yaml", "application/yaml"),
    ("yml", "application/yaml"),
    ("html", "text/html"),
    ("htm", "text/html"),
    ("css", "text/css"),
    ("js", "text/javascript"),
    ("ics", "text/calendar"),
    ("vcf", "text/vcard"),
    ("log", "text/plain"),
    ("rtf", "application/rtf"),
    ("pdf", "application/pdf"),
    ("ps", "application/postscript"),
    ("doc", "application/msword"),
    ("docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
    ("xls", "application/vnd.ms-excel"),
    ("xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
    ("ppt", "application/vnd.ms-powerpoint"),
    ("pptx", "application/vnd.openxmlformats-officedocument.presentationml.presentation"),
    ("odt", "application/vnd.oasis.opendocument.text"),
    ("ods", "application/vnd.oasis.opendocument.spreadsheet"),
    ("odp", "application/vnd.oasis.opendocument.presentation"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("bmp", "image/bmp"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
    ("svg", "image/svg+xml"),
    ("heic", "image/heic"),
    ("ico", "image/x-icon"),
    ("mp3", "audio/mpeg"),
    ("wav", "audio/wav"),
    ("ogg", "audio/ogg"),
    ("m4a", "audio/mp4"),
    ("flac", "audio/flac"),
    ("mp4", "video/mp4"),
    ("mov", "video/quicktime"),
    ("avi", "video/x-msvideo"),
    ("mkv", "video/x-matroska"),
    ("webm", "video/webm"),
    ("zip", "application/zip"),
    ("gz", "application/gzip"),
    ("tgz", "application/gzip"),
    ("tar", "application/x-tar"),
    ("7z", "application/x-7z-compressed"),
    ("rar", "application/vnd.rar"),
    ("exe", "application/vnd.microsoft.portable-executable"),
    ("msi", "application/x-msdownload"),
    ("apk", "application/vnd.android.package-archive"),
    ("deb", "application/vnd.debian.binary-package"),
    ("dmg", "application/x-apple-diskimage"),
];

pub fn guess_mime(name: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    MIME.iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, mime)| mime.to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

/// A name that is safe to write in one folder: no separator, nothing empty.
pub fn safe_file_name(name: &str) -> String {
    let flat: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '\0' => '_',
            c => c,
        })
        .collect();
    let base = flat.trim();
    if base.is_empty() || base == "." || base == ".." {
        "attachment".to_string()
    } else {
        base.to_string()
    }
}

/// The name a file keeps inside a zip: under `top` (the folder it came from), with `..`,
/// `.` and leading separators removed, and backslashes turned into `/`.
pub fn zip_entry_name(top: Option<&str>, rel: &str) -> String {
    let normalised = rel.replace('\\', "/");
    let mut parts: Vec<String> = Vec::new();
    for seg in normalised.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            parts.pop();
            continue;
        }
        parts.push(seg.to_string());
    }
    let mut out: Vec<String> = Vec::new();
    if let Some(folder) = top.map(safe_file_name).filter(|f| !f.is_empty()) {
        // The paths the webview gives for a dropped folder already start with its name:
        // it must not appear twice.
        let already = parts.first().map(|first| *first == folder).unwrap_or(false);
        if !already {
            out.push(folder);
        }
    }
    out.extend(parts);
    out.join("/")
}

/// Build a zip in memory, deflating every entry.
pub fn zip_bytes(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut buf);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, data) in entries {
            if name.is_empty() {
                continue;
            }
            writer
                .start_file(name.as_str(), options)
                .with_context(|| format!("Cannot add {name} to the zip"))?;
            writer
                .write_all(data)
                .with_context(|| format!("Cannot write {name} to the zip"))?;
        }
        writer.finish().context("Cannot finish the zip")?;
    }
    Ok(buf.into_inner())
}

/// The folder the app writes attachments into, under the system temp directory.
fn out_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("le-courriel-attachments");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("Cannot create {}: {e}", dir.display());
    }
    dir
}

/// A fresh folder for one written attachment, so two files of the same name never clash.
fn staging_dir() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = out_dir().join(format!("{stamp}-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("Cannot create {}: {e}", dir.display());
    }
    dir
}

fn write_staged(name: &str, bytes: Vec<u8>) -> Result<AttachmentInfo> {
    let clean = safe_file_name(name);
    let size = bytes.len() as u64;
    let path = staging_dir().join(&clean);
    std::fs::write(&path, &bytes).with_context(|| format!("Cannot write {clean}"))?;
    Ok(AttachmentInfo {
        name: clean,
        size,
        path: path.to_string_lossy().into_owned(),
    })
}

/// Every file under `dir`, named as it sits inside `root`, prefixed with `top`.
fn collect_dir(root: &Path, top: &str, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
    let read = std::fs::read_dir(dir).with_context(|| format!("Cannot read {}", dir.display()))?;
    for entry in read {
        let entry = entry.with_context(|| format!("Cannot read {}", dir.display()))?;
        let path = entry.path();
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(_) => continue,
        };
        if kind.is_dir() {
            // Symbolic links are not followed (their `is_dir` is false), so a link back to
            // a parent cannot loop here.
            collect_dir(root, top, &path, out)?;
        } else if kind.is_file() {
            let inside = path.strip_prefix(root).unwrap_or(path.as_path());
            let rel = inside.to_string_lossy().replace('\\', "/");
            match std::fs::read(&path) {
                Ok(data) => out.push((zip_entry_name(Some(top), &rel), data)),
                Err(e) => tracing::warn!("Attachment skipped {}: {e}", path.display()),
            }
        }
    }
    Ok(())
}

/// Zip a folder on disk into one `FolderName.zip` in the staging folder.
pub fn zip_folder(dir: &Path) -> Result<AttachmentInfo> {
    let top = dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(safe_file_name)
        .filter(|n| n != "attachment")
        .ok_or_else(|| anyhow!("Cannot name this folder"))?;
    let mut entries = Vec::new();
    collect_dir(dir, &top, dir, &mut entries)?;
    if entries.is_empty() {
        return Err(anyhow!("{top} is empty"));
    }
    let bytes = zip_bytes(&entries)?;
    write_staged(&format!("{top}.zip"), bytes)
}

/// Prepare what the picker chose: files as they are, folders as one zip.
pub fn prepare(paths: &[String]) -> Result<Vec<AttachmentInfo>> {
    let mut out = Vec::new();
    for p in paths.iter().filter(|p| !p.trim().is_empty()) {
        let path = Path::new(p);
        if path.is_dir() {
            out.push(zip_folder(path)?);
        } else if path.is_file() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(safe_file_name)
                .ok_or_else(|| anyhow!("Cannot name {p}"))?;
            let size = std::fs::metadata(path)
                .with_context(|| format!("Cannot read {name}"))?
                .len();
            out.push(AttachmentInfo {
                name,
                size,
                path: p.clone(),
            });
        } else {
            return Err(anyhow!("This file is no longer there: {p}"));
        }
    }
    Ok(out)
}

fn decode_base64(text: &str) -> Result<Vec<u8>> {
    // A data: URL may have been sent as it is; the payload is the last part.
    let body = text.rsplit(',').next().unwrap_or(text);
    let clean: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(clean)
        .context("A dropped file could not be read")
}

/// Write what was dropped: a file as it is, a folder as one zip of the dropped files.
pub fn save_dropped(items: &[DroppedItem]) -> Result<Vec<AttachmentInfo>> {
    let mut out = Vec::new();
    for item in items {
        let name = safe_file_name(&item.name);
        if item.dir {
            let mut entries = Vec::new();
            for file in &item.files {
                entries.push((zip_entry_name(Some(&name), &file.path), decode_base64(&file.data)?));
            }
            if entries.is_empty() {
                return Err(anyhow!("{name} is empty"));
            }
            let bytes = zip_bytes(&entries)?;
            out.push(write_staged(&format!("{name}.zip"), bytes)?);
        } else {
            let one = item.files.first().ok_or_else(|| anyhow!("{name} is empty"))?;
            out.push(write_staged(&name, decode_base64(&one.data)?)?);
        }
    }
    Ok(out)
}

/// Read every attachment path for sending.
pub fn load_all(paths: &[String]) -> Result<Vec<Loaded>> {
    paths
        .iter()
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let path = Path::new(p);
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| anyhow!("Cannot name {p}"))?
                .to_string();
            let data = std::fs::read(path).with_context(|| format!("Cannot read {filename}"))?;
            Ok(Loaded {
                content_type: guess_mime(&filename),
                filename,
                data,
            })
        })
        .collect()
}