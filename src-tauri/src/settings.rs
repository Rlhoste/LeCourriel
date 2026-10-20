use anyhow::Result;
use qrcode::QrCode;
use qrcode::render::unicode;

use crate::config::{AccountConfig, ClaudeConfig, Config, ImapConfig, SmtpConfig, UserConfig};
use crate::dto::{AccountDto, SettingsDto};

pub fn config_to_dto(cfg: &Config) -> SettingsDto {
    SettingsDto {
        display_name: cfg.user.display_name.clone(),
        font_scale: cfg.user.font_scale,
        theme: cfg.user.theme.clone(),
        language: cfg.user.language.clone(),
        claude_key: cfg.claude.api_key.clone(),
        claude_model: cfg.claude.model.clone(),
        giphy_key: cfg.gifs.giphy_api_key.clone(),
        supabase_url: cfg.supabase.as_ref().map(|s| s.url.clone()).unwrap_or_default(),
        supabase_anon_key: cfg.supabase.as_ref().map(|s| s.anon_key.clone()).unwrap_or_default(),
        accounts: cfg.accounts.iter().map(|a| AccountDto {
            label: a.name.clone(),
            imap_host: a.imap.host.clone(),
            imap_port: a.imap.port.to_string(),
            imap_user: a.imap.username.clone(),
            imap_pass: a.imap.password.clone(),
            imap_mailbox: a.imap.mailbox.clone(),
            imap_fetch: a.imap.fetch_count.to_string(),
            smtp_host: a.smtp.host.clone(),
            smtp_port: a.smtp.port.to_string(),
            smtp_user: a.smtp.username.clone(),
            smtp_pass: a.smtp.password.clone(),
            smtp_from: a.smtp.from_address.clone(),
        }).collect(),
    }
}

pub fn save_config(
    display_name: &str,
    accounts: &[AccountDto],
    claude_key: &str,
    claude_model: &str,
    giphy_key: &str,
    font_scale: f32,
    theme: Option<&str>,
    language: Option<&str>,
) -> Result<()> {
    let mut cfg = Config::load()?;

    cfg.user = UserConfig {
        display_name: display_name.to_string(),
        font_scale,
        theme: theme.map(str::to_string).unwrap_or_else(|| cfg.user.theme.clone()),
        language: language
            .filter(|l| *l == "en" || *l == "fr")
            .map(str::to_string)
            .unwrap_or_else(|| cfg.user.language.clone()),
    };

    cfg.accounts = accounts.iter().map(|a| AccountConfig {
        name: a.label.clone(),
        imap: ImapConfig {
            host: a.imap_host.clone(),
            port: a.imap_port.parse().unwrap_or(993),
            username: a.imap_user.clone(),
            password: a.imap_pass.clone(),
            mailbox: a.imap_mailbox.clone(),
            fetch_count: a.imap_fetch.parse().unwrap_or(50),
        },
        smtp: SmtpConfig {
            host: a.smtp_host.clone(),
            port: a.smtp_port.parse().unwrap_or(587),
            username: a.smtp_user.clone(),
            password: a.smtp_pass.clone(),
            from_address: a.smtp_from.clone(),
        },
    }).collect();

    cfg.claude = ClaudeConfig {
        api_key: claude_key.to_string(),
        model: claude_model.to_string(),
        max_tokens: cfg.claude.max_tokens,
    };
    cfg.gifs.giphy_api_key = giphy_key.trim().to_string();

    crate::config::save_config(&cfg)
}

pub fn generate_qr(cfg: &Config) -> String {
    let mut lines = vec![format!("courriel://import name={}", cfg.user.display_name)];
    for (i, a) in cfg.accounts.iter().enumerate() {
        lines.push(format!(
            "account{} label={} imap={}:{}@{}:{}/{} smtp={}:{}@{}:{} from={}",
            i, a.name,
            a.imap.username, a.imap.password,
            a.imap.host, a.imap.port, a.imap.mailbox,
            a.smtp.username, a.smtp.password,
            a.smtp.host, a.smtp.port,
            a.smtp.from_address,
        ));
    }
    let payload = lines.join("\n");
    match QrCode::new(payload.as_bytes()) {
        Ok(code) => code
            .render::<unicode::Dense1x2>()
            .dark_color(unicode::Dense1x2::Dark)
            .light_color(unicode::Dense1x2::Light)
            .build(),
        Err(e) => format!("QR error: {}", e),
    }
}
