use anyhow::Result;
use chrono::{Local, NaiveDate, Utc};
use futures::future::join_all;
use html2text;
use reqwest::Client;
use std::sync::{Arc, Mutex};
use tauri::AppHandle;
use tauri::Emitter;
use tracing::{error, info, warn};

use crate::ai_classifier::{OpenTask, RecentMail};
use crate::config::{Config, ImapConfig};
use crate::dismissals::Dismissals;
use crate::tasks::{Board, Status};
use crate::dto::*;
use crate::imap_client::{connect, fetch_emails, fetch_folder};
use crate::models::{EmailCard, Priority};
use crate::state::AppState;

// v2: keyed by real IMAP UIDs (v1 used sequence numbers, which shift on delete).
const CACHE_PATH: &str = "cache-v2.json";

pub async fn refresh(cfg: Arc<Config>, app: AppHandle, state: Arc<Mutex<AppState>>) {
    app.emit("loading", true).ok();

    match do_refresh(&cfg, &app, &state).await {
        Ok(_) => info!("Refresh complete"),
        Err(e) => {
            error!("Refresh failed: {:#}", e);
            app.emit("error", format!("Refresh failed: {}", e)).ok();
            app.emit("loading", false).ok();
        }
    }
}

fn raw_to_card(raw: crate::models::RawEmail) -> EmailCard {
    let (from_name, from_email) = parse_from(&raw.from);
    let date_str = raw.date.map(format_date).unwrap_or_else(|| "Unknown".to_string());
    let date_full = raw.date.map(format_date_full).unwrap_or_default();
    EmailCard {
        uid: raw.uid,
        subject: if raw.subject.is_empty() { "(No Subject)".to_string() } else { raw.subject },
        from_name,
        from_email,
        to: raw.to,
        cc: raw.cc,
        reply_to: raw.reply_to,
        bcc: raw.bcc,
        message_id: raw.message_id,
        date_str,
        date_full,
        summary: String::new(),
        priority: Priority::Information,
        is_read: raw.seen,
        body_text: render_body(&raw.body_text, &raw.body_html),
        body_html: raw.body_html.unwrap_or_default(),
        project: None,
        deadline: None,
        task: None,
        meeting_link: None,
    }
}

