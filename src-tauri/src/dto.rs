use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct EmailCardDto {
    pub uid: u64,
    pub subject: String,
    pub from_name: String,
    pub from_email: String,
    pub to: String,
    pub cc: String,
    pub reply_to: String,
    pub bcc: String,
    pub message_id: String,
    pub references: String,
    pub date_str: String,
    pub date_full: String,
    /// Unix time of the email (0 when unknown); the UI formats it in its language.
    pub date_ts: i64,
    pub summary: String,
    pub priority: String,
    pub is_read: bool,
    pub project: String,
    pub deadline_str: String,
    pub task: String,
    pub body_text: String,
    pub body_html: String,
    /// Files attached to the email: `{ name, size, mime }`, size in bytes.
    #[serde(default)]
    pub attachments: Vec<crate::models::Attachment>,
    /// HTML body but no plain-text part: discussion mode shows it as a page.
    #[serde(default)]
    pub html_only: bool,
    pub meeting_link: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AccountDto {
    pub label: String,
    pub imap_host: String,
    pub imap_port: String,
    pub imap_user: String,
    pub imap_pass: String,
    pub imap_mailbox: String,
    pub imap_fetch: String,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_user: String,
    pub smtp_pass: String,
    pub smtp_from: String,
    /// The account's signature file in the data folder ("" = none) and when to add it.
    #[serde(default)]
    pub signature_file: String,
    #[serde(default)]
    pub signature_on_new: bool,
    #[serde(default)]
    pub signature_on_reply: bool,
}

/// What the composer needs to know about each account, in account order.
#[derive(Serialize, Clone, Debug)]
pub struct ComposeAccountDto {
    /// The sender as recipients see it ("Name <address>").
    pub from: String,
    pub signature_html: String,
    pub signature_on_new: bool,
    pub signature_on_reply: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ProjectDto {
    pub name: String,
    pub urgency: String,
    pub description: String,
    pub email_count: i32,
    /// Set when a new project was created but its emails could not be filed (AI error).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DeadlineDto {
    pub uid: u64,
    pub subject: String,
    pub deadline_str: String,
    /// YYYY-MM-DD, used to place the deadline in the calendar.
    pub date_iso: String,
    pub project: String,
    pub days_until: i32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct WeekDayDto {
    pub day_name: String,
    /// YYYY-MM-DD of this day.
    pub date_iso: String,
    pub day_number: u32,
    pub is_today: bool,
    pub dl1_subject: String,
    pub dl1_uid: u64,
    pub dl2_subject: String,
    pub dl2_uid: u64,
    pub dl3_subject: String,
    pub dl3_uid: u64,
}

/// What the briefing line says, as data: "live" (refreshed at hour:minute, `count` need
/// attention), "cached", "classifying" (`count` new emails), "up_to_date",
/// "ai_missing" (the AI is not set up: `count` emails wait to be sorted),
/// "ai_error" (the AI failed on `count` emails; `detail` says why), or, with a
/// Claude subscription, "ai_not_paid" (no paid plan) and "ai_stopped" (`detail` says why).
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Briefing {
    pub kind: &'static str,
    pub hour: u32,
    pub minute: u32,
    pub count: usize,
    pub detail: String,
}

#[derive(Serialize, Clone)]
pub struct AppStateDto {
    pub greeting: String,
    pub briefing_time: String,
    /// Raw values so the UI can write the greeting and briefing in any language.
    pub user_name: String,
    pub hour: u32,
    pub briefing: Briefing,
    pub loading: bool,
    pub urgent_emails: Vec<EmailCardDto>,
    pub high_emails: Vec<EmailCardDto>,
    pub other_emails: Vec<EmailCardDto>,
    pub info_emails: Vec<EmailCardDto>,
    pub threat_emails: Vec<EmailCardDto>,
    pub task_emails: Vec<EmailCardDto>,
    pub all_emails: Vec<EmailCardDto>,
    pub tasks: Vec<crate::tasks::Task>,
    pub meeting_items: Vec<EmailCardDto>,
    pub important_emails: Vec<EmailCardDto>,
    pub all_emails_by_priority: Vec<EmailCardDto>,
    pub sent_emails: Vec<EmailCardDto>,
    pub drafts_emails: Vec<EmailCardDto>,
    pub trash_emails: Vec<EmailCardDto>,
    pub spam_emails: Vec<EmailCardDto>,
    pub deadlines: Vec<DeadlineDto>,
    pub week_days: Vec<WeekDayDto>,
    pub projects: Vec<ProjectDto>,
    pub calendar_year: i32,
    pub calendar_month: u32,
}

/// The AI provider settings as shown and edited in Settings → AI.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AiSettingsDto {
    pub provider: String,
    pub model: String,
    /// Custom server URL ("" = the provider's default).
    pub base_url: String,
    /// One API key per provider.
    pub keys: std::collections::BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SettingsDto {
    pub display_name: String,
    pub font_scale: f32,
    pub theme: String,
    pub language: String,
    pub notify_new_mail: bool,
    pub discussion_mode: bool,
    pub ai: AiSettingsDto,
    pub giphy_key: String,
    pub supabase_url: String,
    pub supabase_anon_key: String,
    pub accounts: Vec<AccountDto>,
}

/// One mail server Auto-setup found for an account.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct ServerDto {
    pub host: String,
    pub port: String,
    pub user: String,
}

/// What Auto-setup found for an email address: the provider's own name, the host the
/// settings were looked up on, and the servers this app can use. `imap_issue` and
/// `smtp_issue` say why a side is empty ("starttls", "plain", "port").
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct MailSetupDto {
    pub provider: String,
    /// Shown as "Looked up on …" under the result.
    pub source: String,
    pub imap: Option<ServerDto>,
    pub smtp: Option<ServerDto>,
    pub imap_issue: String,
    pub smtp_issue: String,
}

/// One account Auto-scan found in another mail app, with the server settings this app can
/// use. `imap_issue` and `smtp_issue` say why a side is empty, `pop3` that the account
/// cannot be added at all, and `oauth` that its login is a token rather than a password.
/// A password is never part of it.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct FoundAccountDto {
    /// Which app holds it: "Thunderbird", "Betterbird", "Outlook" or "Evolution".
    pub client: String,
    /// The name the account has there, or its address.
    pub label: String,
    /// The sender's name.
    pub name: String,
    pub email: String,
    pub pop3: bool,
    pub imap: Option<ServerDto>,
    pub smtp: Option<ServerDto>,
    pub imap_issue: String,
    pub smtp_issue: String,
    pub oauth: bool,
}

/// Why a connection test failed: `kind` picks the sentence the UI shows (auth, host,
/// refused, reset, tls, folder, timeout, other) and `detail` is the server's own message.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ConnError {
    pub kind: &'static str,
    pub detail: String,
}

