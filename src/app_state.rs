use anyhow::Result;
use chrono::{Local, NaiveDate, Utc};
use html2text;
use futures::future::join_all;
use reqwest::Client;
use slint::{invoke_from_event_loop, ModelRc, SharedString, VecModel, Weak};
use std::sync::Arc;
use tracing::{error, info, warn};

const CACHE_PATH: &str = "cache.json";

use crate::ai_classifier::classify;
use crate::config::Config;
use crate::imap_client::{fetch_emails, fetch_folder};
use crate::config::ImapConfig;
use crate::models::{EmailCard, Priority};

use crate::{AppWindow, DeadlineData, EmailCardData, ProjectData, WeekDayData};

/// Orchestrates fetch + classify + UI update.
pub async fn refresh(cfg: Arc<Config>, ui: Weak<AppWindow>) {
    {
        let ui_ref = ui.clone();
        invoke_from_event_loop(move || {
            if let Some(handle) = ui_ref.upgrade() {
                handle.set_loading(true);
            }
        })
        .ok();
    }

    match do_refresh(&cfg, &ui).await {
        Ok(_) => info!("Refresh complete"),
        Err(e) => {
            error!("Refresh failed: {:#}", e);
            let msg = format!("Error: {}", e);
            let ui_ref = ui.clone();
            invoke_from_event_loop(move || {
                if let Some(handle) = ui_ref.upgrade() {
                    handle.set_loading(false);
                    handle.set_briefing_time(SharedString::from(msg));
                }
            })
            .ok();
        }
    }
}

/// Convert a raw email into an EmailCard without AI classification.
fn raw_to_card(raw: crate::models::RawEmail) -> EmailCard {
    let (from_name, from_email) = parse_from(&raw.from);
    let date_str = raw.date.map(format_date).unwrap_or_else(|| "Unknown".to_string());
    EmailCard {
        uid: raw.uid,
        subject: if raw.subject.is_empty() { "(No Subject)".to_string() } else { raw.subject },
        from_name,
        from_email,
        date_str,
        summary: String::new(),
        priority: Priority::Information,
        is_read: true,
        body_text: render_body(&raw.body_text, &raw.body_html),
        project: None,
        deadline: None,
        task: None,
        meeting_link: None,
    }
}

/// Try to guess the real IMAP folder names for Sent, Drafts, and Trash.
/// We connect, list all mailboxes, and pick the best match for each role.
async fn guess_folder_names(cfg: &ImapConfig) -> (String, String, String) {
    use native_tls::TlsConnector;

    let tls = match TlsConnector::builder()
        .min_protocol_version(Some(native_tls::Protocol::Tlsv10))
        .build()
    {
        Ok(t) => tokio_native_tls::TlsConnector::from(t),
        Err(_) => return ("Sent".to_string(), "Drafts".to_string(), "Trash".to_string()),
    };

    let tcp = match tokio::net::TcpStream::connect((cfg.host.as_str(), cfg.port)).await {
        Ok(s) => s,
        Err(_) => return ("Sent".to_string(), "Drafts".to_string(), "Trash".to_string()),
    };

    let tls_stream = match tls.connect(&cfg.host, tcp).await {
        Ok(s) => s,
        Err(_) => return ("Sent".to_string(), "Drafts".to_string(), "Trash".to_string()),
    };

    let client = async_imap::Client::new(tls_stream);
    let mut session = match client.login(&cfg.username, &cfg.password).await.map_err(|(e, _)| e) {
        Ok(s) => s,
        Err(_) => return ("Sent".to_string(), "Drafts".to_string(), "Trash".to_string()),
    };

    use futures::StreamExt;
    let names: Vec<String> = match session.list(Some(""), Some("*")).await {
        Ok(names_stream) => {
            futures::pin_mut!(names_stream);
            let mut out = Vec::new();
            while let Some(item) = names_stream.next().await {
                if let Ok(name) = item {
                    out.push(name.name().to_string());
                }
            }
            out
        }
        Err(_) => vec![],
    };
    session.logout().await.ok();

    let pick = |candidates: &[&str]| -> String {
        for candidate in candidates {
            let lc = candidate.to_lowercase();
            for n in &names {
                if n.to_lowercase() == lc {
                    return n.clone();
                }
            }
        }
        // Substring match fallback
        for candidate in candidates {
            let lc = candidate.to_lowercase();
            for n in &names {
                if n.to_lowercase().contains(&lc) {
                    return n.clone();
                }
            }
        }
        candidates[0].to_string()
    };

    let sent   = pick(&["Sent", "Sent Items", "Sent Messages", "INBOX.Sent"]);
    let drafts = pick(&["Drafts", "Draft", "INBOX.Drafts"]);
    let trash  = pick(&["Trash", "Deleted Items", "Deleted Messages", "INBOX.Trash"]);

    info!("Folder names: sent={:?} drafts={:?} trash={:?}", sent, drafts, trash);
    (sent, drafts, trash)
}

