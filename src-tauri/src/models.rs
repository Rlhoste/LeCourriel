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

/// Project urgency level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectUrgency {
    High,
    Normal,
}

impl ProjectUrgency {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "high" => ProjectUrgency::High,
            _ => ProjectUrgency::Normal,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectUrgency::High => "high",
            ProjectUrgency::Normal => "normal",
        }
    }
}

/// A user-defined project for grouping emails.
#[derive(Debug, Clone)]
pub struct Project {
    pub name: String,
    pub description: String,
    pub urgency: ProjectUrgency,
}

/// Raw email as fetched from IMAP before AI processing.
#[derive(Debug, Clone)]
pub struct RawEmail {
    pub uid: u32,
    pub subject: String,
    pub from: String,
    pub to: String,
    pub cc: String,
    pub reply_to: String,
    pub bcc: String,
    pub message_id: String,
    /// \Seen flag on the server.
    pub seen: bool,
    pub date: Option<DateTime<Utc>>,
    pub body_text: String,
    pub body_html: Option<String>,
}

/// Processed email card ready for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailCard {
    pub uid: u32,
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
    pub date_str: String,
    /// Full local date and time, e.g. "Monday 19 October 2026, 15:29".
    #[serde(default)]
    pub date_full: String,
    pub summary: String,
    pub priority: Priority,
    pub is_read: bool,
    /// Full plain-text body kept for the reading pane.
    pub body_text: String,
    /// Raw HTML body for WebView rendering.
    pub body_html: String,
    /// Project this email was assigned to, if any.
    pub project: Option<String>,
    /// Deadline extracted from email content.
    pub deadline: Option<NaiveDate>,
    /// Action item extracted from email content.
    pub task: Option<String>,
    /// Video meeting URL extracted by AI (Zoom, Teams, Meet, etc.)
    pub meeting_link: Option<String>,
}
