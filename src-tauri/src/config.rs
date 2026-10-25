use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub user: UserConfig,
    /// Email accounts. Empty on first start, until one is added in Settings.
    #[serde(default)]
    pub accounts: Vec<AccountConfig>,
    /// Legacy `[claude]` section: read for migration into `[ai]`, never written again.
    #[serde(default, skip_serializing)]
    pub claude: ClaudeConfig,
    /// Which AI the app uses, and the API key for each provider.
    #[serde(default)]
    pub ai: AiConfig,
    #[serde(default)]
    pub projects: Vec<ProjectConfig>,
    #[serde(default)]
    pub gifs: GifConfig,
    /// Legacy single signature options, from before signatures belonged to accounts.
    /// Read once by `signature::migrate_legacy`, never written back.
    #[serde(default, skip_serializing)]
    pub signature: SignatureConfig,
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
    /// The user's name, for the greeting and the AI. There is no field to type it in:
    /// it is the old display_name when set, else the name part of the first account's
    /// sender ("Remi Lhoste <remi@x.fr>" → "Remi Lhoste"), else "".
    pub fn user_name(&self) -> String {
        let name = self.user.display_name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        self.accounts.first()
            .and_then(|a| a.smtp.from_address.split_once('<'))
            .map(|(name, _)| name.trim().trim_matches('"').trim().to_string())
            .unwrap_or_default()
    }

    /// Supabase URL and anon key, when both are set.
    pub fn supabase_settings(&self) -> Option<(&str, &str)> {
        let s = self.supabase.as_ref()?;
        let (url, key) = (s.url.trim(), s.anon_key.trim());
        (!url.is_empty() && !key.is_empty()).then_some((url, key))
    }
}

/// An account's signature: its HTML file in the data folder ("" = none) and when to add it.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct SignatureConfig {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file: String,
    #[serde(default = "yes")]
    pub on_new: bool,
    #[serde(default = "yes")]
    pub on_reply: bool,
}

fn yes() -> bool { true }

impl Default for SignatureConfig {
    fn default() -> Self {
        Self { file: String::new(), on_new: true, on_reply: true }
    }
}

/// GIF picker in the composer (GIPHY).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GifConfig {
    /// The user's own key ("" = the app's built-in key).
    #[serde(default)]
    pub giphy_api_key: String,
}

/// The app's GIPHY key, used until the user sets their own in Settings. GIPHY keys are
/// meant to ship inside client apps (they only allow searching public GIFs).
pub const DEFAULT_GIPHY_KEY: &str = "4In929YGrxdWjgnGld0QawcDTvEZmbzf";

