use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub user: UserConfig,
    /// Multiple email accounts. At least one required.
    #[serde(default)]
    pub accounts: Vec<AccountConfig>,
    pub claude: ClaudeConfig,
    #[serde(default)]
    pub projects: Vec<ProjectConfig>,
    #[serde(default)]
    pub gifs: GifConfig,
    /// Optional PostgreSQL server to sync app data with. Local to this computer: never synced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<DatabaseConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct DatabaseConfig {
    /// e.g. postgres://courriel:secret@db.example.com:5432/courriel?sslmode=require
    pub url: String,
    /// Skip TLS certificate checks (self-signed certificates on a self-hosted server).
    #[serde(default)]
    pub accept_invalid_certs: bool,
}

impl Config {
    /// The database URL, if one is configured and non-empty.
    pub fn database_url(&self) -> Option<&str> {
        self.database.as_ref().map(|d| d.url.trim()).filter(|u| !u.is_empty())
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
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
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
        let content = fs::read_to_string("config.toml")
            .context("Failed to read config.toml — copy config.toml.example and fill in credentials")?;

        // Try new format first.
        if let Ok(cfg) = toml::from_str::<Config>(&content) {
            if !cfg.accounts.is_empty() {
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
            database: None,
        })
    }
}

/// Persist the full config to config.toml and queue a push of the synced preferences.
pub fn save_config(cfg: &Config) -> Result<()> {
    write_local(cfg)?;
    crate::pgsync::schedule_push(crate::pgsync::Doc::Prefs);
    Ok(())
}

/// Write config.toml only (used when applying preferences pulled from the database).
pub fn write_local(cfg: &Config) -> Result<()> {
    let serialized = toml::to_string_pretty(cfg).context("Failed to serialize config")?;
    fs::write("config.toml", serialized).context("Failed to write config.toml")?;
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
