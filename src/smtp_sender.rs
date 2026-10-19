use anyhow::{Context, Result};
use lettre::{
    message::{header::ContentType, Mailbox},
    transport::smtp::authentication::Credentials,
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};
use tracing::info;

use crate::config::SmtpConfig;

/// Test SMTP connectivity and auth without sending any email.
pub async fn check_connection(cfg: &SmtpConfig) -> Result<()> {
    let creds = Credentials::new(cfg.username.clone(), cfg.password.clone());
    if cfg.port == 465 {
        let transport: AsyncSmtpTransport<Tokio1Executor> =
            AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host)
                .context("Failed to create SMTP transport")?
                .port(cfg.port)
                .credentials(creds)
                .build();
        transport.test_connection().await.context("SMTP connection/auth failed")?;
    } else {
        let transport: AsyncSmtpTransport<Tokio1Executor> =
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host)
                .context("Failed to create SMTP transport")?
                .port(cfg.port)
                .credentials(creds)
                .build();
        transport.test_connection().await.context("SMTP connection/auth failed")?;
    }
    Ok(())
}

pub async fn send_reply(
    cfg: &SmtpConfig,
    to: &str,
    subject: &str,
    body: &str,
) -> Result<()> {
    info!("Sending email to {}", to);

    let from_mailbox: Mailbox = cfg
        .from_address
        .parse()
        .context("Invalid from_address in config")?;

    let to_mailbox: Mailbox = to
        .parse()
        .context("Invalid recipient address")?;

    let email = Message::builder()
        .from(from_mailbox)
        .to(to_mailbox)
        .subject(subject)
        .header(ContentType::TEXT_PLAIN)
        .body(body.to_string())
        .context("Failed to build email message")?;

    let creds = Credentials::new(cfg.username.clone(), cfg.password.clone());

    // Port 465 = implicit TLS (SSL); port 587 = STARTTLS.
    let transport = if cfg.port == 465 {
        AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host)
            .context("Failed to create SMTP transport")?
            .port(cfg.port)
            .credentials(creds)
            .build()
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host)
            .context("Failed to create SMTP transport")?
            .port(cfg.port)
            .credentials(creds)
            .build()
    };

    transport
        .send(email)
        .await
        .context("SMTP send failed")?;

    info!("Email sent successfully");
    Ok(())
}
