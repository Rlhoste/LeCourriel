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
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserConfig {
    pub display_name: String,
    #[serde(default = "default_font_scale")]
    pub font_scale: f32,
}

fn default_font_scale() -> f32 { 1.0 }

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
        })
    }
}

/// Persist the full config to config.toml.
pub fn save_config(cfg: &Config) -> Result<()> {
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