async fn do_refresh(cfg: &Config, app: &AppHandle, state: &Arc<Mutex<AppState>>) -> Result<()> {
    let mut raw_emails = Vec::new();
    let mut sent_raw = Vec::new();
    let mut drafts_raw = Vec::new();
    let mut trash_raw = Vec::new();
    let mut inbox_ok = false;

    for account in &cfg.accounts {
        let (sent_name, drafts_name, trash_name, spam_name) = guess_folder_names(&account.imap).await;
        if std::ptr::eq(account, &cfg.accounts[0]) {
            let mut s = state.lock().unwrap();
            s.trash_folder = trash_name.clone();
            s.spam_folder = spam_name;
            s.sent_folder = sent_name.clone();
        }
        let (inbox_res, sent_res, drafts_res, trash_res) = tokio::join!(
            fetch_emails(&account.imap),
            fetch_folder(&account.imap, &sent_name),
            fetch_folder(&account.imap, &drafts_name),
            fetch_folder(&account.imap, &trash_name),
        );
        match inbox_res {
            Ok(mut v) => { info!("Inbox[{}]: {} emails", account.name, v.len()); raw_emails.append(&mut v); inbox_ok = true; }
            Err(e) => warn!("Inbox[{}]: {}", account.name, e),
        }
        match sent_res {
            Ok(mut v) => { sent_raw.append(&mut v); }
            Err(e) => warn!("Sent[{}]: {}", account.name, e),
        }
        match drafts_res {
            Ok(mut v) => { drafts_raw.append(&mut v); }
            Err(e) => warn!("Drafts[{}]: {}", account.name, e),
        }
        match trash_res {
            Ok(mut v) => { trash_raw.append(&mut v); }
            Err(e) => warn!("Trash[{}]: {}", account.name, e),
        }
    }

    // If no inbox could be fetched, keep the cache untouched rather than wiping it.
    if !inbox_ok {
        anyhow::bail!("could not fetch any inbox (check IMAP settings)");
    }

    // Evidence for the task check, captured before the raw emails are consumed.
    let inbox_dates: std::collections::HashMap<u32, chrono::DateTime<Utc>> = raw_emails.iter()
        .filter_map(|r| r.date.map(|d| (r.uid, d))).collect();
    let recent_inbox: Vec<(chrono::DateTime<Utc>, RecentMail)> = raw_emails.iter()
        .filter_map(|r| r.date.map(|d| (d, to_recent_mail("inbox", r)))).collect();
    let recent_sent: Vec<(chrono::DateTime<Utc>, RecentMail)> = sent_raw.iter()
        .filter_map(|r| r.date.map(|d| (d, to_recent_mail("sent", r)))).collect();

    // Load previously classified emails from cache — reuse results for known UIDs.
    let mut cached_cards = load_cache();
    let cached_uids: std::collections::HashSet<u32> = cached_cards.iter().map(|c| c.uid).collect();

    // Split inbox into already-known (reuse cache) and new (need AI).
    let (known_raw, new_raw): (Vec<_>, Vec<_>) = raw_emails.into_iter()
        .partition(|r| cached_uids.contains(&r.uid));

    // Merge known emails from cache, updating body/date in case they changed.
    for raw in &known_raw {
        if let Some(c) = cached_cards.iter_mut().find(|c| c.uid == raw.uid) {
            c.body_text = render_body(&raw.body_text, &raw.body_html);
            c.body_html = raw.body_html.clone().unwrap_or_default();
            c.to = raw.to.clone();
            c.cc = raw.cc.clone();
            c.reply_to = raw.reply_to.clone();
            c.bcc = raw.bcc.clone();
            c.message_id = raw.message_id.clone();
            c.date_full = raw.date.map(format_date_full).unwrap_or_default();
            c.is_read = raw.seen;
        }
    }

    // Emit immediately with cached data + raw new emails (unclassified) so UI isn't blank.
    let new_raw_cards: Vec<EmailCard> = new_raw.iter().map(|r| raw_to_card(r.clone())).collect();
    let sent_cards_pre: Vec<EmailCard> = sent_raw.iter().map(|r| raw_to_card(r.clone())).collect();
    let drafts_cards_pre: Vec<EmailCard> = drafts_raw.iter().map(|r| raw_to_card(r.clone())).collect();
    let trash_cards_pre: Vec<EmailCard> = trash_raw.iter().map(|r| raw_to_card(r.clone())).collect();
    let mut pre_cards = cached_cards.clone();
    pre_cards.extend(new_raw_cards);
    let new_count = new_raw.len();
    let briefing_pre = if new_count > 0 {
        format!("Classifying {} new email{} with AI…", new_count, if new_count == 1 { "" } else { "s" })
    } else {
        "Up to date.".to_string()
    };
    populate(cfg, app, state, pre_cards, sent_cards_pre.clone(), drafts_cards_pre.clone(), trash_cards_pre.clone(), Some(briefing_pre));

    // Classify only new emails.
    let new_cards: Vec<EmailCard> = if new_raw.is_empty() {
        vec![]
    } else {
        let http_client = Client::builder().timeout(std::time::Duration::from_secs(30)).build()?;
        let projects = cfg.projects.clone();
        let classify_tasks: Vec<_> = new_raw.iter().map(|email| {
            let client = http_client.clone();
            let claude_cfg = cfg.claude.clone();
            let email = email.clone();
            let projects = projects.clone();
            tokio::spawn(async move { crate::ai_classifier::classify(&client, &claude_cfg, &email, &projects).await })
        }).collect();
        let results = join_all(classify_tasks).await;
        new_raw.into_iter().zip(results).map(|(raw, result)| {
            let classified = match result {
                Ok(Ok(c)) => c,
                _ => crate::ai_classifier::ClassifiedEmail {
                    priority: Priority::Information,
                    summary: String::new(),
                    project: None, deadline: None, task: None, meeting_link: None,
                },
            };
            let (from_name, from_email) = parse_from(&raw.from);
            let date_str = raw.date.map(format_date).unwrap_or_else(|| "Unknown".to_string());
            let date_full = raw.date.map(format_date_full).unwrap_or_default();
            let body_html = raw.body_html.clone().unwrap_or_default();
            EmailCard {
                uid: raw.uid,
                subject: if raw.subject.is_empty() { "(No Subject)".to_string() } else { raw.subject },
                from_name, from_email,
                to: raw.to, cc: raw.cc, reply_to: raw.reply_to, bcc: raw.bcc, message_id: raw.message_id,
                date_str, date_full,
                summary: classified.summary,
                priority: classified.priority,
                is_read: raw.seen,
                body_text: render_body(&raw.body_text, &raw.body_html),
                body_html,
                project: classified.project,
                deadline: classified.deadline,
                task: classified.task,
                meeting_link: classified.meeting_link,
            }
        }).collect()
    };

    // Merge: cached + newly classified. Remove UIDs no longer in inbox.
    let all_uids: std::collections::HashSet<u32> = cached_uids.iter()
        .chain(new_cards.iter().map(|c| &c.uid)).copied().collect();
    // Keep only emails still present on server (by UID range we fetched).
    // Simple approach: keep cached entries whose UID appears in known_raw, plus new.
    let known_uid_set: std::collections::HashSet<u32> = known_raw.iter().map(|r| r.uid).collect();
    cached_cards.retain(|c| known_uid_set.contains(&c.uid));
    let mut cards = cached_cards;
    cards.extend(new_cards);
    let _ = all_uids; // suppress warning

    let sent_cards: Vec<EmailCard> = sent_raw.into_iter().map(raw_to_card).collect();
    let drafts_cards: Vec<EmailCard> = drafts_raw.into_iter().map(raw_to_card).collect();
    let trash_cards: Vec<EmailCard> = trash_raw.into_iter().map(raw_to_card).collect();

    save_cache(&cards);

    if let Err(e) = check_open_tasks(cfg, &cards, &inbox_dates, recent_inbox, recent_sent).await {
        warn!("Task check failed: {:#}", e);
    }

    let urgent_count = cards.iter().filter(|c| c.priority == Priority::Urgent).count();
    let high_count   = cards.iter().filter(|c| c.priority == Priority::HighPriority).count();
    let attention = urgent_count + high_count;
    use chrono::Timelike;
    let now = Local::now();
    let live_briefing = format!(
        "Briefed at {:02}:{:02} · {} require{}",
        now.hour(), now.minute(),
        attention, if attention == 1 { "s you" } else { " your attention" }
    );

    populate(cfg, app, state, cards, sent_cards, drafts_cards, trash_cards, Some(live_briefing));
    Ok(())
}

