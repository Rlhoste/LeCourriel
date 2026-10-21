use anyhow::Result;
use qrcode::QrCode;
use qrcode::render::unicode;

use crate::config::{AccountConfig, Config, ImapConfig, SignatureConfig, SmtpConfig, UserConfig, AI_PROVIDERS};
use crate::dto::{AccountDto, AiSettingsDto, SettingsDto};

pub fn config_to_dto(cfg: &Config) -> SettingsDto {
    SettingsDto {
        display_name: cfg.user.display_name.clone(),
        font_scale: cfg.user.font_scale,
        theme: cfg.user.theme.clone(),
        language: cfg.user.language.clone(),
        ai: AiSettingsDto {
            provider: cfg.ai.provider().to_string(),
            model: cfg.ai.model.clone(),
            base_url: cfg.ai.base_url.clone(),
            keys: cfg.ai.keys.clone(),
        },
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
            signature_file: a.signature.file.clone(),
            signature_on_new: a.signature.on_new,
            signature_on_reply: a.signature.on_reply,
        }).collect(),
    }
}

pub fn save_config(
    display_name: Option<&str>,
    accounts: &[AccountDto],
    ai: &AiSettingsDto,
    giphy_key: &str,
    font_scale: f32,
    theme: Option<&str>,
    language: Option<&str>,
) -> Result<()> {
    let mut cfg = Config::load()?;

    cfg.user = UserConfig {
        // Not in Settings any more: keep the stored name (it may come from config.toml or the cloud).
        display_name: display_name.map(str::to_string).unwrap_or_else(|| cfg.user.display_name.clone()),
        font_scale,
        theme: theme.map(str::to_string).unwrap_or_else(|| cfg.user.theme.clone()),
        language: language
            .filter(|l| crate::i18n::is_supported(l))
            .map(str::to_string)
            .unwrap_or_else(|| cfg.user.language.clone()),
    };

    cfg.accounts = accounts.iter().map(account_from_dto).collect();

    if AI_PROVIDERS.contains(&ai.provider.as_str()) {
        cfg.ai.provider = ai.provider.clone();
    }
    cfg.ai.model = ai.model.trim().to_string();
    cfg.ai.base_url = ai.base_url.trim().to_string();
    cfg.ai.keys = ai.keys.iter()
        .map(|(p, k)| (p.clone(), k.trim().to_string()))
        .filter(|(p, k)| AI_PROVIDERS.contains(&p.as_str()) && !k.is_empty())
        .collect();
    cfg.gifs.giphy_api_key = giphy_key.trim().to_string();

    crate::config::save_config(&cfg)
}

/// An account as typed in Settings. Blank ports and advanced options mean the defaults.
pub fn account_from_dto(a: &AccountDto) -> AccountConfig {
    AccountConfig {
        name: a.label.clone(),
        imap: ImapConfig {
            host: a.imap_host.clone(),
            port: a.imap_port.trim().parse().unwrap_or(993),
            username: a.imap_user.clone(),
            password: a.imap_pass.clone(),
            mailbox: Some(a.imap_mailbox.trim()).filter(|m| !m.is_empty()).unwrap_or("INBOX").to_string(),
            fetch_count: a.imap_fetch.trim().parse().unwrap_or(50),
        },
        smtp: SmtpConfig {
            host: a.smtp_host.clone(),
            port: a.smtp_port.trim().parse().unwrap_or(587),
            username: a.smtp_user.clone(),
            password: a.smtp_pass.clone(),
            from_address: a.smtp_from.clone(),
        },
        signature: SignatureConfig {
            file: a.signature_file.clone(),
            on_new: a.signature_on_new,
            on_reply: a.signature_on_reply,
        },
    }
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
