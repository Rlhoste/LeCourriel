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

/// Log in and open the configured folder, as a settings check. Returns how many
/// messages the folder holds.
pub async fn check_connection(cfg: &ImapConfig) -> Result<u32> {
    let mut session = connect(cfg).await?;
    let mailbox = session
        .select(&cfg.mailbox)
        .await
        .with_context(|| format!("Folder \"{}\" not found", cfg.mailbox))?;
    session.logout().await.ok();
    Ok(mailbox.exists)
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
            .with_context(|| format!("Could not move the email to {} (MOVE: {})", folder, e))?;
        {
            let stream = session
                .uid_store(&uid_set, "+FLAGS (\\Deleted)")
                .await
                .context("IMAP STORE \\Deleted failed")?;
            futures::pin_mut!(stream);
            while stream.next().await.is_some() {}
        }
        // A plain EXPUNGE would permanently erase *every* message flagged \Deleted in the
        // mailbox (including ones another client flagged). Only purge this one, with UID
        // EXPUNGE (UIDPLUS); otherwise leave it flagged — it is already copied, and flagged
        // messages are hidden by the app and by other clients.
        let uidplus = session.capabilities().await.map_or(false, |c| c.has_str("UIDPLUS"));
        if uidplus {
            let stream = session.uid_expunge(&uid_set).await.context("IMAP UID EXPUNGE failed")?;
            futures::pin_mut!(stream);
            while stream.next().await.is_some() {}
        } else {
            info!("Server has no UIDPLUS: uid {} left flagged \\Deleted instead of expunging the mailbox", uid);
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

/// Quote a string for an IMAP SEARCH command.
fn imap_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The IMAP SEARCH criteria for these words: every word must appear in the
/// subject, the sender or the text; optionally only emails since a date.
pub fn search_criteria(terms: &[String], since: Option<chrono::NaiveDate>) -> String {
    let mut parts: Vec<String> = terms
        .iter()
        .take(4)
        .map(|t| {
            let q = imap_quote(t);
            format!("OR OR SUBJECT {q} FROM {q} TEXT {q}")
        })
        .collect();
    if let Some(d) = since {
        parts.push(format!("SINCE {}", d.format("%d-%b-%Y")));
    }
    if parts.is_empty() {
        parts.push("ALL".into());
    }
    let criteria = parts.join(" ");
    if criteria.is_ascii() { criteria } else { format!("CHARSET UTF-8 {}", criteria) }
}

/// Search the whole mailbox on the server (not just the cached emails).
/// Returns the newest `limit` matches, without marking them read.
pub async fn search_server(
    cfg: &ImapConfig,
    terms: &[String],
    since: Option<chrono::NaiveDate>,
    limit: usize,
) -> Result<Vec<RawEmail>> {
    let mut session = connect(cfg).await?;
    session
        .select(&cfg.mailbox)
        .await
        .context("IMAP SELECT mailbox failed")?;
    let mut uids: Vec<u32> = session
        .uid_search(search_criteria(terms, since))
        .await
        .context("IMAP SEARCH failed")?
        .into_iter()
        .collect();
    uids.sort_unstable_by(|a, b| b.cmp(a));
    uids.truncate(limit);
    let mut emails = Vec::new();
    if !uids.is_empty() {
        let set = uids.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",");
        let stream = session
            .uid_fetch(&set, "(UID FLAGS BODY.PEEK[])")
            .await
            .context("IMAP FETCH failed")?;
        futures::pin_mut!(stream);
        while let Some(item) = stream.next().await {
            if let Some(email) = item.ok().as_ref().and_then(parse_fetch) {
                if !email.deleted {
                    emails.push(email);
                }
            }
        }
    }
    session.logout().await.ok();
    emails.sort_by(|a, b| b.date.cmp(&a.date));
    Ok(emails)
}

/// Make sure a folder exists, creating (and subscribing to) it if needed.
pub async fn ensure_folder(cfg: &ImapConfig, folder: &str) -> Result<()> {
    let mut session = connect(cfg).await?;
    let exists = {
        let stream = session.list(Some(""), Some(folder)).await.context("IMAP LIST failed")?;
        futures::pin_mut!(stream);
        let mut found = false;
        while let Some(item) = stream.next().await {
            if item.map_or(false, |n| n.name() == folder) {
                found = true;
            }
        }
        found
    };
    if !exists {
        info!("Creating folder {}", folder);
        session.create(folder).await.with_context(|| format!("Could not create the folder {}", folder))?;
        session.subscribe(folder).await.ok();
    }
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
        // Flagged \Deleted (moved away, waiting for a purge): not in this folder any more.
        .filter(|e: &RawEmail| !e.deleted)
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
    let deleted = fetch.flags().any(|f| matches!(f, async_imap::types::Flag::Deleted));

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
    let references = format!("{} {}", header("In-Reply-To"), header("References")).trim().to_string();

    let date_str = parsed.headers.get_first_value("Date").unwrap_or_default();
    let date: Option<DateTime<Utc>> = mailparse::dateparse(&date_str)
        .ok()
        .and_then(|ts| DateTime::from_timestamp(ts, 0));

    let (body_text, body_html) = extract_bodies(&parsed);

    debug!("Parsed email uid={} subject={:?}", uid, subject);

    Some(RawEmail {
        uid: uid.into(),   // the caller adds the account (`mail_id`)
        subject,
        from,
        to,
        cc,
        reply_to,
        bcc,
        message_id,
        references,
        seen,
        deleted,
        date,
        body_text,
        body_html,
    })
}

/// Inline images bigger than this are left out (they would bloat the cache).
const MAX_INLINE_IMAGE: usize = 3 * 1024 * 1024;

pub(crate) fn extract_bodies(mail: &mailparse::ParsedMail) -> (String, Option<String>) {
    let mut plain = String::new();
    let mut html: Option<String> = None;
    let mut inline = Vec::new();
    collect_parts(mail, &mut plain, &mut html, &mut inline);
    if plain.is_empty() {
        if let Ok(body) = decode_text_body(mail) {
            plain = body;
        }
    }
    (plain, html.map(|h| embed_inline_images(h, inline)))
}

/// Inline images (signatures, logos) are attached parts that the HTML points to with
/// `cid:<Content-ID>`. The reading pane cannot fetch those, so put each image in the
/// HTML itself as a data: URL.
fn embed_inline_images(mut html: String, mut inline: Vec<(String, String)>) -> String {
    // Longest first, so "cid:a@b" is not replaced inside "cid:a@b.c".
    inline.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    for (cid, data_url) in inline {
        for prefix in ["cid:", "CID:", "Cid:"] {
            html = html.replace(&format!("{prefix}{cid}"), &data_url);
        }
    }
    html
}

/// Decode a transfer-decoded text part using its declared charset. A surprising
/// number of messages omit the charset while still containing Windows-1252 bytes
/// (especially French mail from older Outlook versions), so try UTF-8 and then
/// Windows-1252 instead of mailparse's lossy US-ASCII default.
pub(crate) fn decode_text_body(mail: &mailparse::ParsedMail) -> Result<String, mailparse::MailParseError> {
    let raw = mail.get_body_raw()?;
    if let Some(encoding) = mail
        .ctype
        .params
        .get("charset")
        .and_then(|label| encoding_rs::Encoding::for_label(label.trim().as_bytes()))
    {
        let (text, _, had_errors) = encoding.decode(&raw);
        if !had_errors {
            return Ok(text.trim_start_matches('\u{feff}').to_string());
        }
    }

    let (utf8, _, had_errors) = encoding_rs::UTF_8.decode(&raw);
    if !had_errors {
        return Ok(utf8.trim_start_matches('\u{feff}').to_string());
    }
    let (windows, _, _) = encoding_rs::WINDOWS_1252.decode(&raw);
    Ok(windows.into_owned())
}

fn collect_parts(
    mail: &mailparse::ParsedMail,
    plain: &mut String,
    html: &mut Option<String>,
    inline: &mut Vec<(String, String)>,
) {
    let ctype = mail.ctype.mimetype.to_lowercase();
    if ctype == "text/plain" {
        if let Ok(body) = decode_text_body(mail) {
            if plain.is_empty() {
                *plain = body;
            }
        }
    } else if ctype == "text/html" {
        if html.is_none() {
            if let Ok(body) = decode_text_body(mail) {
                *html = Some(body);
            }
        }
    } else if mail.subparts.is_empty() {
        let cid = mail.headers.get_first_value("Content-ID").unwrap_or_default();
        let cid = cid.trim().trim_start_matches('<').trim_end_matches('>').trim();
        if cid.is_empty() {
            return;
        }
        if let Ok(data) = mail.get_body_raw() {
            if !data.is_empty() && data.len() <= MAX_INLINE_IMAGE {
                use base64::Engine;
                let mime = if ctype.starts_with("image/") { ctype.as_str() } else { "application/octet-stream" };
                let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
                inline.push((cid.to_string(), format!("data:{mime};base64,{encoded}")));
            }
        }
    } else {
        for sub in &mail.subparts {
            collect_parts(sub, plain, html, inline);
        }
    }
}