pub fn load_and_display_cache(cfg: &Config, app: &AppHandle, state: &Arc<Mutex<AppState>>) {
    let json = match std::fs::read_to_string(CACHE_PATH) {
        Ok(s) => s,
        Err(_) => return,
    };
    let cards: Vec<EmailCard> = match serde_json::from_str(&json) {
        Ok(v) => v,
        Err(e) => { warn!("Cache parse error: {}", e); return; }
    };
    info!("Loaded {} emails from cache", cards.len());
    populate(cfg, app, state, cards, vec![], vec![], vec![], None);
}

fn populate(
    cfg: &Config,
    app: &AppHandle,
    state: &Arc<Mutex<AppState>>,
    cards: Vec<EmailCard>,
    sent: Vec<EmailCard>,
    drafts: Vec<EmailCard>,
    trash: Vec<EmailCard>,
    briefing_override: Option<String>,
) {
    // Projects the user re-tagged by hand win over the AI's choice.
    let overrides = crate::overrides::Overrides::load();
    let cards: Vec<EmailCard> = cards.into_iter().map(|mut c| {
        overrides.apply(&mut c);
        c
    }).collect();

    let mut urgent = Vec::new(); let mut high = Vec::new();
    let mut other  = Vec::new(); let mut info_vec = Vec::new();
    let mut threat = Vec::new(); let mut task_vec = Vec::new();

    for card in &cards {
        match card.priority {
            Priority::Urgent       => urgent.push(card.clone()),
            Priority::HighPriority => high.push(card.clone()),
            Priority::Threat       => threat.push(card.clone()),
            Priority::Task         => task_vec.push(card.clone()),
            Priority::Information  => info_vec.push(card.clone()),
            Priority::OtherPriority => other.push(card.clone()),
        }
    }

    let all_emails = cards.clone();
    let priority_order = |c: &EmailCard| match c.priority {
        Priority::Urgent        => 0u8,
        Priority::HighPriority  => 1,
        Priority::Task          => 2,
        Priority::OtherPriority => 3,
        Priority::Threat        => 4,
        Priority::Information   => 5,
    };
    let mut all_by_priority = all_emails.clone();
    all_by_priority.sort_by_key(priority_order);

    let dismissed = Dismissals::load();
    // New email action items land in the Todo column of the task board.
    let needs_sync = {
        let board = Board::load();
        cards.iter().any(|c| (c.task.is_some() || c.priority == Priority::Task)
            && !board.deleted_sources.contains(&c.uid)
            && !board.tasks.iter().any(|t| t.source_uid == Some(c.uid)))
    };
    if needs_sync {
        Board::update(|b| b.sync_from_cards(&cards, &dismissed));
    }
    let board_tasks = Board::load().tasks;
    let important = important_unread(&urgent, &high);
    let meeting_items: Vec<EmailCard> = cards.iter()
        .filter(|c| c.meeting_link.is_some() && !dismissed.meetings.contains(&c.uid))
        .cloned().collect();

    let today = Local::now().date_naive();
    let mut deadlines: Vec<(NaiveDate, EmailCard)> = cards.iter()
        .filter(|c| !dismissed.deadlines.contains(&c.uid))
        .filter_map(|c| c.deadline.map(|d| (d, c.clone()))).collect();
    deadlines.sort_by_key(|(d, _)| *d);

    let week_days = build_week_days(today, &deadlines);

    let deadline_dtos: Vec<DeadlineDto> = deadlines.iter().map(|(date, card)| DeadlineDto {
        uid: card.uid,
        subject: card.subject.clone(),
        deadline_str: date.format("%b %d").to_string(),
        date_iso: date.format("%Y-%m-%d").to_string(),
        project: card.project.clone().unwrap_or_default(),
        days_until: (*date - today).num_days() as i32,
    }).collect();

    let attention = urgent.len() + high.len();
    let briefing = briefing_override.unwrap_or_else(|| format!(
        "Cached · {} require{}", attention,
        if attention == 1 { "s you" } else { " your attention" }
    ));

    use chrono::Datelike;
    let now = Local::now();
    let cal_year  = now.year();
    let cal_month = now.month();

    let project_dtos: Vec<ProjectDto> = cfg.projects.iter().map(|p| {
        let email_count = all_emails.iter()
            .filter(|c| c.project.as_deref() == Some(&p.name)).count() as i32;
        ProjectDto { name: p.name.clone(), urgency: p.urgency.clone(), description: p.description.clone(), email_count }
    }).collect();

    // Store all emails in shared state for command lookups.
    {
        let mut s = state.lock().unwrap();
        let mut all: Vec<EmailCard> = all_emails.clone();
        all.extend(sent.clone());
        all.extend(drafts.clone());
        all.extend(trash.clone());
        s.emails = all;
    }

    use chrono::Timelike;
    let hour = Local::now().hour();
    let time_word = if hour < 12 { "morning" } else if hour < 17 { "afternoon" } else { "evening" };
    let greeting = format!("Good {}, {}.", time_word, cfg.user.display_name);

    let snapshot = AppStateDto {
        greeting,
        briefing_time: briefing,
        loading: false,
        urgent_emails:        to_dto_vec(&urgent),
        high_emails:          to_dto_vec(&high),
        other_emails:         to_dto_vec(&other),
        info_emails:          to_dto_vec(&info_vec),
        threat_emails:        to_dto_vec(&threat),
        task_emails:          to_dto_vec(&task_vec),
        all_emails:           to_dto_vec(&all_emails),
        tasks:                board_tasks,
        meeting_items:        to_dto_vec(&meeting_items),
        important_emails:     to_dto_vec(&important),
        all_emails_by_priority: to_dto_vec(&all_by_priority),
        sent_emails:          to_dto_vec(&sent),
        drafts_emails:        to_dto_vec(&drafts),
        trash_emails:         to_dto_vec(&trash),
        deadlines:            deadline_dtos,
        week_days,
        projects:             project_dtos,
        calendar_year:        cal_year,
        calendar_month:       cal_month,
    };

    app.emit("state-updated", snapshot).ok();
}