impl GifConfig {
    /// The key to use: the user's, else the built-in one.
    pub fn key(&self) -> &str {
        let own = self.giphy_api_key.trim();
        if own.is_empty() { DEFAULT_GIPHY_KEY } else { own }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserConfig {
    #[serde(default)]
    pub display_name: String,
    #[serde(default = "default_font_scale")]
    pub font_scale: f32,
    /// UI theme: midnight, premium (was white), modern, brutalist, aero, terminal.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// UI language: "en" or "fr".
    #[serde(default = "default_language")]
    pub language: String,
    /// Desktop notification when new unread emails arrive.
    #[serde(default = "yes")]
    pub notify_new_mail: bool,
}

fn default_font_scale() -> f32 { 1.0 }
fn default_theme() -> String { "brutalist".to_string() }
fn default_language() -> String { "en".to_string() }

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            display_name: String::new(),
            font_scale: default_font_scale(),
            theme: default_theme(),
            language: default_language(),
            notify_new_mail: true,
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
    #[serde(default)]
    pub signature: SignatureConfig,
}

impl AccountConfig {
    /// The SMTP settings used to send. Fields left blank in Settings fall back to the
    /// incoming (IMAP) ones: username and password are usually the same on both servers.
    /// A blank sender becomes "Display Name <login>" when the login is an email address.
    pub fn smtp_for_sending(&self, display_name: &str) -> SmtpConfig {
        let mut smtp = self.smtp.clone();
        if smtp.username.trim().is_empty() {
            smtp.username = self.imap.username.clone();
        }
        if smtp.password.is_empty() {
            smtp.password = self.imap.password.clone();
        }
        if smtp.from_address.trim().is_empty() {
            let email = smtp.username.trim();
            if email.contains('@') {
                let name = display_name.trim().replace(['"', '<', '>'], "");
                smtp.from_address = if name.is_empty() { email.to_string() } else { format!("\"{}\" <{}>", name, email) };
            }
        }
        smtp
    }
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

/// AI providers. Anthropic has its own API; all the others speak the OpenAI Chat Completions format.
pub const AI_PROVIDERS: [&str; 5] = ["anthropic", "openai", "mistral", "ollama", "custom"];

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AiConfig {
    /// anthropic | openai | mistral | ollama | custom ("" = not chosen yet: anthropic).
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    /// Output limit for email classification (other AI calls set their own).
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Empty = the provider's default URL; required for "custom".
    #[serde(default)]
    pub base_url: String,
    /// One API key per provider, so switching back and forth keeps them.
    #[serde(default)]
    pub keys: std::collections::BTreeMap<String, String>,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: String::new(),
            model: String::new(),
            max_tokens: default_max_tokens(),
            base_url: String::new(),
            keys: Default::default(),
        }
    }
}

impl AiConfig {
    pub fn provider(&self) -> &str {
        if self.provider.is_empty() { "anthropic" } else { &self.provider }
    }

    pub fn key(&self) -> &str {
        self.keys.get(self.provider()).map(|k| k.trim()).unwrap_or("")
    }

    /// The API base URL, without a trailing slash.
    pub fn base_url(&self) -> String {
        let custom = self.base_url.trim().trim_end_matches('/');
        if !custom.is_empty() {
            return custom.to_string();
        }
        default_base_url(self.provider()).to_string()
    }
}

pub fn default_base_url(provider: &str) -> &'static str {
    match provider {
        "openai" => "https://api.openai.com/v1",
        "mistral" => "https://api.mistral.ai/v1",
        "ollama" => "http://localhost:11434/v1",
        "custom" => "",
        _ => "https://api.anthropic.com/v1",
    }
}
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
        Self::load_from(&crate::paths::file("config.toml"))
    }

    /// Load a configuration from an explicit path. Keeping this independent from the
    /// process-wide data directory makes startup and migration behavior testable.
    pub fn load_from(path: &Path) -> Result<Self> {
        let content = match fs::read_to_string(path) {
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
                return Ok(cfg.migrated());
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
                signature: SignatureConfig::default(),
            }]
        } else {
            vec![]
        };

        Ok(Config {
            user: legacy.user,
            accounts: account,
            claude: legacy.claude,
            ai: AiConfig::default(),
            projects: legacy.projects,
            gifs: GifConfig::default(),
            signature: SignatureConfig::default(),
            supabase: None,
        }
        .migrated())
    }

    /// Move a legacy `[claude]` section into `[ai]` (Anthropic) when `[ai]` isn't set yet.
    pub fn migrated(mut self) -> Self {
        if self.ai.provider.is_empty() {
            self.ai.provider = "anthropic".into();
            if self.ai.model.is_empty() {
                self.ai.model = self.claude.model.clone();
            }
            self.ai.max_tokens = self.claude.max_tokens;
            if !self.claude.api_key.trim().is_empty() {
                self.ai.keys.entry("anthropic".into()).or_insert_with(|| self.claude.api_key.trim().to_string());
            }
        }
        self
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
    write_to(cfg, &crate::paths::file("config.toml"))
}

/// Write a configuration to an explicit path (used by production and isolated tests).
pub fn write_to(cfg: &Config, path: &Path) -> Result<()> {
    let serialized = toml::to_string_pretty(cfg).context("Failed to serialize config")?;
    fs::write(path, serialized).context("Failed to write config.toml")?;
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
