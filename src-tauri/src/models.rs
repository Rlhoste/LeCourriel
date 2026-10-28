use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// Priority categories assigned by the AI classifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Priority {
    Urgent,
    HighPriority,
    OtherPriority,
    Information,
    Threat,
    Task,
}

impl Priority {
    /// Parse a string returned by the Claude API into a Priority variant.
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "urgent" => Priority::Urgent,
            "high_priority" | "high" | "highpriority" => Priority::HighPriority,
            "threat" => Priority::Threat,
            "task" => Priority::Task,
            "information" | "info" => Priority::Information,
            _ => Priority::OtherPriority,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Priority::Urgent => "urgent",
            Priority::HighPriority => "high",
            Priority::OtherPriority => "other",
            Priority::Information => "info",
            Priority::Threat => "threat",
            Priority::Task => "task",
        }
    }
}

/// An email's id in the app: its account's position above bit 32, its IMAP UID below, so
/// emails of two accounts never share an id. The first account's ids are its UIDs (older
/// caches, tasks and dismissals stay valid). Stays below 2^53 for the UI's JavaScript numbers.
pub fn mail_id(account: usize, uid: u32) -> u64 {
    ((account as u64) << 32) | uid as u64
}

/// The account position and the IMAP UID inside a `mail_id`.
pub fn split_mail_id(id: u64) -> (usize, u32) {
    ((id >> 32) as usize, id as u32)
}

/// The header lines of a raw message, as written (up to the first blank line), with `\n` breaks.
pub fn header_block(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw).replace("\r\n", "\n");
    text.split("\n\n").next().unwrap_or_default().trim_end().to_string()
}

/// Raw email as fetched from IMAP before AI processing.
#[derive(Debug, Clone)]
pub struct RawEmail {
    pub uid: u64,
    pub subject: String,
    pub from: String,
    pub to: String,
    pub cc: String,
    pub reply_to: String,
    pub bcc: String,
    pub message_id: String,
    /// Message-IDs from In-Reply-To and References, space-separated: the discussion it answers.
    pub references: String,
    /// \Seen flag on the server.
    pub seen: bool,
    /// \Deleted flag: moved away and waiting for a purge, so not shown.
    pub deleted: bool,
    pub date: Option<DateTime<Utc>>,
    pub body_text: String,
    pub body_html: Option<String>,
    pub attachments: Vec<Attachment>,
    /// HTML body but no plain-text part: laid out as a page.
    pub html_only: bool,
}

/// A file attached to an email (images shown in the body are not counted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    /// Decoded size in bytes.
    pub size: u64,
    /// MIME type, lowercase ("application/pdf").
    pub mime: String,
}

/// Processed email card ready for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailCard {
    pub uid: u64,
    pub subject: String,
    pub from_name: String,
    pub from_email: String,
    /// Raw To / Cc / Reply-To header values (empty when absent).
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub cc: String,
    #[serde(default)]
    pub reply_to: String,
    #[serde(default)]
    pub bcc: String,
    /// Message-ID header, for reply threading.
    #[serde(default)]
    pub message_id: String,
    /// Message-IDs this email replies to (In-Reply-To + References), space-separated.
    #[serde(default)]
    pub references: String,
    pub date_str: String,
    /// Full local date and time, e.g. "Monday 19 October 2026, 15:29".
    #[serde(default)]
    pub date_full: String,
    /// Unix timestamp of the Date header (0 when unknown); used by search.
    #[serde(default)]
    pub date_ts: i64,
    pub summary: String,
    pub priority: Priority,
    pub is_read: bool,
    /// Full plain-text body kept for the reading pane.
    pub body_text: String,
    /// Raw HTML body for WebView rendering.
    pub body_html: String,
    /// Older caches have no list: none.
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// HTML body but no plain-text part (older caches: false until the next refresh).
    #[serde(default)]
    pub html_only: bool,
    /// Project this email was assigned to, if any.
    pub project: Option<String>,
    /// Deadline extracted from email content.
    pub deadline: Option<NaiveDate>,
    /// Action item extracted from email content.
    pub task: Option<String>,
    /// Video meeting URL extracted by AI (Zoom, Teams, Meet, etc.)
    pub meeting_link: Option<String>,
}
