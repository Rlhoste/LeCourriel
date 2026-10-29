use anyhow::{Context, Result};
use lettre::{
    message::{header::ContentType, Attachment, Mailbox, Mailboxes, MultiPart, SinglePart},
    transport::smtp::authentication::Credentials,
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};
use tracing::info;

use crate::attachments::{self, Loaded};
use crate::config::SmtpConfig;

/// Test SMTP connectivity and auth without sending any email.
pub async fn check_connection(cfg: &SmtpConfig) -> Result<()> {
    let creds = Credentials::new(cfg.username.clone(), cfg.password.clone());
    if cfg.port == 465 {
        let transport: AsyncSmtpTransport<Tokio1Executor> = AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host)
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

/// An outgoing message as written in the composer.
pub struct OutgoingMail {
    /// Comma-separated address lists ("Name <a@b>, c@d").
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    /// Body HTML from the editor; inline images may be data: URIs.
    pub html: String,
    /// Message-ID of the email being replied to, if any.
    pub in_reply_to: Option<String>,
    /// Files attached to the message: paths on disk (#43, #44).
    pub attachments: Vec<String>,
}

pub struct InlineImage {
    pub cid: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

/// Replace every `src="data:image/...;base64,..."` with `src="cid:imgN"` and
/// return the decoded images. Other sources (http, cid) are left untouched.
pub fn extract_inline_images(html: &str) -> (String, Vec<InlineImage>) {
    use base64::Engine;
    let mut out = String::with_capacity(html.len());
    let mut images = Vec::new();
    let mut rest = html;
    while let Some(pos) = rest.find("src=") {
        let (before, after_src) = rest.split_at(pos + 4);
        out.push_str(before);
        let quote = after_src.chars().next();
        let (q, value_start) = match quote {
            Some(c @ ('"' | '\'')) => (c, &after_src[1..]),
            _ => {
                rest = after_src;
                continue;
            }
        };
        let Some(end) = value_start.find(q) else {
            rest = after_src;
            continue;
        };
        let value = &value_start[..end];
        let decoded = value
            .strip_prefix("data:")
            .and_then(|v| v.split_once(";base64,"))
            .filter(|(ct, _)| ct.starts_with("image/"))
            .and_then(|(ct, b64)| {
                let clean: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
                base64::engine::general_purpose::STANDARD.decode(clean).ok().map(|d| (ct.to_string(), d))
            });
        match decoded {
            Some((content_type, data)) => {
                let cid = format!("img{}", images.len() + 1);
                out.push(q);
                out.push_str("cid:");
                out.push_str(&cid);
                out.push(q);
                images.push(InlineImage { cid, content_type, data });
            }
            None => {
                out.push(q);
                out.push_str(value);
                out.push(q);
            }
        }
        rest = &value_start[end + 1..];
    }
    out.push_str(rest);
    (out, images)
}

fn parse_list(list: &str, field: &str) -> Result<Vec<Mailbox>> {
    if list.trim().is_empty() {
        return Ok(vec![]);
    }
    let boxes: Mailboxes = list
        .trim()
        .trim_end_matches(',')
        .parse()
        .with_context(|| format!("Invalid {} address list: {}", field, list))?;
    Ok(boxes.into_iter().collect())
}

/// The message body: the plain-text alternative of the HTML, its inline images, and the
/// attached files. With attachments the whole becomes `multipart/mixed`, which is what a
/// reader expects: the alternative/related part stays one piece of it.
pub fn build_body(html: &str, files: &[Loaded]) -> MultiPart {
    let (html_body, images) = extract_inline_images(html);
    let html_doc = format!("<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body>{}</body></html>", html_body);
    let text = html2text::from_read(html.as_bytes(), 80);

    let mut related = MultiPart::related().singlepart(SinglePart::html(html_doc));
    for img in images {
        related = related.singlepart(inline_part(&img.cid, &img.content_type, img.data));
    }
    let alternative = MultiPart::alternative()
        .singlepart(SinglePart::plain(text))
        .multipart(related);
    if files.is_empty() {
        return alternative;
    }
    let mut mixed = MultiPart::mixed().multipart(alternative);
    for file in files {
        mixed = mixed.singlepart(attachment_part(&file.filename, &file.content_type, file.data.clone()));
    }
    mixed
}

/// An inline part (a picture in the message) with a safe content type.
fn inline_part(cid: &str, content_type: &str, data: Vec<u8>) -> SinglePart {
    Attachment::new_inline(cid.to_string()).body(data, mime(content_type))
}

/// An attached file, named as the recipient should see it.
fn attachment_part(filename: &str, content_type: &str, data: Vec<u8>) -> SinglePart {
    Attachment::new(filename.to_string()).body(data, mime(content_type))
}

fn mime(content_type: &str) -> ContentType {
    ContentType::parse(content_type)
        .unwrap_or_else(|_| ContentType::parse("application/octet-stream").expect("static type"))
}

/// Send an HTML message (with a plain-text alternative, inline images and attachments).
/// Returns the raw RFC 822 bytes so a copy can be stored in the Sent folder.
pub async fn send_message(cfg: &SmtpConfig, mail: &OutgoingMail) -> Result<Vec<u8>> {
    let from_mailbox: Mailbox = cfg.from_address.parse().with_context(|| {
        format!(
            "Invalid sender address \"{}\": write it as Name <you@example.com> in Settings → Accounts",
            cfg.from_address
        )
    })?;

    let to = parse_list(&mail.to, "To")?;
    let cc = parse_list(&mail.cc, "Cc")?;
    let bcc = parse_list(&mail.bcc, "Bcc")?;
    if to.is_empty() && cc.is_empty() && bcc.is_empty() {
        anyhow::bail!("Please enter at least one recipient");
    }
    info!("Sending email to {} recipient(s)", to.len() + cc.len() + bcc.len());

    let mut builder = Message::builder().from(from_mailbox).subject(mail.subject.clone());
    for m in to {
        builder = builder.to(m);
    }
    for m in cc {
        builder = builder.cc(m);
    }
    for m in bcc {
        builder = builder.bcc(m);
    }
    if let Some(id) = mail.in_reply_to.as_ref().filter(|id| !id.trim().is_empty()) {
        builder = builder.in_reply_to(id.trim().to_string()).references(id.trim().to_string());
    }

    let files = attachments::load_all(&mail.attachments)?;
    let email = builder
        .multipart(build_body(&mail.html, &files))
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

    let raw = email.formatted();
    transport.send(email).await.context("SMTP send failed")?;

    info!("Email sent successfully");
    Ok(raw)
}