async fn do_refresh(cfg: &Config, ui: &Weak<AppWindow>) -> Result<()> {
    // Fetch inbox + folder emails concurrently across all accounts.
    let mut raw_emails = Vec::new();
    let mut sent_raw = Vec::new();
    let mut drafts_raw = Vec::new();
    let mut trash_raw = Vec::new();

    for account in &cfg.accounts {
        // Common IMAP folder names — try both standard and localized variants.
        let (sent_name, drafts_name, trash_name) = guess_folder_names(&account.imap).await;

        let (inbox_res, sent_res, drafts_res, trash_res) = tokio::join!(
            fetch_emails(&account.imap),
            fetch_folder(&account.imap, &sent_name),
            fetch_folder(&account.imap, &drafts_name),
            fetch_folder(&account.imap, &trash_name),
        );

        match inbox_res {
            Ok(mut v) => { info!("Inbox[{}]: {} emails", account.name, v.len()); raw_emails.append(&mut v); }
            Err(e) => warn!("Inbox fetch failed [{}]: {}", account.name, e),
        }
        match sent_res {
            Ok(mut v) => { info!("Sent[{}]: {} emails", account.name, v.len()); sent_raw.append(&mut v); }
            Err(e) => warn!("Sent fetch failed [{}] ({}): {}", account.name, sent_name, e),
        }
        match drafts_res {
            Ok(mut v) => { info!("Drafts[{}]: {} emails", account.name, v.len()); drafts_raw.append(&mut v); }
            Err(e) => warn!("Drafts fetch failed [{}] ({}): {}", account.name, drafts_name, e),
        }
        match trash_res {
            Ok(mut v) => { info!("Trash[{}]: {} emails", account.name, v.len()); trash_raw.append(&mut v); }
            Err(e) => warn!("Trash fetch failed [{}] ({}): {}", account.name, trash_name, e),
        }
    }
    info!("Total inbox: {} emails", raw_emails.len());

    let http_client = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let projects = cfg.projects.clone();

    let classify_tasks: Vec<_> = raw_emails
        .iter()
        .map(|email| {
            let client = http_client.clone();
            let claude_cfg = cfg.claude.clone();
            let email = email.clone();
            let projects = projects.clone();
            tokio::spawn(async move { classify(&client, &claude_cfg, &email, &projects).await })
        })
        .collect();

    let results = join_all(classify_tasks).await;

    let cards: Vec<EmailCard> = raw_emails
        .into_iter()
        .zip(results.into_iter())
        .map(|(raw, result)| {
            let classified = match result {
                Ok(Ok(c)) => c,
                Ok(Err(e)) => {
                    warn!("Classification error for uid={}: {}", raw.uid, e);
                    crate::ai_classifier::ClassifiedEmail {
                        priority: Priority::Information,
                        summary: "Unable to classify.".to_string(),
                        project: None,
                        deadline: None,
                        task: None,
                        meeting_link: None,
                    }
                }
                Err(e) => {
                    warn!("Task join error: {}", e);
                    crate::ai_classifier::ClassifiedEmail {
                        priority: Priority::Information,
                        summary: "Classification task failed.".to_string(),
                        project: None,
                        deadline: None,
                        task: None,
                        meeting_link: None,
                    }
                }
            };

            let (from_name, from_email) = parse_from(&raw.from);
            let date_str = raw
                .date
                .map(|d| format_date(d))
                .unwrap_or_else(|| "Unknown".to_string());

            EmailCard {
                uid: raw.uid,
                subject: if raw.subject.is_empty() {
                    "(No Subject)".to_string()
                } else {
                    raw.subject
                },
                from_name,
                from_email,
                date_str,
                summary: classified.summary,
                priority: classified.priority,
                is_read: false,
                body_text: render_body(&raw.body_text, &raw.body_html),
                project: classified.project,
                deadline: classified.deadline,
                task: classified.task,
                meeting_link: classified.meeting_link,
            }
        })
        .collect();

    // Compute live briefing line before consuming cards.
    let urgent_count = cards.iter().filter(|c| c.priority == Priority::Urgent).count();
    let high_count = cards.iter().filter(|c| c.priority == Priority::HighPriority).count();
    let attention = urgent_count + high_count;
    let now = Local::now();
    use chrono::Timelike;
    let live_briefing = format!(
        "Briefed at {:02}:{:02} · {} require{}",
        now.hour(), now.minute(),
        attention,
        if attention == 1 { "s you" } else { " your attention" }
    );

    // Convert folder raws to cards (no AI classification for sent/drafts/trash).
    let sent_cards: Vec<EmailCard> = sent_raw.into_iter().map(raw_to_card).collect();
    let drafts_cards: Vec<EmailCard> = drafts_raw.into_iter().map(raw_to_card).collect();
    let trash_cards: Vec<EmailCard> = trash_raw.into_iter().map(raw_to_card).collect();

    // Persist cards to disk so next launch can display them instantly.
    save_cache(&cards);

    // Signal refresh complete then hand off to shared populate logic.
    {
        let ui_ref = ui.clone();
        invoke_from_event_loop(move || {
            if let Some(handle) = ui_ref.upgrade() {
                handle.set_loading(false);
            }
        })
        .ok();
    }
    populate_ui(cfg, ui, cards, sent_cards, drafts_cards, trash_cards, Some(live_briefing));

    Ok(())
}

