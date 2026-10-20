//! Where the app keeps its files (config.toml, cache, tasks…).
//!
//! Every data file goes through `file(name)`, which places it in the data
//! directory chosen at startup. It is, in order:
//! 1. `$COURRIEL_DATA_DIR`, if set;
//! 2. the current directory, if it already holds a config.toml (development,
//!    `./run.sh`, and installs from before packaged builds existed);
//! 3. the per-user application folder:
//!    Linux `$XDG_DATA_HOME/le-courriel` (default `~/.local/share/le-courriel`),
//!    Windows `%APPDATA%\LeCourriel`, macOS `~/Library/Application Support/LeCourriel`.
//!
//! Packaged builds (AppImage, .exe) are launched from read-only or arbitrary
//! folders, so they always end up in 3.
//!
//! The process never changes its working directory: the AppImage's WebKit finds
//! its helper programs through paths relative to the folder it was started in.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn platform_data_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("LeCourriel"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/LeCourriel"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|d| d.join("le-courriel"))
    }
}

/// Pick the data directory (see the module docs). Pure apart from the existence check.
pub fn choose_data_dir(env_override: Option<PathBuf>, cwd: Option<&Path>, platform: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(dir) = env_override.filter(|d| !d.as_os_str().is_empty()) {
        return Some(dir);
    }
    if let Some(cwd) = cwd {
        if cwd.join("config.toml").is_file() {
            return Some(cwd.to_path_buf());
        }
    }
    platform
}

/// Choose the data directory and create it. Call once, at startup.
pub fn init_data_dir() -> std::io::Result<PathBuf> {
    let cwd = std::env::current_dir().ok();
    let dir = choose_data_dir(
        std::env::var_os("COURRIEL_DATA_DIR").map(PathBuf::from),
        cwd.as_deref(),
        platform_data_dir(),
    )
    .or(cwd)
    .ok_or_else(|| std::io::Error::other("no usable data directory"))?;
    std::fs::create_dir_all(&dir)?;
    let _ = DATA_DIR.set(dir.clone());
    Ok(dir)
}

/// Full path of a data file. Before `init_data_dir` (unit tests), relative to the current directory.
pub fn file(name: &str) -> PathBuf {
    match DATA_DIR.get() {
        Some(dir) => dir.join(name),
        None => PathBuf::from(name),
    }
}
