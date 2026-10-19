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

pub type ImapSession = async_imap::Session<tokio_native_tls::TlsStream<tokio::net::TcpStream>>;

/// Open a TLS connection to the IMAP server and log in.
pub async fn connect(cfg: &ImapConfig) -> Result<ImapSession> {
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

    client
        .login(&cfg.username, &cfg.password)
        .await
        .map_err(|(e, _)| e)
        .context("IMAP login failed — check credentials / App Password")
}

/// Move one message (by real UID) from the configured mailbox to `folder` (Trash, Junk…).
pub async fn move_message(cfg: &ImapConfig, uid: u32, folder: &str) -> Result<()> {
    let mut session = connect(cfg).await?;
    session
        .select(&cfg.mailbox)
        .await
        .context("IMAP SELECT mailbox failed")?;

    let uid_set = uid.to_string();
    if let Err(e) = session.uid_mv(&uid_set, folder).await {
        // Server without the MOVE extension: copy, flag deleted, expunge.
        warn!("UID MOVE failed ({}), falling back to COPY + EXPUNGE", e);
        session
            .uid_copy(&uid_set, folder)
            .await
            .context("IMAP COPY failed")?;
        {
            let stream = session
                .uid_store(&uid_set, "+FLAGS (\\Deleted)")
                .await
                .context("IMAP STORE \\Deleted failed")?;
            futures::pin_mut!(stream);
            while stream.next().await.is_some() {}
        }
        {
            let stream = session.expunge().await.context("IMAP EXPUNGE failed")?;
            futures::pin_mut!(stream);
            while stream.next().await.is_some() {}
        }
    }

    info!("Moved uid {} to {}", uid, folder);
    session.logout().await.ok();
    Ok(())
}

/// Set or clear the \Seen flag on one message (by real UID) in the configured mailbox.
pub async fn set_seen(cfg: &ImapConfig, uid: u32, seen: bool) -> Result<()> {
    let mut session = connect(cfg).await?;
    session
        .select(&cfg.mailbox)
        .await
        .context("IMAP SELECT mailbox failed")?;
    {
        let stream = session
            .uid_store(uid.to_string(), if seen { "+FLAGS (\\Seen)" } else { "-FLAGS (\\Seen)" })
            .await
            .context("IMAP STORE \\Seen failed")?;
        futures::pin_mut!(stream);
        while stream.next().await.is_some() {}
    }
    session.logout().await.ok();
    Ok(())
}

/// The exact original message (RFC 822 bytes) of one inbox email, without marking it read.
pub async fn fetch_raw(cfg: &ImapConfig, uid: u32) -> Result<Vec<u8>> {
    let mut session = connect(cfg).await?;
    session
        .select(&cfg.mailbox)
        .await
        .context("IMAP SELECT mailbox failed")?;
    let raw = {
        let stream = session
            .uid_fetch(uid.to_string(), "BODY.PEEK[]")
            .await
            .context("IMAP FETCH failed")?;
        futures::pin_mut!(stream);
        let mut raw = None;
        while let Some(item) = stream.next().await {
            if let Ok(fetch) = item {
                if let Some(body) = fetch.body() {
                    raw = Some(body.to_vec());
                }
            }
        }
        raw
    };
    session.logout().await.ok();
    raw.with_context(|| format!("Message uid {} not found on the server", uid))
}

/// Store a copy of a sent message in `folder`, flagged as read.
pub async fn append_sent(cfg: &ImapConfig, folder: &str, raw: &[u8]) -> Result<()> {
    let mut session = connect(cfg).await?;
    session
        .append(folder, Some("(\\Seen)"), None, raw)
        .await
        .with_context(|| format!("IMAP APPEND to {} failed", folder))?;
    session.logout().await.ok();
    Ok(())
}

pub async fn fetch_emails(cfg: &ImapConfig) -> Result<Vec<RawEmail>> {
    let mut session = connect(cfg).await?;

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
        // BODY.PEEK[] (not RFC822) so fetching does not mark messages \Seen.
        .fetch(&seq_set, "(UID FLAGS BODY.PEEK[])")
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
    let seen = fetch.flags().any(|f| matches!(f, async_imap::types::Flag::Seen));

    let subject = parsed
        .headers
        .get_first_value("Subject")
        .unwrap_or_default()
        .trim()
        .to_string();

    let header = |name: &str| {
        parsed
            .headers
            .get_first_value(name)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let from = header("From");
    let to = header("To");
    let cc = header("Cc");
    let reply_to = header("Reply-To");
    let bcc = header("Bcc");
    let message_id = header("Message-ID");

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
        to,
        cc,
        reply_to,
        bcc,
        message_id,
        seen,
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