fn to_slint_vec(cards: &[EmailCard]) -> Vec<EmailCardData> {
    cards
        .iter()
        .map(|c| EmailCardData {
            uid: c.uid as i32,
            subject: SharedString::from(c.subject.as_str()),
            from_name: SharedString::from(c.from_name.as_str()),
            date_str: SharedString::from(c.date_str.as_str()),
            summary: SharedString::from(c.summary.as_str()),
            priority: SharedString::from(c.priority.as_str()),
            is_read: c.is_read,
            project: SharedString::from(c.project.as_deref().unwrap_or("")),
            deadline_str: SharedString::from(
                c.deadline
                    .as_ref()
                    .map(|d| d.format("%b %d").to_string())
                    .unwrap_or_default()
                    .as_str(),
            ),
            task: SharedString::from(c.task.as_deref().unwrap_or("")),
            body: SharedString::from(c.body_text.as_str()),
            meeting_link: SharedString::from(c.meeting_link.as_deref().unwrap_or("")),
        })
        .collect()
}

fn parse_from(from: &str) -> (String, String) {
    let from = from.trim();
    if let Some(lt) = from.find('<') {
        if let Some(gt) = from.find('>') {
            let name = from[..lt].trim().trim_matches('"').to_string();
            let email = from[lt + 1..gt].trim().to_string();
            let display = if name.is_empty() { email.clone() } else { name };
            return (display, email);
        }
    }
    (from.to_string(), from.to_string())
}

fn format_date(dt: chrono::DateTime<Utc>) -> String {
    use chrono::Local;
    let local = dt.with_timezone(&Local);
    let now = Local::now();
    let diff = now.signed_duration_since(local);
    if diff.num_hours() < 24 {
        format!("{:02}:{:02}", local.hour(), local.minute())
    } else if diff.num_days() < 7 {
        local.format("%a").to_string()
    } else {
        local.format("%b %d").to_string()
    }
}

use chrono::Datelike;
use chrono::Timelike;

fn render_body(plain: &str, html: &Option<String>) -> String {
    if let Some(h) = html {
        if !h.trim().is_empty() {
            return html2text::from_read(h.as_bytes(), 100);
        }
    }
    plain.to_string()
}

