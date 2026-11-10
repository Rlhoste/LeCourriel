use anyhow::{Context, Result};
use async_imap::types::Fetch;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use mailparse::{parse_mail, DispositionType, MailHeaderMap};
use native_tls::TlsConnector;
use tracing::{debug, info, warn};

use crate::config::ImapConfig;
use crate::models::RawEmail;

/// Result of the cheap Inbox synchronization used by routine refreshes.
/// `reset` means the server's UID namespace changed (or the local cursor was
/// impossible), so `emails` is a fresh recent window rather than only arrivals.
#[derive(Debug)]
pub struct InboxDelta {
    pub emails: Vec<RawEmail>,
    pub uid_validity: Option<u32>,
    pub highest_uid: u32,
    pub reset: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InboxFetchPlan {
    None,
    NewerThan(u32),
    RecentWindow,
}

fn inbox_fetch_plan(
    saved_validity: Option<u32>,
    saved_highest: Option<u32>,
    server_validity: Option<u32>,
    server_next: Option<u32>,
) -> (InboxFetchPlan, bool) {
    let validity_changed = saved_validity.is_some() && server_validity.is_some() && saved_validity != server_validity;
    let impossible_cursor = saved_highest.zip(server_next).is_some_and(|(highest, next)| highest >= next);
    if validity_changed || impossible_cursor {
        return (InboxFetchPlan::RecentWindow, true);
    }
    let Some(highest) = saved_highest else {
        return (InboxFetchPlan::RecentWindow, false);
    };
    if server_next.is_some_and(|next| next <= highest.saturating_add(1)) {
        (InboxFetchPlan::None, false)
    } else {
        (InboxFetchPlan::NewerThan(highest), false)
    }
}

pub async fn fetch_folder(cfg: &ImapConfig, mailbox: &str) -> Result<Vec<RawEmail>> {
    let mut override_cfg = cfg.clone();
    override_cfg.mailbox = mailbox.to_string();
    fetch_emails(&override_cfg).await
}

/// Fetch every message header needed for contact discovery, without downloading bodies or
/// changing read state. Used once per account/folder, then incremental refresh handles new mail.
pub async fn fetch_all_contact_headers(cfg: &ImapConfig, mailbox: &str) -> Result<Vec<RawEmail>> {
    let mut session = connect(cfg).await?;
    let selected = session.select(mailbox).await.context("IMAP SELECT contact scan folder failed")?;
    let mut emails = Vec::new();
    const BATCH: u32 = 500;
    let fields = "(UID FLAGS BODY.PEEK[HEADER.FIELDS (FROM TO CC BCC MESSAGE-ID DATE AUTHENTICATION-RESULTS ARC-AUTHENTICATION-RESULTS X-SPAM-FLAG X-SPAM-STATUS X-SPAM-SCORE X-MS-EXCHANGE-ORGANIZATION-SCL LIST-ID LIST-UNSUBSCRIBE PRECEDENCE)])";
    let mut start = 1;
    while start <= selected.exists {
        let end = start.saturating_add(BATCH - 1).min(selected.exists);
        let mut stream =
            session.fetch(format!("{start}:{end}"), fields).await.context("IMAP contact header scan failed")?;
        while let Some(message) = stream.next().await {
            match message {
                Ok(fetch) => {
                    if let Some(email) = parse_fetch(&fetch) {
                        if !email.deleted {
                            emails.push(email);
                        }
                    }
                }
                Err(error) => warn!("Skipping malformed contact header: {}", error),
            }
        }
        drop(stream);
        start = end.saturating_add(1);
    }
    session.logout().await.ok();
    Ok(emails)
}

/// One-time bounded body scan for signature details. At most the first 64 KiB of message text is
/// requested; attachments and the remainder of long threads are not downloaded.
pub async fn fetch_all_signature_messages(cfg: &ImapConfig, mailbox: &str) -> Result<Vec<RawEmail>> {
    let mut session = connect(cfg).await?;
    let selected = session.select(mailbox).await.context("IMAP SELECT signature scan folder failed")?;
    let mut emails = Vec::new();
    const BATCH: u32 = 200;
    let mut start = 1;
    while start <= selected.exists {
        let end = start.saturating_add(BATCH - 1).min(selected.exists);
        let mut stream = session
            .fetch(format!("{start}:{end}"), "(UID FLAGS BODY.PEEK[HEADER] BODY.PEEK[TEXT]<0.65536>)")
            .await
            .context("IMAP signature scan failed")?;
        while let Some(message) = stream.next().await {
            match message {
                Ok(fetch) => {
                    // The E2E server returns BODY[] for every FETCH; real servers return the
                    // requested separate header/text sections.
                    let parsed =
                        if fetch.body().is_some() { parse_fetch(&fetch) } else { parse_header_and_text(&fetch) };
                    if let Some(email) = parsed {
                        if !email.deleted {
                            emails.push(email);
                        }
                    }
                }
                Err(error) => warn!("Skipping malformed signature scan result: {}", error),
            }
        }
        drop(stream);
        start = end.saturating_add(1);
    }
    session.logout().await.ok();
    Ok(emails)
}

pub type ImapSession = async_imap::Session<tokio_native_tls::TlsStream<tokio::net::TcpStream>>;

/// Open a TLS connection to the IMAP server and log in.
pub async fn connect(cfg: &ImapConfig) -> Result<ImapSession> {
    info!("Connecting to IMAP {}:{}", cfg.host, cfg.port);

    let tls = TlsConnector::builder()
        .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
        .build()
        .context("Failed to build TLS connector")?;
    let tls = tokio_native_tls::TlsConnector::from(tls);

    let tcp = tokio::net::TcpStream::connect((cfg.host.as_str(), cfg.port))
        .await
        .context("TCP connection to IMAP server failed")?;

    let tls_stream = tls.connect(&cfg.host, tcp).await.context("TLS handshake with IMAP server failed")?;

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
    let mailbox =
        session.select(&cfg.mailbox).await.with_context(|| format!("Folder \"{}\" not found", cfg.mailbox))?;
    session.logout().await.ok();
    Ok(mailbox.exists)
}

/// Move one message (by real UID) from the configured mailbox to `folder` (Trash, Junk…).
pub async fn move_message(cfg: &ImapConfig, uid: u32, folder: &str) -> Result<()> {
    let mut session = connect(cfg).await?;
    session.select(&cfg.mailbox).await.context("IMAP SELECT mailbox failed")?;

    let uid_set = uid.to_string();
    if let Err(e) = session.uid_mv(&uid_set, folder).await {
        // Server without the MOVE extension: copy, flag deleted, expunge.
        warn!("UID MOVE failed ({}), falling back to COPY + EXPUNGE", e);
        session
            .uid_copy(&uid_set, folder)
            .await
            .with_context(|| format!("Could not move the email to {} (MOVE: {})", folder, e))?;
        {
            let stream =
                session.uid_store(&uid_set, "+FLAGS (\\Deleted)").await.context("IMAP STORE \\Deleted failed")?;
            futures::pin_mut!(stream);
            while stream.next().await.is_some() {}
        }
        // A plain EXPUNGE would permanently erase *every* message flagged \Deleted in the
        // mailbox (including ones another client flagged). Only purge this one, with UID
        // EXPUNGE (UIDPLUS); otherwise leave it flagged — it is already copied, and flagged
        // messages are hidden by the app and by other clients.
        let uidplus = session.capabilities().await.is_ok_and(|c| c.has_str("UIDPLUS"));
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
    session.select(&cfg.mailbox).await.context("IMAP SELECT mailbox failed")?;
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
    session.select(&cfg.mailbox).await.context("IMAP SELECT mailbox failed")?;
    let raw = {
        let stream = session.uid_fetch(uid.to_string(), "BODY.PEEK[]").await.context("IMAP FETCH failed")?;
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
    if criteria.is_ascii() {
        criteria
    } else {
        format!("CHARSET UTF-8 {}", criteria)
    }
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
    session.select(&cfg.mailbox).await.context("IMAP SELECT mailbox failed")?;
    let mut uids: Vec<u32> =
        session.uid_search(search_criteria(terms, since)).await.context("IMAP SEARCH failed")?.into_iter().collect();
    uids.sort_unstable_by(|a, b| b.cmp(a));
    uids.truncate(limit);
    let mut emails = Vec::new();
    if !uids.is_empty() {
        let set = uids.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",");
        let stream = session.uid_fetch(&set, "(UID FLAGS BODY.PEEK[])").await.context("IMAP FETCH failed")?;
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
            if item.is_ok_and(|n| n.name() == folder) {
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

    let mailbox = session.select(&cfg.mailbox).await.context("IMAP SELECT mailbox failed")?;

    let total = mailbox.exists;
    if total == 0 {
        session.logout().await.ok();
        return Ok(vec![]);
    }

    let start = if total > cfg.fetch_count { total - cfg.fetch_count + 1 } else { 1 };
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
        .filter_map(parse_fetch)
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

/// Select Inbox and download only messages whose UID is newer than the saved
/// cursor. With no usable cursor (first run or UIDVALIDITY reset), download the
/// configured recent window once. A no-change check performs no FETCH at all.
pub async fn fetch_inbox_delta(
    cfg: &ImapConfig,
    saved_validity: Option<u32>,
    saved_highest: Option<u32>,
) -> Result<InboxDelta> {
    let mut session = connect(cfg).await?;
    let mailbox = session.select(&cfg.mailbox).await.context("IMAP SELECT mailbox failed")?;
    let (mut plan, reset) = inbox_fetch_plan(saved_validity, saved_highest, mailbox.uid_validity, mailbox.uid_next);

    // UIDNEXT is normally present. For unusual servers, one UID SEARCH is still
    // much cheaper than downloading and parsing message bodies every two minutes.
    let server_highest = match mailbox.uid_next {
        Some(next) => next.saturating_sub(1),
        None => session.uid_search("ALL").await.context("IMAP UID SEARCH failed")?.into_iter().max().unwrap_or(0),
    };
    if mailbox.uid_next.is_none() {
        plan = if reset {
            InboxFetchPlan::RecentWindow
        } else {
            match saved_highest {
                Some(highest) if server_highest <= highest => InboxFetchPlan::None,
                Some(highest) => InboxFetchPlan::NewerThan(highest),
                None => InboxFetchPlan::RecentWindow,
            }
        };
    }

    let fetch_set = match plan {
        InboxFetchPlan::None => None,
        InboxFetchPlan::NewerThan(highest) => Some(format!("{}:*", highest.saturating_add(1))),
        InboxFetchPlan::RecentWindow if mailbox.exists == 0 => None,
        InboxFetchPlan::RecentWindow => {
            let start = if mailbox.exists > cfg.fetch_count { mailbox.exists - cfg.fetch_count + 1 } else { 1 };
            Some(format!("{start}:{}", mailbox.exists))
        }
    };

    let mut emails = Vec::new();
    let mut incomplete_fetch = false;
    if let Some(set) = fetch_set {
        info!("Incremental Inbox fetch: {}", set);
        // RecentWindow is a sequence-number range; NewerThan is a UID range.
        match plan {
            InboxFetchPlan::NewerThan(_) => {
                let stream = session
                    .uid_fetch(&set, "(UID FLAGS BODY.PEEK[])")
                    .await
                    .context("IMAP incremental UID FETCH failed")?;
                futures::pin_mut!(stream);
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(fetch) => {
                            if let Some(email) = parse_fetch(&fetch) {
                                if !email.deleted {
                                    emails.push(email);
                                }
                            } else {
                                incomplete_fetch = true;
                            }
                        }
                        Err(error) => {
                            incomplete_fetch = true;
                            warn!("Malformed incremental fetch result: {}", error);
                        }
                    }
                }
            }
            InboxFetchPlan::RecentWindow => {
                let stream =
                    session.fetch(&set, "(UID FLAGS BODY.PEEK[])").await.context("IMAP recent-window FETCH failed")?;
                futures::pin_mut!(stream);
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(fetch) => {
                            if let Some(email) = parse_fetch(&fetch) {
                                if !email.deleted {
                                    emails.push(email);
                                }
                            } else {
                                incomplete_fetch = true;
                            }
                        }
                        Err(error) => {
                            incomplete_fetch = true;
                            warn!("Malformed recent-window fetch result: {}", error);
                        }
                    }
                }
            }
            InboxFetchPlan::None => unreachable!(),
        }
    }
    if incomplete_fetch {
        anyhow::bail!("IMAP FETCH returned incomplete message data; sync cursor was not advanced");
    }
    emails.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.uid.cmp(&a.uid)));
    session.logout().await.ok();
    Ok(InboxDelta { emails, uid_validity: mailbox.uid_validity, highest_uid: server_highest, reset })
}

