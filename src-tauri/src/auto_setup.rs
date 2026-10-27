//! Auto-setup: finding an account's server settings from its email address alone.
//! Only the domain leaves the computer — the address itself is never sent anywhere.

use anyhow::{bail, Context, Result};
use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::dto::{MailSetupDto, ServerDto};

/// One server as published in an autoconfig answer.
#[derive(Default)]
struct Entry {
    /// "imap", "pop3" or "smtp".
    kind: String,
    host: String,
    port: String,
    /// "SSL" (implicit TLS), "STARTTLS" or "plain".
    socket: String,
    user: String,
}

/// The domain of an email address ("" when it is not one).
pub fn domain_of(email: &str) -> String {
    match email.rsplit_once('@') {
        Some((_, d)) => d.trim().trim_end_matches('.').to_lowercase(),
        None => String::new(),
    }
}

/// What ISPDB knows a host under: its last two labels (`aspmx.l.google.com` → `google.com`).
/// shortcut: multi-part TLDs are not known here, so a provider on mail.foo.co.uk gives
/// `co.uk` and that lookup simply finds nothing.
pub fn base_domain(host: &str) -> String {
    let host = host.trim().trim_end_matches('.');
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    if labels.len() < 2 {
        return host.to_lowercase();
    }
    labels[labels.len() - 2..].join(".").to_lowercase()
}

/// The MX with the lowest preference in a Cloudflare DNS-over-HTTPS answer, without its
/// trailing dot. None when the domain has no MX or the answer is not what it should be.
pub fn mx_host(json: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Answer {
        #[serde(default)]
        r#type: u16,
        #[serde(default)]
        data: String,
    }
    #[derive(serde::Deserialize)]
    struct DoH {
        #[serde(rename = "Answer", default)]
        answers: Vec<Answer>,
    }
    let answer: DoH = serde_json::from_str(json).ok()?;
    let mut best: Option<(u32, String)> = None;
    for a in answer.answers.iter().filter(|a| a.r#type == 15) {
        // An MX answer reads "<preference> <exchange>.".
        let Some((pref, host)) = a.data.split_once(char::is_whitespace) else { continue };
        let host = host.trim().trim_end_matches('.').to_lowercase();
        if host.is_empty() {
            continue;
        }
        let pref: u32 = pref.trim().parse().unwrap_or(u32::MAX);
        if best.as_ref().map_or(true, |(p, _)| pref < *p) {
            best = Some((pref, host));
        }
    }
    best.map(|(_, host)| host)
}

/// The provider's name and the first servers this app can use in an autoconfig answer.
/// `imap_issue` / `smtp_issue` say why a side has nothing ("starttls", "plain", "port").
/// Anything that is not an autoconfig file, or holds no server, is an error: the lookup
/// then tries its next source.
pub fn parse_autoconfig(xml: &str, email: &str) -> Result<MailSetupDto> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut entries: Vec<Entry> = Vec::new();
    let mut provider = String::new();
    let mut current: Option<Entry> = None;
    let mut text = String::new();
    let mut root_seen = false;
    loop {
        let event = reader.read_event_into(&mut buf).context("Not an autoconfig answer")?;
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.local_name().as_ref().to_string();
                if !root_seen {
                    if name != "clientConfig" {
                        bail!("Not an autoconfig answer");
                    }
                    root_seen = true;
                }
                text.clear();
                if name == "incomingServer" || name == "outgoingServer" {
                    let kind = if name == "incomingServer" {
                        if attribute(&e, "type") == "pop3" { "pop3" } else { "imap" }
                    } else {
                        "smtp"
                    };
                    current = Some(Entry { kind: kind.to_string(), ..Default::default() });
                }
            }
            // The reader splits the text of an element at every entity reference.
            Event::Text(e) => text.push_str(&e.xml10_content()),
            Event::GeneralRef(e) => text.push_str(entity(e.into_inner().as_ref())),
            Event::End(e) => {
                let name = e.local_name().as_ref().to_string();
                let value = text.trim().to_string();
                match name.as_str() {
                    "displayName" => provider = value,
                    "hostname" | "port" | "socketType" | "username" => {
                        if let Some(c) = current.as_mut() {
                            match name.as_str() {
                                "hostname" => c.host = value,
                                "port" => c.port = value,
                                "socketType" => c.socket = value,
                                _ => c.user = value,
                            }
                        }
                    }
                    "incomingServer" | "outgoingServer" => {
                        if let Some(c) = current.take() {
                            entries.push(c);
                        }
                    }
                    _ => {}
                }
                text.clear();
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    if entries.is_empty() {
        bail!("This autoconfig answer has no server");
    }
    let mut out = MailSetupDto { provider, ..Default::default() };
    for e in entries.iter().filter(|e| e.kind == "imap") {
        match usable_port("imap", &e.socket, &e.port) {
            Ok(port) => { out.imap = Some(server(e, port, email)); break; }
            Err(why) => if out.imap_issue.is_empty() { out.imap_issue = why.to_string() },
        }
    }
    for e in entries.iter().filter(|e| e.kind == "smtp") {
        match usable_port("smtp", &e.socket, &e.port) {
            Ok(port) => { out.smtp = Some(server(e, port, email)); break; }
            Err(why) => if out.smtp_issue.is_empty() { out.smtp_issue = why.to_string() },
        }
    }
    Ok(out)
}

