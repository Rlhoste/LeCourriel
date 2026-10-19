use anyhow::{Context, Result};
use async_imap::types::Fetch;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use mailparse::{parse_mail, MailHeaderMap};
use native_tls::TlsConnector;
use tracing::{debug, info, warn};

use crate::config::ImapConfig;
use crate::models::RawEmail;

pub async fn fetch_folder(cfg: &ImapConfig, mailbox: &str) -> Result<Vec<RawEmail>> {
    let mut override_cfg = cfg.clone();
    override_cfg.mailbox = mailbox.to_string();
    fetch_emails(&override_cfg).await
}

pub async fn fetch_emails(cfg: &ImapConfig) -> Result<Vec<RawEmail>> {
    info!("Connecting to IMAP {}:{}", cfg.host, cfg.port);

    let tls = TlsConnector::builder()
        .min_protocol_version(Some(native_tls::Protocol::Tlsv10))
        .build()
        .context("Failed to build TLS connector")?;
    let tls = tokio_native_tls::TlsConnector::from(tls);

    let tcp = tokio::net::TcpStream::connect((cfg.host.as_str(), cfg.port))
        .await
        .context("TCP connection to IMAP server failed")?;

    let tls_stream = tls
        .connect(&cfg.host, tcp)
        .await
        .context("TLS handshake with IMAP server failed")?;

    let client = async_imap::Client::new(tls_stream);

    let mut session = client
        .login(&cfg.username, &cfg.password)
        .await
        .map_err(|(e, _)| e)
        .context("IMAP login failed — check credentials / App Password")?;

    let mailbox = session
        .select(&cfg.mailbox)
        .await
        .context("IMAP SELECT mailbox failed")?;

    let total = mailbox.exists;
    if total == 0 {
        session.logout().await.ok();
        return Ok(vec![]);
    }

    let start = if total > cfg.fetch_count {
        total - cfg.fetch_count + 1
    } else {
        1
    };
    let seq_set = format!("{}:{}", start, total);

    info!("Fetching messages {} of total {}", seq_set, total);

    let mut messages_stream = session
        .fetch(&seq_set, "RFC822")
        .await
        .context("IMAP FETCH command failed")?;

    let mut messages: Vec<Fetch> = Vec::new();
    while let Some(msg) = messages_stream.next().await {
        match msg {
            Ok(fetch) => messages.push(fetch),
            Err(e) => warn!("Skipping malformed fetch result: {}", e),
        }
    }
    drop(messages_stream);

    info!("Received {} raw messages", messages.len());

    let mut emails: Vec<RawEmail> = messages
        .iter()
        .filter_map(|fetch| parse_fetch(fetch))
        .collect();

    emails.sort_by(|a, b| match (&b.date, &a.date) {
        (Some(bd), Some(ad)) => bd.cmp(ad),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => b.uid.cmp(&a.uid),
    });

    session.logout().await.ok();

    Ok(emails)
}

fn parse_fetch(fetch: &Fetch) -> Option<RawEmail> {
    let raw = fetch.body()?;
    let parsed = parse_mail(raw).ok()?;

    let uid = fetch.uid.unwrap_or(fetch.message);

    let subject = parsed
        .headers
        .get_first_value("Subject")
        .unwrap_or_default()
        .trim()
        .to_string();

    let from = parsed
        .headers
        .get_first_value("From")
        .unwrap_or_default()
        .trim()
        .to_string();

    let date_str = parsed.headers.get_first_value("Date").unwrap_or_default();
    let date: Option<DateTime<Utc>> = mailparse::dateparse(&date_str)
        .ok()
        .and_then(|ts| DateTime::from_timestamp(ts, 0));

    let (body_text, body_html) = extract_bodies(&parsed);

    debug!("Parsed email uid={} subject={:?}", uid, subject);

    Some(RawEmail {
        uid,
        subject,
        from,
        date,
        body_text,
        body_html,
    })
}

fn extract_bodies(mail: &mailparse::ParsedMail) -> (String, Option<String>) {
    let mut plain = String::new();
    let mut html: Option<String> = None;
    collect_parts(mail, &mut plain, &mut html);
    if plain.is_empty() {
        if let Ok(body) = mail.get_body() {
            plain = body;
        }
    }
    (plain, html)
}

fn collect_parts(mail: &mailparse::ParsedMail, plain: &mut String, html: &mut Option<String>) {
    let ctype = mail.ctype.mimetype.to_lowercase();
    if ctype == "text/plain" {
        if let Ok(body) = mail.get_body() {
            if plain.is_empty() {
                *plain = body;
            }
        }
    } else if ctype == "text/html" {
        if html.is_none() {
            if let Ok(body) = mail.get_body() {
                *html = Some(body);
            }
        }
    } else {
        for sub in &mail.subparts {
            collect_parts(sub, plain, html);
        }
    }
}
