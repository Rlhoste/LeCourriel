use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub user: UserConfig,
    /// Email accounts. Empty on first start, until one is added in Settings.
    #[serde(default)]
    pub accounts: Vec<AccountConfig>,
    #[serde(default)]
    pub claude: ClaudeConfig,
    #[serde(default)]
    pub projects: Vec<ProjectConfig>,
    #[serde(default)]
    pub gifs: GifConfig,
    /// Optional Supabase project to sync app data with. Local to this computer: never synced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supabase: Option<SupabaseConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SupabaseConfig {
    /// e.g. https://abcdefgh.supabase.co
    pub url: String,
    /// The project's public "anon" key. Safe to store: row-level security limits each
    /// signed-in user to their own rows.
    pub anon_key: String,
}

impl Config {
    /// Supabase URL and anon key, when both are set.
    pub fn supabase_settings(&self) -> Option<(&str, &str)> {
        let s = self.supabase.as_ref()?;
        let (url, key) = (s.url.trim(), s.anon_key.trim());
        (!url.is_empty() && !key.is_empty()).then_some((url, key))
    }
}

/// GIF picker in the composer (GIPHY).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GifConfig {
    #[serde(default)]
    pub giphy_api_key: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserConfig {
    #[serde(default)]
    pub display_name: String,
    #[serde(default = "default_font_scale")]
    pub font_scale: f32,
    /// UI theme: midnight, white, modern, brutalist, aero, terminal.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// UI language: "en" or "fr".
    #[serde(default = "default_language")]
    pub language: String,
}

fn default_font_scale() -> f32 { 1.0 }
fn default_theme() -> String { "midnight".to_string() }
fn default_language() -> String { "en".to_string() }

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            display_name: String::new(),
            font_scale: default_font_scale(),
            theme: default_theme(),
            language: default_language(),
        }
    }
}

/// One email account (IMAP + SMTP pair).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountConfig {
    /// Human-readable label, e.g. "Work" or "Personal".
    pub name: String,
    pub imap: ImapConfig,
    pub smtp: SmtpConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub mailbox: String,
    pub fetch_count: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from_address: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClaudeConfig {
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_model() -> String { "claude-opus-4-6".to_string() }
fn default_max_tokens() -> u32 { 300 }

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self { api_key: String::new(), model: default_model(), max_tokens: default_max_tokens() }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProjectConfig {
    pub name: String,
    pub description: String,
    #[serde(default = "default_urgency")]
    pub urgency: String,
}

fn default_urgency() -> String {
    "normal".to_string()
}

/// Legacy single-account format for automatic migration.
#[derive(Debug, Deserialize)]
struct LegacyConfig {
    user: UserConfig,
    imap: Option<ImapConfig>,
    smtp: Option<SmtpConfig>,
    claude: ClaudeConfig,
    #[serde(default)]
    projects: Vec<ProjectConfig>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let content = match fs::read_to_string(crate::paths::file("config.toml")) {
            Ok(c) => c,
            // First start: no config yet. The app opens Settings so an account can be added.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(e).context("Failed to read config.toml"),
        };

        // Try new format first. Only a file with a top-level [imap] is the legacy format.
        if let Ok(cfg) = toml::from_str::<Config>(&content) {
            let legacy_shape = toml::from_str::<toml::Value>(&content)
                .map(|v| v.get("imap").is_some())
                .unwrap_or(false);
            if !cfg.accounts.is_empty() || !legacy_shape {
                return Ok(cfg);
            }
        }

        // Fall back to legacy single-account format and migrate.
        let legacy: LegacyConfig = toml::from_str(&content)
            .context("Failed to parse config.toml")?;

        let account = if let (Some(imap), Some(smtp)) = (legacy.imap, legacy.smtp) {
            vec![AccountConfig {
                name: "Default".to_string(),
                imap,
                smtp,
            }]
        } else {
            vec![]
        };

        Ok(Config {
            user: legacy.user,
            accounts: account,
            claude: legacy.claude,
            projects: legacy.projects,
            gifs: GifConfig::default(),
            supabase: None,
        })
    }
}

/// Persist the full config to config.toml and queue a push of the synced preferences.
pub fn save_config(cfg: &Config) -> Result<()> {
    write_local(cfg)?;
    crate::cloud::schedule_push(crate::cloud::Doc::Prefs);
    Ok(())
}

/// Write config.toml only (used when applying preferences pulled from the database).
pub fn write_local(cfg: &Config) -> Result<()> {
    let serialized = toml::to_string_pretty(cfg).context("Failed to serialize config")?;
    fs::write(crate::paths::file("config.toml"), serialized).context("Failed to write config.toml")?;
    Ok(())
}

/// Update a project's description in config.toml in-place.
pub fn save_project_description(name: &str, description: &str) -> Result<()> {
    let mut cfg = Config::load()?;
    if let Some(proj) = cfg.projects.iter_mut().find(|p| p.name == name) {
        proj.description = description.to_string();
    }
    save_config(&cfg)
}
