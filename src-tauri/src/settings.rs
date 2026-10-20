use anyhow::Result;
use qrcode::QrCode;
use qrcode::render::unicode;

use crate::config::{AccountConfig, Config, ImapConfig, SmtpConfig, UserConfig, AI_PROVIDERS};
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
        signature: cfg.signature.clone(),
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
    ai: &AiSettingsDto,
    giphy_key: &str,
    signature: Option<&crate::config::SignatureConfig>,
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
            .filter(|l| crate::i18n::is_supported(l))
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
    if let Some(sig) = signature {
        cfg.signature = sig.clone();
    }

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