fn save_cache(cards: &[EmailCard]) {
    match serde_json::to_string(cards) {
        Ok(json) => {
            if let Err(e) = std::fs::write(CACHE_PATH, json) {
                warn!("Failed to write cache: {}", e);
            } else {
                info!("Cache saved ({} emails)", cards.len());
            }
        }
        Err(e) => warn!("Failed to serialize cache: {}", e),
    }
}

/// Load previously cached emails and populate the UI immediately.
/// Called synchronously at startup before the background refresh begins.
pub fn load_and_display_cache(cfg: &Config, ui: &Weak<AppWindow>) {
    let json = match std::fs::read_to_string(CACHE_PATH) {
        Ok(s) => s,
        Err(_) => return, // No cache yet — first launch.
    };
    let cards: Vec<EmailCard> = match serde_json::from_str(&json) {
        Ok(v) => v,
        Err(e) => {
            warn!("Cache parse error: {}", e);
            return;
        }
    };
    info!("Loaded {} emails from cache", cards.len());
    populate_ui(cfg, ui, cards, vec![], vec![], vec![], None);
}

/// Shared UI-population logic used by both do_refresh and load_and_display_cache.
fn populate_ui(
    cfg: &Config,
    ui: &Weak<AppWindow>,
    cards: Vec<EmailCard>,
    sent: Vec<EmailCard>,
    drafts: Vec<EmailCard>,
    trash: Vec<EmailCard>,
    briefing_override: Option<String>,
) {
    let mut urgent: Vec<EmailCard> = Vec::new();
    let mut high: Vec<EmailCard> = Vec::new();
    let mut other: Vec<EmailCard> = Vec::new();
    let mut info_vec: Vec<EmailCard> = Vec::new();
    let mut threat: Vec<EmailCard> = Vec::new();
    let mut task: Vec<EmailCard> = Vec::new();

    for card in &cards {
        match card.priority {
            Priority::Urgent => urgent.push(card.clone()),
            Priority::HighPriority => high.push(card.clone()),
            Priority::Threat => threat.push(card.clone()),
            Priority::Task => task.push(card.clone()),
            Priority::Information => info_vec.push(card.clone()),
            Priority::OtherPriority => other.push(card.clone()),
        }
    }

    let all_emails = cards.clone();
    let priority_order = |c: &EmailCard| match c.priority {
        Priority::Urgent => 0u8,
        Priority::HighPriority => 1,
        Priority::Task => 2,
        Priority::OtherPriority => 3,
        Priority::Threat => 4,
        Priority::Information => 5,
    };
    let mut all_emails_by_priority = all_emails.clone();
    all_emails_by_priority.sort_by_key(priority_order);

    let task_items: Vec<EmailCard> = cards
        .iter()
        .filter(|c| c.task.is_some() || c.priority == Priority::Task)
        .cloned()
        .collect();

    let important: Vec<EmailCard> = urgent.iter().chain(high.iter()).take(5).cloned().collect();
    let meeting_items: Vec<EmailCard> = cards.iter().filter(|c| c.meeting_link.is_some()).cloned().collect();

    let today = Local::now().date_naive();
    let mut deadlines: Vec<(NaiveDate, EmailCard)> = cards
        .iter()
        .filter_map(|c| c.deadline.map(|d| (d, c.clone())))
        .collect();
    deadlines.sort_by_key(|(d, _)| *d);

    let week_day_data = build_week_days(today, &deadlines);

    let deadline_data: Vec<DeadlineData> = deadlines
        .into_iter()
        .map(|(date, card)| {
            let days_until = (date - today).num_days() as i32;
            let deadline_str = date.format("%b %d").to_string();
            DeadlineData {
                uid: card.uid as i32,
                subject: SharedString::from(card.subject.as_str()),
                deadline_str: SharedString::from(deadline_str),
                project: SharedString::from(card.project.as_deref().unwrap_or("")),
                days_until,
            }
        })
        .collect();

    let attention = urgent.len() + high.len();
    let now = Local::now();
    let briefing = briefing_override.unwrap_or_else(|| format!(
        "Cached · {} require{}",
        attention,
        if attention == 1 { "s you" } else { " your attention" }
    ));

    use chrono::Datelike;
    let cal_year = now.year() as i32;
    let cal_month = now.month() as i32;

    let project_data: Vec<ProjectData> = cfg.projects.iter().map(|p| {
        let email_count = all_emails.iter()
            .filter(|c| c.project.as_deref() == Some(p.name.as_str()))
            .count() as i32;
        ProjectData {
            name: SharedString::from(p.name.as_str()),
            urgency: SharedString::from(p.urgency.as_str()),
            description: SharedString::from(p.description.as_str()),
            email_count,
        }
    }).collect();

    let urgent_slint = to_slint_vec(&urgent);
    let high_slint = to_slint_vec(&high);
    let other_slint = to_slint_vec(&other);
    let info_slint = to_slint_vec(&info_vec);
    let threat_slint = to_slint_vec(&threat);
    let task_slint = to_slint_vec(&task);
    let all_slint = to_slint_vec(&all_emails);
    let task_items_slint = to_slint_vec(&task_items);
    let all_by_priority_slint = to_slint_vec(&all_emails_by_priority);
    let important_slint = to_slint_vec(&important);
    let meeting_slint = to_slint_vec(&meeting_items);
    let sent_slint = to_slint_vec(&sent);
    let drafts_slint = to_slint_vec(&drafts);
    let trash_slint = to_slint_vec(&trash);

    let ui_ref = ui.clone();
    invoke_from_event_loop(move || {
        if let Some(handle) = ui_ref.upgrade() {
            handle.set_briefing_time(SharedString::from(briefing));
            handle.set_urgent_emails(ModelRc::new(VecModel::from(urgent_slint)));
            handle.set_high_emails(ModelRc::new(VecModel::from(high_slint)));
            handle.set_other_emails(ModelRc::new(VecModel::from(other_slint)));
            handle.set_info_emails(ModelRc::new(VecModel::from(info_slint)));
            handle.set_threat_emails(ModelRc::new(VecModel::from(threat_slint)));
            handle.set_task_emails(ModelRc::new(VecModel::from(task_slint)));
            handle.set_all_emails(ModelRc::new(VecModel::from(all_slint)));
            handle.set_task_items(ModelRc::new(VecModel::from(task_items_slint)));
            handle.set_deadlines(ModelRc::new(VecModel::from(deadline_data)));
            handle.set_calendar_year(cal_year);
            handle.set_calendar_month(cal_month);
            handle.set_important_emails(ModelRc::new(VecModel::from(important_slint)));
            handle.set_meeting_items(ModelRc::new(VecModel::from(meeting_slint)));
            handle.set_week_days(ModelRc::new(VecModel::from(week_day_data)));
            handle.set_all_emails_by_priority(ModelRc::new(VecModel::from(all_by_priority_slint)));
            handle.set_projects(ModelRc::new(VecModel::from(project_data)));
            handle.set_sent_emails(ModelRc::new(VecModel::from(sent_slint)));
            handle.set_drafts_emails(ModelRc::new(VecModel::from(drafts_slint)));
            handle.set_trash_emails(ModelRc::new(VecModel::from(trash_slint)));
        }
    })
    .ok();
}

fn build_week_days(today: NaiveDate, deadlines: &[(NaiveDate, EmailCard)]) -> Vec<WeekDayData> {
    use chrono::Datelike;
    let days_from_mon = today.weekday().num_days_from_monday();
    let monday = today - chrono::Duration::days(days_from_mon as i64);
    (0..7).map(|offset| {
        let day = monday + chrono::Duration::days(offset);
        let slots: Vec<&EmailCard> = deadlines.iter()
            .filter(|(d, _)| *d == day)
            .map(|(_, c)| c)
            .take(3)
            .collect();
        let get = |i: usize| slots.get(i)
            .map(|c| (c.subject.clone(), c.uid as i32))
            .unwrap_or_else(|| (String::new(), 0));
        let (s1, u1) = get(0);
        let (s2, u2) = get(1);
        let (s3, u3) = get(2);
        WeekDayData {
            day_name: SharedString::from(day.format("%a").to_string()),
            day_number: day.day() as i32,
            is_today: day == today,
            dl1_subject: SharedString::from(s1),
            dl1_uid: u1,
            dl2_subject: SharedString::from(s2),
            dl2_uid: u2,
            dl3_subject: SharedString::from(s3),
            dl3_uid: u3,
        }
    }).collect()
}
