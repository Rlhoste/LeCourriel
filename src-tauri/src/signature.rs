//! Email signatures imported from an HTML file (exported from Outlook, Gmail, a
//! signature generator…). Each account has its own, kept as `signature-<hex>.html` in
//! the data folder and named in config.toml (`[accounts.signature] file = …`).
//!
//! On import the HTML is decoded (Outlook writes Windows-1252), reduced to its
//! <body>, and local images are embedded as data: URIs, so the signature no longer
//! depends on the original file and its images travel with the email (sent as
//! inline attachments by `smtp_sender::extract_inline_images`).

use anyhow::{bail, Context, Result};
use base64::Engine;
use std::path::{Path, PathBuf};

use crate::config::{Config, SignatureConfig};

/// The single signature file of older versions (see `migrate_legacy`).
const LEGACY_FILE: &str = "signature.html";
const MAX_HTML_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: u64 = 2 * 1024 * 1024;

/// Decode HTML bytes: UTF-8 when valid (or with a BOM), else the charset declared in a
/// <meta> tag, else Windows-1252 (what Outlook uses for its signature files).
pub fn decode_html(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.trim_start_matches('\u{feff}').to_string();
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_lowercase();
    let label = head.find("charset=").map(|i| {
        head[i + 8..]
            .trim_start_matches(['"', '\''])
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect::<String>()
    });
    let encoding = label
        .and_then(|l| encoding_rs::Encoding::for_label(l.as_bytes()))
        .unwrap_or(encoding_rs::WINDOWS_1252);
    let (text, _, _) = encoding.decode(bytes);
    text.into_owned()
}

/// The inside of <body>…</body> for a full document, otherwise the HTML as is.
pub fn body_inner(html: &str) -> String {
    // ASCII lowercase keeps byte positions identical to `html`.
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<body").and_then(|i| lower[i..].find('>').map(|j| i + j + 1));
    match start {
        Some(start) => {
            let end = lower.rfind("</body").filter(|&e| e >= start).unwrap_or(html.len());
            html[start..end].trim().to_string()
        }
        None => html.trim().to_string(),
    }
}

fn image_mime(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None, // svg and others: many mail clients block them
    }
}

/// Decode %XX escapes and the HTML entity &amp; in an attribute value.
fn unescape_src(v: &str) -> String {
    let v = v.replace("&amp;", "&");
    let bytes = v.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&v[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Where a local image src points to, if it is local at all.
fn local_path(src: &str, base_dir: &Path) -> Option<PathBuf> {
    let lower = src.to_lowercase();
    if lower.starts_with("http:") || lower.starts_with("https:") || lower.starts_with("data:")
        || lower.starts_with("cid:") || lower.starts_with("//")
    {
        return None;
    }
    let raw = unescape_src(src);
    let raw = raw
        .strip_prefix("file://localhost")
        .or_else(|| raw.strip_prefix("file://"))
        .unwrap_or(&raw);
    // file:///C:/… on Windows
    let raw = if raw.len() > 3 && raw.starts_with('/') && raw.as_bytes()[2] == b':' { &raw[1..] } else { raw };
    let p = PathBuf::from(raw);
    Some(if p.is_absolute() { p } else { base_dir.join(p) })
}

/// Replace local image sources by data: URIs. Remote images, missing files, unsupported
/// types and images over `max_bytes` are left untouched.
pub fn embed_local_images(html: &str, base_dir: &Path, max_bytes: u64) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(pos) = rest.to_ascii_lowercase().find("src=") {
        let (before, after) = rest.split_at(pos + 4);
        out.push_str(before);
        let quote = match after.chars().next() {
            Some(c @ ('"' | '\'')) => c,
            _ => { rest = after; continue; }
        };
        let value_start = &after[1..];
        let Some(end) = value_start.find(quote) else { rest = after; continue; };
        let value = &value_start[..end];
        let embedded = local_path(value, base_dir).and_then(|path| {
            let mime = image_mime(&path)?;
            let meta = std::fs::metadata(&path).ok()?;
            if !meta.is_file() || meta.len() > max_bytes {
                return None;
            }
            let data = std::fs::read(&path).ok()?;
            Some(format!("data:{};base64,{}", mime, base64::engine::general_purpose::STANDARD.encode(data)))
        });
        out.push(quote);
        out.push_str(embedded.as_deref().unwrap_or(value));
        out.push(quote);
        rest = &value_start[end + 1..];
    }
    out.push_str(rest);
    out
}

/// The data folder, where signature files live next to config.toml.
pub fn data_dir() -> PathBuf {
    crate::paths::file(".")
}

/// Whether `name` is one of our signature files ("signature-<hex>.html"): nothing else
/// (no path, no other file) can be read or deleted through a signature name.
pub fn is_signature_file(name: &str) -> bool {
    name.strip_prefix("signature-")
        .and_then(|rest| rest.strip_suffix(".html"))
        .is_some_and(|hex| !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit()))
}

/// A file name not used yet in `dir`.
fn new_file_name(dir: &Path) -> String {
    let mut n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    loop {
        let name = format!("signature-{:x}.html", n);
        if !dir.join(&name).exists() {
            return name;
        }
        n += 1;
    }
}

/// Import a signature file into `dir` under a new name. Returns (file name, HTML).
/// The file only becomes an account's signature once Settings are saved.
pub fn import(path: &Path, dir: &Path) -> Result<(String, String)> {
    let meta = std::fs::metadata(path).with_context(|| format!("Cannot read {}", path.display()))?;
    if meta.len() > MAX_HTML_BYTES {
        bail!("This file is too large for a signature (over 2 MB)");
    }
    let bytes = std::fs::read(path).with_context(|| format!("Cannot read {}", path.display()))?;
    let html = body_inner(&decode_html(&bytes));
    if html.trim().is_empty() {
        bail!("This file contains no signature");
    }
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let html = embed_local_images(&html, base_dir, MAX_IMAGE_BYTES);
    let name = new_file_name(dir);
    std::fs::write(dir.join(&name), &html).context("Could not save the signature")?;
    Ok((name, html))
}

/// A stored signature ("" when there is none or the name is not a signature file).
pub fn load(dir: &Path, name: &str) -> String {
    if !is_signature_file(name) {
        return String::new();
    }
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
}

/// Delete the signature files no account uses any more (removed, replaced, or imported
/// and then cancelled). Returns how many were deleted.
pub fn remove_unused(dir: &Path, cfg: &Config) -> usize {
    let used: Vec<&str> = cfg.accounts.iter().map(|a| a.signature.file.as_str()).collect();
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| is_signature_file(name) && !used.contains(&name.as_str()))
        .filter(|name| std::fs::remove_file(dir.join(name)).is_ok())
        .count()
}

/// Signatures used to be one `signature.html` with a global `[signature]` block. Give it
/// to the first account (under a new name) when no account has a signature yet.
/// Returns true when `cfg` changed and should be saved.
pub fn migrate_legacy(dir: &Path, cfg: &mut Config) -> bool {
    let legacy = dir.join(LEGACY_FILE);
    if !legacy.is_file() {
        return false;
    }
    if cfg.accounts.is_empty() || cfg.accounts.iter().any(|a| !a.signature.file.is_empty()) {
        return false;
    }
    let name = new_file_name(dir);
    if std::fs::rename(&legacy, dir.join(&name)).is_err() {
        return false;
    }
    cfg.accounts[0].signature = SignatureConfig { file: name, ..cfg.signature.clone() };
    true
}
