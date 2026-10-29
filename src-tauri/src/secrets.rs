//! Mail passwords and API keys live in the OS keychain (Windows Credential Manager, macOS
//! Keychain, Secret Service on Linux), not in config.toml. Where there is no keychain they
//! stay in config.toml, and the log says so.

use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex};

use crate::config::Config;

const SERVICE: &str = "io.courriel.app";

/// Secret name → value, e.g. "imap:0:remi@imap.example.com" → the password.
pub type Secrets = BTreeMap<String, String>;

/// An account's secrets are named after its place and login: an account removed or moved
/// never gets another one's password.
fn account_names(i: usize, a: &crate::config::AccountConfig) -> (String, String) {
    let login = format!("{}@{}", a.imap.username, a.imap.host);
    (format!("imap:{i}:{login}"), format!("smtp:{i}:{login}"))
}

/// Take every password and key out of `cfg`.
pub fn split(cfg: &mut Config) -> Secrets {
    let mut out = Secrets::new();
    for (i, a) in cfg.accounts.iter_mut().enumerate() {
        let (imap, smtp) = account_names(i, a);
        out.insert(imap, std::mem::take(&mut a.imap.password));
        out.insert(smtp, std::mem::take(&mut a.smtp.password));
    }
    for (p, k) in std::mem::take(&mut cfg.ai.keys) {
        out.insert(format!("ai:{p}"), k);
    }
    out.insert("giphy".into(), std::mem::take(&mut cfg.gifs.giphy_api_key));
    out.retain(|_, v| !v.is_empty());
    out
}

/// Put back the secrets `split` took. A value already in `cfg` (config.toml) is kept.
pub fn join(cfg: &mut Config, secrets: &Secrets) {
    let fill = |value: &mut String, name: &str| {
        if value.is_empty() {
            *value = secrets.get(name).cloned().unwrap_or_default();
        }
    };
    for (i, a) in cfg.accounts.iter_mut().enumerate() {
        let (imap, smtp) = account_names(i, a);
        fill(&mut a.imap.password, &imap);
        fill(&mut a.smtp.password, &smtp);
    }
    for (name, key) in secrets {
        if let Some(p) = name.strip_prefix("ai:") {
            cfg.ai.keys.entry(p.to_string()).or_insert_with(|| key.clone());
        }
    }
    fill(&mut cfg.gifs.giphy_api_key, "giphy");
}

/// One keychain entry per data folder: other installs, profiles and test runs keep their own.
fn entry() -> Option<keyring::Entry> {
    // Tests never touch the real keychain: their configs stay in their own files.
    if cfg!(test) || keyring::Entry::store_status().is_err() {
        return None;
    }
    keyring::Entry::new(SERVICE, &crate::paths::file("config.toml").display().to_string()).ok()
}

/// The keychain's content (JSON), read once per run. None when there is no keychain or it
/// could not be read: then nothing is written to it for the whole run, so a failed read
/// (a locked keyring, a dismissed prompt) can never erase the saved passwords.
static STORED: LazyLock<Mutex<Option<String>>> = LazyLock::new(|| {
    Mutex::new(entry().and_then(|e| match e.get_secret() {
        Ok(bytes) => String::from_utf8(bytes).ok(),
        Err(keyring::Error::NoEntry) => Some("{}".into()),
        Err(err) => {
            tracing::warn!("Could not read the OS keychain: {}", err);
            None
        }
    }))
});

/// The secrets saved in the keychain, None if it can't be used.
pub fn load() -> Option<Secrets> {
    serde_json::from_str(STORED.lock().unwrap().as_deref()?).ok()
}

/// Save the secrets to the keychain (only when they changed). False when it can't be used.
// shortcut: Windows limits an entry to 2560 bytes (about 10 long API keys); beyond that the
// secrets stay in config.toml. Split into one entry per secret if users hit it.
pub fn save(secrets: &Secrets) -> bool {
    let mut stored = STORED.lock().unwrap();
    let Some(old) = stored.as_mut() else { return false };
    let json = serde_json::to_string(secrets).expect("a map of strings");
    if *old == json {
        return true;
    }
    match entry().map(|e| e.set_secret(json.as_bytes())) {
        Some(Ok(())) => {
            *old = json;
            true
        }
        Some(Err(e)) => {
            tracing::warn!("Could not write to the OS keychain: {}", e);
            false
        }
        None => false,
    }
}