impl ConnError {
    pub fn timeout() -> Self {
        ConnError { kind: "timeout", detail: String::new() }
    }

    pub fn from_error(e: &anyhow::Error) -> Self {
        // Libraries repeat their causes ("x: x: x"): keep each part once.
        let mut parts: Vec<String> = Vec::new();
        for cause in e.chain() {
            for part in cause.to_string().split(": ") {
                let part = part.trim();
                if !part.is_empty() && !parts.iter().any(|p| p == part) {
                    parts.push(part.to_string());
                }
            }
        }
        let detail = parts.join(": ");
        let lower = detail.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
        let kind = if has(&[
            "authenticationfailed",
            "authentication failed",
            "(535)",
            "login failed",
            "invalid credentials",
        ]) {
            "auth"
        } else if has(&["lookup address", "name or service not known", "no such host", "nodename nor servname"]) {
            "host"
        } else if has(&["mailbox doesn't exist", "folder \""]) {
            "folder"
        } else if has(&["connection refused"]) {
            "refused"
        } else if has(&["reset by peer", "connection reset", "broken pipe", "unexpected eof"]) {
            // The server answered, then hung up: usually a block after failed logins.
            "reset"
        } else if has(&["tls", "ssl", "handshake", "certificate"]) {
            "tls"
        } else if has(&["timed out"]) {
            "timeout"
        } else {
            "other"
        };
        ConnError { kind, detail }
    }
}
