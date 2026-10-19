use anyhow::Result;
use qrcode::QrCode;
use qrcode::render::unicode;

use crate::config::{AccountConfig, ClaudeConfig, Config, ImapConfig, SmtpConfig, UserConfig};
use crate::AccountData;

/// Convert a Slint AccountData list + global fields into a Config and save.
pub fn save_config(
    display_name: &str,
    accounts: &[AccountData],
    claude_key: &str,
    claude_model: &str,
    font_scale: f32,
) -> Result<()> {
    let mut cfg = Config::load()?;

    cfg.user = UserConfig { display_name: display_name.to_string(), font_scale };

    cfg.accounts = accounts.iter().map(|a| AccountConfig {
        name: a.label.to_string(),
        imap: ImapConfig {
            host: a.imap_host.to_string(),
            port: a.imap_port.parse().unwrap_or(993),
            username: a.imap_user.to_string(),
            password: a.imap_pass.to_string(),
            mailbox: a.imap_mailbox.to_string(),
            fetch_count: a.imap_fetch.parse().unwrap_or(50),
        },
        smtp: SmtpConfig {
            host: a.smtp_host.to_string(),
            port: a.smtp_port.parse().unwrap_or(587),
            username: a.smtp_user.to_string(),
            password: a.smtp_pass.to_string(),
            from_address: a.smtp_from.to_string(),
        },
    }).collect();

    cfg.claude = ClaudeConfig {
        api_key: claude_key.to_string(),
        model: claude_model.to_string(),
        max_tokens: cfg.claude.max_tokens,
    };

    crate::config::save_config(&cfg)
}

/// Build the Slint AccountData list from the current config.
pub fn accounts_to_slint(cfg: &Config) -> Vec<AccountData> {
    use slint::SharedString;
    cfg.accounts.iter().map(|a| AccountData {
        label: SharedString::from(a.name.as_str()),
        imap_host: SharedString::from(a.imap.host.as_str()),
        imap_port: SharedString::from(a.imap.port.to_string()),
        imap_user: SharedString::from(a.imap.username.as_str()),
        imap_pass: SharedString::from(a.imap.password.as_str()),
        imap_mailbox: SharedString::from(a.imap.mailbox.as_str()),
        imap_fetch: SharedString::from(a.imap.fetch_count.to_string()),
        smtp_host: SharedString::from(a.smtp.host.as_str()),
        smtp_port: SharedString::from(a.smtp.port.to_string()),
        smtp_user: SharedString::from(a.smtp.username.as_str()),
        smtp_pass: SharedString::from(a.smtp.password.as_str()),
        smtp_from: SharedString::from(a.smtp.from_address.as_str()),
    }).collect()
}

/// Render all accounts as a Unicode QR block grid.
pub fn generate_qr(cfg: &Config) -> String {
    let mut lines = vec![format!("courriel://import name={}", cfg.user.display_name)];
    for (i, a) in cfg.accounts.iter().enumerate() {
        lines.push(format!(
            "account{} label={} imap={}:{}@{}:{}/{} smtp={}:{}@{}:{} from={}",
            i,
            a.name,
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