fn to_dto_vec(cards: &[EmailCard]) -> Vec<EmailCardDto> {
    cards.iter().map(|c| EmailCardDto {
        uid:          c.uid,
        subject:      c.subject.clone(),
        from_name:    c.from_name.clone(),
        from_email:   c.from_email.clone(),
        to:           c.to.clone(),
        cc:           c.cc.clone(),
        reply_to:     c.reply_to.clone(),
        bcc:          c.bcc.clone(),
        message_id:   c.message_id.clone(),
        date_str:     c.date_str.clone(),
        date_full:    c.date_full.clone(),
        summary:      c.summary.clone(),
        priority:     c.priority.as_str().to_string(),
        is_read:      c.is_read,
        project:      c.project.clone().unwrap_or_default(),
        deadline_str: c.deadline.map(|d| d.format("%b %d").to_string()).unwrap_or_default(),
        task:         c.task.clone().unwrap_or_default(),
        body_text:    c.body_text.clone(),
        body_html:    c.body_html.clone(),
        meeting_link: c.meeting_link.clone().unwrap_or_default(),
    }).collect()
}

fn build_week_days(today: NaiveDate, deadlines: &[(NaiveDate, EmailCard)]) -> Vec<WeekDayDto> {
    use chrono::Datelike;
    let days_from_mon = today.weekday().num_days_from_monday();
    let monday = today - chrono::Duration::days(days_from_mon as i64);
    (0..7).map(|offset| {
        let day = monday + chrono::Duration::days(offset);
        let slots: Vec<&EmailCard> = deadlines.iter()
            .filter(|(d, _)| *d == day).map(|(_, c)| c).take(3).collect();
        let get = |i: usize| slots.get(i)
            .map(|c| (c.subject.clone(), c.uid))
            .unwrap_or_else(|| (String::new(), 0));
        let (s1, u1) = get(0); let (s2, u2) = get(1); let (s3, u3) = get(2);
        WeekDayDto {
            day_name: day.format("%a").to_string(),
            day_number: day.day(),
            is_today: day == today,
            dl1_subject: s1, dl1_uid: u1,
            dl2_subject: s2, dl2_uid: u2,
            dl3_subject: s3, dl3_uid: u3,
        }
    }).collect()
}