fn parse_fetch(fetch: &Fetch) -> Option<RawEmail> {
    let raw = fetch.body()?;
    parse_fetch_bytes(fetch, raw)
}

fn parse_header_and_text(fetch: &Fetch) -> Option<RawEmail> {
    let header = fetch.header()?;
    let text = fetch.text().unwrap_or_default();
    let mut raw = Vec::with_capacity(header.len() + text.len() + 4);
    raw.extend_from_slice(header);
    if header.ends_with(b"\r\n") && !header.ends_with(b"\r\n\r\n") {
        raw.extend_from_slice(b"\r\n");
    } else if !header.ends_with(b"\r\n\r\n") {
        raw.extend_from_slice(b"\r\n\r\n");
    }
    raw.extend_from_slice(text);
    parse_fetch_bytes(fetch, &raw)
}

fn parse_fetch_bytes(fetch: &Fetch, raw: &[u8]) -> Option<RawEmail> {
    let parsed = parse_mail(raw).ok()?;

    let uid = fetch.uid.unwrap_or(fetch.message);
    let seen = fetch.flags().any(|f| matches!(f, async_imap::types::Flag::Seen));
    let deleted = fetch.flags().any(|f| matches!(f, async_imap::types::Flag::Deleted));

    let subject = parsed.headers.get_first_value("Subject").unwrap_or_default().trim().to_string();

    let header = |name: &str| parsed.headers.get_first_value(name).unwrap_or_default().trim().to_string();
    let from = header("From");
    let to = header("To");
    let cc = header("Cc");
    let reply_to = header("Reply-To");
    let bcc = header("Bcc");
    let message_id = header("Message-ID");
    let references = format!("{} {}", header("In-Reply-To"), header("References")).trim().to_string();
    // Receiving servers prepend their Authentication-Results. Use only the first one rather
    // than accepting a later header that the sender may have forged.
    let authentication_results = {
        let direct = header("Authentication-Results");
        if direct.is_empty() {
            header("ARC-Authentication-Results")
        } else {
            direct
        }
    };
    let security = crate::models::MailSecurity {
        authentication_results,
        spam_flag: header("X-Spam-Flag"),
        spam_status: header("X-Spam-Status"),
        spam_score: header("X-Spam-Score"),
        microsoft_scl: header("X-MS-Exchange-Organization-SCL"),
        list_id: header("List-ID"),
        list_unsubscribe: header("List-Unsubscribe"),
        precedence: header("Precedence"),
    };

    let date_str = parsed.headers.get_first_value("Date").unwrap_or_default();
    let date: Option<DateTime<Utc>> =
        mailparse::dateparse(&date_str).ok().and_then(|ts| DateTime::from_timestamp(ts, 0));

    let (body_text, body_html, attachments) = extract_message_parts(&parsed);

    debug!("Parsed email uid={} subject={:?}", uid, subject);

    Some(RawEmail {
        uid: uid.into(), // the caller adds the account (`mail_id`)
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
        attachments,
        security,
    })
}

