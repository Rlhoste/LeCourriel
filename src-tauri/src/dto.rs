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
    pub day_number: u32,
    pub is_today: bool,
    pub dl1_subject: String,
    pub dl1_uid: u32,
    pub dl2_subject: String,
    pub dl2_uid: u32,
    pub dl3_subject: String,
    pub dl3_uid: u32,
}

#[derive(Serialize, Clone)]
pub struct AppStateDto {
    pub greeting: String,
    pub briefing_time: String,
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

#[derive(Serialize, Deserialize, Clone)]
pub struct SettingsDto {
    pub display_name: String,
    pub font_scale: f32,
    pub claude_key: String,
    pub claude_model: String,
    pub giphy_key: String,
    pub accounts: Vec<AccountDto>,
}