fn parse_from(from: &str) -> (String, String) {
    let from = from.trim();
    if let Some(lt) = from.find('<') {
        if let Some(gt) = from.find('>') {
            let name = from[..lt].trim().trim_matches('"').to_string();
            let email = from[lt+1..gt].trim().to_string();
            let display = if name.is_empty() { email.clone() } else { name };
            return (display, email);
        }
    }
    (from.to_string(), from.to_string())
}

fn format_date(dt: chrono::DateTime<Utc>) -> String {
    use chrono::{Local, Timelike};
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

fn format_date_full(dt: chrono::DateTime<Utc>) -> String {
    dt.with_timezone(&Local).format("%A %d %B %Y, %H:%M").to_string()
}

fn render_body(plain: &str, html: &Option<String>) -> String {
    if let Some(h) = html {
        if !h.trim().is_empty() {
            return html2text::from_read(h.as_bytes(), 100);
        }
    }
    plain.to_string()
}

fn load_cache() -> Vec<EmailCard> {
    match std::fs::read_to_string(CACHE_PATH) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => vec![],
    }
}

fn save_cache(cards: &[EmailCard]) {
    match serde_json::to_string(cards) {
        Ok(json) => { std::fs::write(CACHE_PATH, json).ok(); }
        Err(e) => warn!("Cache serialize error: {}", e),
    }
}

async fn guess_folder_names(cfg: &ImapConfig) -> (String, String, String, String) {
    use futures::StreamExt;

    let mut session = match connect(cfg).await {
        Ok(s) => s,
        Err(_) => return ("Sent".into(), "Drafts".into(), "Trash".into(), "Junk".into()),
    };
    let names: Vec<String> = match session.list(Some(""), Some("*")).await {
        Ok(stream) => {
            futures::pin_mut!(stream);
            let mut out = Vec::new();
            while let Some(item) = stream.next().await {
                if let Ok(n) = item { out.push(n.name().to_string()); }
            }
            out
        }
        Err(_) => vec![],
    };
    session.logout().await.ok();

    let pick = |candidates: &[&str]| -> String {
        for c in candidates {
            if let Some(n) = names.iter().find(|n| n.to_lowercase() == c.to_lowercase()) {
                return n.clone();
            }
        }
        for c in candidates {
            if let Some(n) = names.iter().find(|n| n.to_lowercase().contains(&c.to_lowercase())) {
                return n.clone();
            }
        }
        candidates[0].to_string()
    };

    let sent   = pick(&["Sent", "Sent Items", "Sent Messages"]);
    let drafts = pick(&["Drafts", "Draft"]);
    let trash  = pick(&["Trash", "Deleted Items", "Deleted Messages"]);
    let spam   = pick(&["Junk", "Spam", "Junk E-mail", "Courrier indésirable"]);
    info!("Folders: sent={} drafts={} trash={} spam={}", sent, drafts, trash, spam);
    (sent, drafts, trash, spam)
}