/// Inline images bigger than this are left out (they would bloat the cache).
const MAX_INLINE_IMAGE: usize = 3 * 1024 * 1024;

#[cfg(test)]
pub(crate) fn extract_bodies(mail: &mailparse::ParsedMail) -> (String, Option<String>) {
    let (plain, html, _) = extract_message_parts(mail);
    (plain, html)
}

/// Extract display bodies and lightweight attachment metadata. Attachment bytes are not kept in
/// the cache; the original message is fetched and the chosen MIME part decoded when it is opened.
pub(crate) fn extract_message_parts(
    mail: &mailparse::ParsedMail,
) -> (String, Option<String>, Vec<crate::models::AttachmentMeta>) {
    let mut plain = String::new();
    let mut html: Option<String> = None;
    let mut inline = Vec::new();
    let mut candidates = Vec::new();
    collect_parts(mail, "", &mut plain, &mut html, &mut inline, &mut candidates);
    if plain.is_empty() {
        if let Ok(body) = decode_text_body(mail) {
            plain = body;
        }
    }
    let html_lower = html.as_deref().unwrap_or_default().to_lowercase();
    let attachments = candidates
        .into_iter()
        .filter(|(_, cid, explicit)| {
            *explicit || cid.is_empty() || !html_lower.contains(&format!("cid:{}", cid.to_lowercase()))
        })
        .map(|(meta, _, _)| meta)
        .collect();
    (plain, html.map(|h| embed_inline_images(h, inline)), attachments)
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
    if let Some(encoding) =
        mail.ctype.params.get("charset").and_then(|label| encoding_rs::Encoding::for_label(label.trim().as_bytes()))
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
    part_id: &str,
    plain: &mut String,
    html: &mut Option<String>,
    inline: &mut Vec<(String, String)>,
    attachments: &mut Vec<(crate::models::AttachmentMeta, String, bool)>,
) {
    let ctype = mail.ctype.mimetype.to_lowercase();
    let disposition = mail.get_content_disposition();
    let filename = disposition
        .params
        .get("filename")
        .or_else(|| mail.ctype.params.get("name"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let explicit_attachment = disposition.disposition == DispositionType::Attachment;
    let is_attachment = explicit_attachment || !filename.is_empty();

    if mail.subparts.is_empty() && is_attachment {
        let data = mail.get_body_raw().unwrap_or_default();
        let name = if filename.is_empty() { format!("attachment-{}", attachments.len() + 1) } else { filename };
        let cid = mail.headers.get_first_value("Content-ID").unwrap_or_default();
        let cid = cid.trim().trim_start_matches('<').trim_end_matches('>').trim().to_string();
        attachments.push((
            crate::models::AttachmentMeta {
                part_id: if part_id.is_empty() { "1".into() } else { part_id.into() },
                filename: name,
                mime_type: ctype.clone(),
                size: data.len() as u64,
            },
            cid.clone(),
            explicit_attachment,
        ));
        if !cid.is_empty() && !data.is_empty() && data.len() <= MAX_INLINE_IMAGE {
            use base64::Engine;
            let mime = if ctype.starts_with("image/") { ctype.as_str() } else { "application/octet-stream" };
            let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
            inline.push((cid, format!("data:{mime};base64,{encoded}")));
        }
    } else if ctype == "text/plain" {
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
        for (index, sub) in mail.subparts.iter().enumerate() {
            let child_id =
                if part_id.is_empty() { (index + 1).to_string() } else { format!("{part_id}.{}", index + 1) };
            collect_parts(sub, &child_id, plain, html, inline, attachments);
        }
    }
}

/// Decode one previously advertised attachment from a complete RFC 822 message.
pub(crate) fn attachment_from_message(raw: &[u8], part_id: &str) -> Result<(crate::models::AttachmentMeta, Vec<u8>)> {
    let parsed = parse_mail(raw).context("Could not parse the message")?;
    let (_, _, attachments) = extract_message_parts(&parsed);
    let meta = attachments.into_iter().find(|item| item.part_id == part_id).context("Attachment not found")?;
    let part = find_part(&parsed, "", part_id).context("Attachment part not found")?;
    let bytes = part.get_body_raw().context("Could not decode the attachment")?;
    Ok((meta, bytes))
}

fn find_part<'a>(
    mail: &'a mailparse::ParsedMail<'a>,
    current: &str,
    wanted: &str,
) -> Option<&'a mailparse::ParsedMail<'a>> {
    let own_id = if current.is_empty() && mail.subparts.is_empty() { "1" } else { current };
    if own_id == wanted {
        return Some(mail);
    }
    for (index, sub) in mail.subparts.iter().enumerate() {
        let child = if current.is_empty() { (index + 1).to_string() } else { format!("{current}.{}", index + 1) };
        if let Some(found) = find_part(sub, &child, wanted) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod incremental_tests {
    use super::{inbox_fetch_plan, InboxFetchPlan};

    #[test]
    fn unchanged_uidnext_needs_no_fetch() {
        assert_eq!(inbox_fetch_plan(Some(9), Some(42), Some(9), Some(43)), (InboxFetchPlan::None, false));
    }

    #[test]
    fn newer_uidnext_fetches_only_arrivals() {
        assert_eq!(inbox_fetch_plan(Some(9), Some(42), Some(9), Some(46)), (InboxFetchPlan::NewerThan(42), false));
    }

    #[test]
    fn changed_uidvalidity_forces_recent_reconciliation() {
        assert_eq!(inbox_fetch_plan(Some(9), Some(42), Some(10), Some(8)), (InboxFetchPlan::RecentWindow, true));
    }

    #[test]
    fn impossible_cursor_forces_recent_reconciliation() {
        assert_eq!(inbox_fetch_plan(Some(9), Some(99), Some(9), Some(50)), (InboxFetchPlan::RecentWindow, true));
    }

    #[test]
    fn missing_cursor_bootstraps_with_recent_window() {
        assert_eq!(inbox_fetch_plan(None, None, Some(9), Some(50)), (InboxFetchPlan::RecentWindow, false));
    }
}