/// An entry this app can use, as the account block needs it.
fn server(e: &Entry, port: String, email: &str) -> ServerDto {
    ServerDto { host: e.host.clone(), port, user: username(&e.user, email) }
}

/// The port this app can use for an entry, or why it cannot: incoming mail only speaks
/// implicit TLS (`imap_client::connect`), outgoing one of 465 SSL or 587 STARTTLS (the
/// SMTP sender picks implicit TLS by port 465 alone). Auto-scan uses it for the servers a
/// profile holds too.
pub(crate) fn usable_port(kind: &str, socket: &str, port: &str) -> Result<String, &'static str> {
    let ssl = socket.eq_ignore_ascii_case("SSL");
    let starttls = socket.eq_ignore_ascii_case("STARTTLS");
    let port = port.trim();
    if kind == "imap" {
        if ssl {
            return Ok(if port.is_empty() { "993".into() } else { port.into() });
        }
        return Err(if starttls { "starttls" } else { "plain" });
    }
    if ssl {
        if port.is_empty() || port == "465" {
            return Ok("465".into());
        }
        return Err("port");
    }
    if starttls {
        if port.is_empty() || port == "587" {
            return Ok("587".into());
        }
        return Err("port");
    }
    Err("plain")
}

/// The provider's username with its placeholders replaced; a missing one means the
/// full address, which is what nearly every provider expects.
fn username(user: &str, email: &str) -> String {
    let filled = user
        .replace("%EMAILADDRESS%", email)
        .replace("%EMAILLOCALPART%", email.split('@').next().unwrap_or(""))
        .replace("%EMAILDOMAIN%", &domain_of(email))
        .trim()
        .to_string();
    if filled.is_empty() { email.to_string() } else { filled }
}

/// The five entities XML defines by default; any other reference brings nothing.
fn entity(name: &str) -> &'static str {
    match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => "",
    }
}

/// One attribute of a start tag, lowercased ("" when absent).
fn attribute(e: &quick_xml::events::BytesStart, key: &str) -> String {
    e.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == key)
        .map(|a| a.value.trim().to_lowercase())
        .unwrap_or_default()
}

// ── The lookup: first source that answers wins ───────────────────────────────

const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The server settings for an address, None when no source knows its domain.
pub async fn lookup(email: &str) -> Result<Option<MailSetupDto>> {
    let domain = domain_of(email);
    if domain.is_empty() {
        return Ok(None);
    }
    let client = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build()?;
    let sources = [
        (format!("https://autoconfig.{domain}/mail/config-v1.1.xml"), format!("autoconfig.{domain}")),
        (format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"), domain.clone()),
        (format!("https://autoconfig.thunderbird.net/v1.1/{domain}"), "autoconfig.thunderbird.net".to_string()),
    ];
    for (url, host) in sources {
        if let Some(found) = from_url(&client, &url, &host, email).await {
            return Ok(Some(found));
        }
    }
    // Nothing for the domain itself: ask DNS who receives its mail and try ISPDB for them.
    if let Some(mx) = mx_of(&client, &domain).await {
        let base = base_domain(&mx);
        if base != domain {
            let url = format!("https://autoconfig.thunderbird.net/v1.1/{base}");
            if let Some(mut found) = from_url(&client, &url, "autoconfig.thunderbird.net", email).await {
                found.source = format!("autoconfig.thunderbird.net ({base})");
                return Ok(Some(found));
            }
        }
    }
    Ok(None)
}

/// One autoconfig source; None on any HTTP error, non-200 or unparseable answer.
async fn from_url(client: &reqwest::Client, url: &str, host: &str, email: &str) -> Option<MailSetupDto> {
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let mut found = parse_autoconfig(&resp.text().await.ok()?, email).ok()?;
    found.source = host.to_string();
    Some(found)
}

/// The domain's MX host through Cloudflare's DNS-over-HTTPS; its `accept` header is what
/// makes the endpoint answer JSON instead of DNS wire format.
async fn mx_of(client: &reqwest::Client, domain: &str) -> Option<String> {
    let url = format!("https://cloudflare-dns.com/dns-query?name={domain}&type=MX");
    let resp = client.get(url).header("accept", "application/dns-json").send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    mx_host(&resp.text().await.ok()?)
}