fn to_recent_mail(folder: &'static str, raw: &crate::models::RawEmail) -> RecentMail {
    RecentMail {
        folder,
        from: raw.from.clone(),
        to: raw.to.clone(),
        subject: raw.subject.clone(),
        date: raw.date.map(format_date_full).unwrap_or_default(),
        snippet: render_body(&raw.body_text, &raw.body_html).chars().take(300).collect(),
    }
}

/// Ask the AI whether open tasks were completed or became moot, judging from
/// sent mail of the last 14 days and inbox mail newer than the oldest open task.
/// "done" moves the task to Done, "obsolete" to Given up on the task board.
async fn check_open_tasks(
    cfg: &Config,
    cards: &[EmailCard],
    inbox_dates: &std::collections::HashMap<u32, chrono::DateTime<Utc>>,
    recent_inbox: Vec<(chrono::DateTime<Utc>, RecentMail)>,
    recent_sent: Vec<(chrono::DateTime<Utc>, RecentMail)>,
) -> Result<()> {
    // Make sure new email tasks are on the board before checking them.
    let dismissed = Dismissals::load();
    let board = Board::update(|b| { b.sync_from_cards(cards, &dismissed); b.tasks.clone() });
    let open: Vec<OpenTask> = board.iter()
        .filter(|t| t.status.is_open())
        .filter_map(|t| {
            let uid = t.source_uid?;
            let card = cards.iter().find(|c| c.uid == uid)?;
            Some(OpenTask {
                uid,
                task: t.title.clone(),
                subject: card.subject.clone(),
                from: card.from_name.clone(),
                date: card.date_full.clone(),
            })
        })
        .collect();
    if open.is_empty() {
        return Ok(());
    }

    let open_uids: std::collections::HashSet<u32> = open.iter().map(|t| t.uid).collect();
    let oldest = open.iter().filter_map(|t| inbox_dates.get(&t.uid)).min().copied();
    let sent_cutoff = Utc::now() - chrono::Duration::days(14);

    let mut recent: Vec<(chrono::DateTime<Utc>, RecentMail)> = recent_sent.into_iter()
        .filter(|(d, _)| *d >= sent_cutoff)
        .collect();
    if let Some(oldest) = oldest {
        recent.extend(recent_inbox.into_iter().filter(|(d, _)| *d > oldest));
    }
    // Newest first, capped to keep the prompt small.
    recent.sort_by(|a, b| b.0.cmp(&a.0));
    recent.truncate(40);
    let recent: Vec<RecentMail> = recent.into_iter().map(|(_, m)| m).collect();
    if recent.is_empty() {
        return Ok(());
    }

    info!("Checking {} open task(s) against {} recent email(s)", open.len(), recent.len());
    let client = Client::builder().timeout(std::time::Duration::from_secs(60)).build()?;
    let verdicts = crate::ai_classifier::check_tasks(&client, &cfg.claude, &open, &recent).await?;
    if verdicts.is_empty() {
        return Ok(());
    }
    Board::update(|b| {
        for (uid, verdict) in verdicts {
            if !open_uids.contains(&uid) {
                continue;
            }
            // Only move tasks still open: the user may have moved one meanwhile.
            if let Some(t) = b.tasks.iter_mut()
                .find(|t| t.source_uid == Some(uid) && t.status.is_open())
            {
                info!("Task uid={} is {}", uid, verdict);
                t.status = if verdict == "done" { Status::Done } else { Status::GivenUp };
                t.ai_verdict = Some(verdict.to_string());
            }
        }
    });
    Ok(())
}

/// Unread urgent + high emails for the Overview, at most 5.
pub fn important_unread(urgent: &[EmailCard], high: &[EmailCard]) -> Vec<EmailCard> {
    urgent.iter().chain(high.iter()).filter(|c| !c.is_read).take(5).cloned().collect()
}

/// Set one email's read state in the on-disk cache.
pub fn set_read_in_cache(uid: u32, read: bool) {
    let mut cards = load_cache();
    if let Some(c) = cards.iter_mut().find(|c| c.uid == uid) {
        c.is_read = read;
        save_cache(&cards);
    }
}

/// Drop one email from the on-disk cache (after it was moved to the trash).
pub fn remove_from_cache(uid: u32) {
    let mut cards = load_cache();
    cards.retain(|c| c.uid != uid);
    save_cache(&cards);
}
