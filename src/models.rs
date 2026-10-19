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
    pub date_str: String,
    pub summary: String,
    pub priority: Priority,
    pub is_read: bool,
    /// Full plain-text body kept for the reading pane.
    pub body_text: String,
    /// Project this email was assigned to, if any.
    pub project: Option<String>,
    /// Deadline extracted from email content.
    pub deadline: Option<NaiveDate>,
    /// Action item extracted from email content.
    pub task: Option<String>,
    /// Video meeting URL extracted by AI (Zoom, Teams, Meet, etc.)
    pub meeting_link: Option<String>,
}
