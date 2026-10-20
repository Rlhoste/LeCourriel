use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct EmailCardDto {
    pub uid: u32,
    pub subject: String,
    pub from_name: String,
    pub from_email: String,
    pub to: String,
    pub cc: String,
    pub reply_to: String,
    pub bcc: String,
    pub message_id: String,
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
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ProjectDto {
    pub name: String,
    pub urgency: String,
    pub description: String,
    pub email_count: i32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DeadlineDto {
    pub uid: u32,
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
    pub dl1_uid: u32,
    pub dl2_subject: String,
    pub dl2_uid: u32,
    pub dl3_subject: String,
    pub dl3_uid: u32,
}

/// What the briefing line says, as data: "live" (refreshed at hour:minute, `count` need
/// attention), "cached", "classifying" (`count` new emails), or "up_to_date".
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Briefing {
    pub kind: &'static str,
    pub hour: u32,
    pub minute: u32,
    pub count: usize,
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
    pub ai: AiSettingsDto,
    pub giphy_key: String,
    pub signature: crate::config::SignatureConfig,
    pub supabase_url: String,
    pub supabase_anon_key: String,
    pub accounts: Vec<AccountDto>,
}
