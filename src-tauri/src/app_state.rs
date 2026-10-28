use anyhow::Result;
use chrono::{Local, NaiveDate, Utc};
use std::sync::{Arc, Mutex};
use tauri::AppHandle;
use tauri::Emitter;
use tracing::{error, info, warn};

use crate::ai_classifier::{OpenTask, RecentMail};
use crate::config::{Config, ImapConfig};
use crate::dismissals::Dismissals;
use crate::dto::*;
use crate::i18n;
use crate::imap_client::{connect, fetch_emails, fetch_folder};
use crate::models::{EmailCard, Priority, RawEmail};
use crate::state::AppState;
use crate::tasks::{Board, Status};

// v2: keyed by real IMAP UIDs (v1 used sequence numbers, which shift on delete).
const CACHE_PATH: &str = "cache-v2.json";

pub async fn refresh(cfg: Arc<Config>, app: AppHandle, state: Arc<Mutex<AppState>>) {
    // One refresh at a time. A request made meanwhile (new settings saved, the refresh
    // button) is not dropped: one more refresh runs right after, with the settings of then.
    use std::sync::atomic::{AtomicBool, Ordering};
    static REFRESHING: AtomicBool = AtomicBool::new(false);
    static AGAIN: AtomicBool = AtomicBool::new(false);
    if REFRESHING.swap(true, Ordering::SeqCst) {
        AGAIN.store(true, Ordering::SeqCst);
        return;
    }
    let mut cfg = cfg;
    loop {
        app.emit("loading", true).ok();
        match do_refresh(&cfg, &app, &state).await {
            Ok(_) => info!("Refresh complete"),
            Err(e) => {
                error!("Refresh failed: {:#}", e);
                app.emit("error", format!("Refresh failed: {}", e)).ok();
            }
        }
        // Hide the loading bar whether the refresh succeeded or failed.
        app.emit("loading", false).ok();
        REFRESHING.store(false, Ordering::SeqCst);
        if !AGAIN.swap(false, Ordering::SeqCst) || REFRESHING.swap(true, Ordering::SeqCst) {
            break;
        }
        cfg = state.lock().unwrap().cfg.clone();
    }
}

/// Notifications for newly arrived emails as (title, body): one per email (sender, subject)
/// for up to 3, otherwise a single "N new emails" listing the first senders. Read ones are skipped.
pub fn notification_texts(cards: &[EmailCard], l: i18n::Lang) -> Vec<(String, String)> {
    let unread: Vec<&EmailCard> = cards.iter().filter(|c| !c.is_read).collect();
    if unread.len() <= 3 {
        return unread
            .iter()
            .map(|c| {
                let from = if c.from_name.is_empty() { c.from_email.clone() } else { c.from_name.clone() };
                (from, c.subject.clone())
            })
            .collect();
    }
    let mut senders: Vec<&str> = Vec::new();
    for c in &unread {
        let from = if c.from_name.is_empty() { c.from_email.as_str() } else { c.from_name.as_str() };
        if !senders.contains(&from) {
            senders.push(from);
        }
    }
    let mut body = senders.iter().take(3).copied().collect::<Vec<_>>().join(", ");
    if senders.len() > 3 {
        body.push_str(", …");
    }
    vec![(i18n::new_emails(unread.len(), l), body)]
}

fn notify_new_mail(app: &AppHandle, cards: &[EmailCard]) {
    use tauri_plugin_notification::NotificationExt;
    for (title, body) in notification_texts(cards, i18n::lang()) {
        if let Err(e) = app.notification().builder().title(title).body(body).show() {
            warn!("Notification failed: {}", e);
        }
    }
}

pub fn raw_to_card(raw: crate::models::RawEmail) -> EmailCard {
    let (from_name, from_email) = parse_from(&raw.from);
    let date_str = raw.date.map(format_date).unwrap_or_else(|| i18n::unknown_date(i18n::lang()).to_string());
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
        references: raw.references,
        date_str,
        date_full,
        date_ts: raw.date.map(|d| d.timestamp()).unwrap_or(0),
        summary: String::new(),
        priority: Priority::Information,
        is_read: raw.seen,
        body_text: render_body(&raw.body_text, &raw.body_html),
        body_html: raw.body_html.unwrap_or_default(),
        attachments: raw.attachments,
        html_only: raw.html_only,
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
    let mut spam_raw = Vec::new();
    let mut inbox_ok = false;
    let mut mailboxes = Vec::new();

    for (index, account) in cfg.accounts.iter().enumerate() {
        let folders = guess_folder_names(&account.imap).await;
        let (sent_name, drafts_name, trash_name) =
            (folders.sent.clone(), folders.drafts.clone(), folders.trash.clone());
        let spam_name = folders.spam.clone();
        {
            let mut s = state.lock().unwrap();
            s.sent_folders.resize(cfg.accounts.len(), "Sent".into());
            s.sent_folders[index] = sent_name.clone();
        }
        mailboxes.push((account.clone(), folders));
        let (inbox_res, sent_res, drafts_res, trash_res, spam_res) = tokio::join!(
            fetch_emails(&account.imap),
            fetch_folder(&account.imap, &sent_name),
            fetch_folder(&account.imap, &drafts_name),
            fetch_folder(&account.imap, &trash_name),
            async {
                // No spam folder yet: nothing to show (it is created on the first "Spam").
                match &spam_name {
                    Some(name) => fetch_folder(&account.imap, name).await,
                    None => Ok(Vec::new()),
                }
            },
        );
        // Ids carry the account, so two accounts' emails never share one.
        let tag = |res: Result<Vec<RawEmail>>| {
            res.map(|mut v| {
                for r in &mut v {
                    r.uid = crate::models::mail_id(index, r.uid as u32);
                }
                v
            })
        };
        let (inbox_res, sent_res, drafts_res, trash_res, spam_res) =
            (tag(inbox_res), tag(sent_res), tag(drafts_res), tag(trash_res), tag(spam_res));
        match inbox_res {
            Ok(mut v) => {
                info!("Inbox[{}]: {} emails", account.name, v.len());
                raw_emails.append(&mut v);
                inbox_ok = true;
            }
            Err(e) => warn!("Inbox[{}]: {}", account.name, e),
        }
        match sent_res {
            Ok(mut v) => {
                sent_raw.append(&mut v);
            }
            Err(e) => warn!("Sent[{}]: {}", account.name, e),
        }
        match drafts_res {
            Ok(mut v) => {
                drafts_raw.append(&mut v);
            }
            Err(e) => warn!("Drafts[{}]: {}", account.name, e),
        }
        match trash_res {
            Ok(mut v) => {
                trash_raw.append(&mut v);
            }
            Err(e) => warn!("Trash[{}]: {}", account.name, e),
        }
        match spam_res {
            Ok(mut v) => {
                spam_raw.append(&mut v);
            }
            Err(e) => warn!("Spam[{}]: {}", account.name, e),
        }
    }

    // If no inbox could be fetched, keep the cache untouched rather than wiping it.
    if !inbox_ok {
        anyhow::bail!("could not fetch any inbox (check IMAP settings)");
    }

    // Evidence for the task check, captured before the raw emails are consumed.
    let inbox_dates: std::collections::HashMap<u64, chrono::DateTime<Utc>> =
        raw_emails.iter().filter_map(|r| r.date.map(|d| (r.uid, d))).collect();
    let recent_inbox: Vec<(chrono::DateTime<Utc>, RecentMail)> =
        raw_emails.iter().filter_map(|r| r.date.map(|d| (d, to_recent_mail("inbox", r)))).collect();
    let recent_sent: Vec<(chrono::DateTime<Utc>, RecentMail)> =
        sent_raw.iter().filter_map(|r| r.date.map(|d| (d, to_recent_mail("sent", r)))).collect();

    // Load previously classified emails from cache — reuse results for known UIDs.
    // Emails the AI has not sorted yet (it was not set up, or it failed) are sorted again.
    let mut cached_cards = load_cache();
    migrate_account_ids(&mut cached_cards, &raw_emails);
    let seen_uids: std::collections::HashSet<u64> = cached_cards.iter().map(|c| c.uid).collect();
    cached_cards.retain(is_sorted);
    // A cached email whose Message-ID differs is another email under the same id (older
    // versions gave two accounts' emails the same one): it is sorted again.
    let server_ids: std::collections::HashMap<u64, &str> =
        raw_emails.iter().map(|r| (r.uid, r.message_id.as_str())).collect();
    cached_cards.retain(|c| match server_ids.get(&c.uid) {
        Some(id) => id.is_empty() || c.message_id.is_empty() || *id == c.message_id,
        None => true,
    });
    let cached_uids: std::collections::HashSet<u64> = cached_cards.iter().map(|c| c.uid).collect();
    let ai_ready = crate::ai::readiness(&cfg.ai);

    // Split inbox into already-known (reuse cache) and new (need AI).
    let (known_raw, new_raw): (Vec<_>, Vec<_>) = raw_emails.into_iter().partition(|r| cached_uids.contains(&r.uid));

    // Merge known emails from cache, updating body/date in case they changed.
    for raw in &known_raw {
        if let Some(c) = cached_cards.iter_mut().find(|c| c.uid == raw.uid) {
            c.body_text = render_body(&raw.body_text, &raw.body_html);
            c.body_html = raw.body_html.clone().unwrap_or_default();
            c.attachments = raw.attachments.clone();
            c.html_only = raw.html_only;
            c.to = raw.to.clone();
            c.cc = raw.cc.clone();
            c.reply_to = raw.reply_to.clone();
            c.bcc = raw.bcc.clone();
            c.message_id = raw.message_id.clone();
            c.references = raw.references.clone();
            c.date_full = raw.date.map(format_date_full).unwrap_or_default();
            c.date_ts = raw.date.map(|d| d.timestamp()).unwrap_or(0);
            c.date_str = raw.date.map(format_date).unwrap_or_else(|| i18n::unknown_date(i18n::lang()).to_string());
            c.is_read = raw.seen;
        }
    }

    // Emit immediately with cached data + raw new emails (unclassified) so UI isn't blank.
    let new_raw_cards: Vec<EmailCard> = new_raw.iter().map(|r| raw_to_card(r.clone())).collect();
    let sent_cards_pre: Vec<EmailCard> = sent_raw.iter().map(|r| raw_to_card(r.clone())).collect();
    let drafts_cards_pre: Vec<EmailCard> = drafts_raw.iter().map(|r| raw_to_card(r.clone())).collect();
    let trash_cards_pre: Vec<EmailCard> = trash_raw.iter().map(|r| raw_to_card(r.clone())).collect();
    let spam_cards_pre: Vec<EmailCard> = spam_raw.iter().map(|r| raw_to_card(r.clone())).collect();
    let mut pre_cards = cached_cards.clone();
    pre_cards.extend(new_raw_cards);
    let new_count = new_raw.len();
    let briefing_pre = if new_count > 0 {
        Briefing { kind: "classifying", count: new_count, ..Default::default() }
    } else {
        Briefing { kind: "up_to_date", ..Default::default() }
    };
    // From here the emails shown carry this refresh's ids: actions resolve them with its accounts.
    {
        let mut s = state.lock().unwrap();
        if accounts_changed(&s.cfg, cfg) {
            return Ok(()); // accounts changed while fetching: the refresh runs again
        }
        s.mailboxes = mailboxes;
    }
    populate(
        cfg,
        app,
        state,
        pre_cards,
        sent_cards_pre.clone(),
        drafts_cards_pre.clone(),
        trash_cards_pre.clone(),
        spam_cards_pre.clone(),
        Some(briefing_pre),
    );

    // Classify only new emails.
    let mut ai_failures = 0usize;
    let mut first_ai_error: Option<String> = None;
    // Why a Claude subscription sorted nothing, or stopped early (shown in the briefing line).
    let mut ai_notice: Option<Briefing> = None;
    let new_cards: Vec<EmailCard> = if new_raw.is_empty() {
        vec![]
    } else {
        let projects = cfg.projects.clone();
        // Emails already classified on another computer: reuse the stored result (no AI call).
        let ids: Vec<String> = new_raw.iter().map(|r| r.message_id.clone()).filter(|id| !id.is_empty()).collect();
        let stored = crate::cloud::lookup_ai(&ids).await;
        let concurrency = crate::ai::parallel_requests(&cfg.ai);
        use futures::StreamExt;
        // Owned jobs first: the stream must not borrow from this function across awaits.
        let jobs: Vec<_> = new_raw
            .iter()
            .map(|email| (stored.get(&email.message_id).cloned(), cfg.ai.clone(), email.clone(), projects.clone()))
            .collect();
        // With a Claude subscription, only a paid plan is used (a free plan or an API-key
        // login would bill the API), and after one email fails (e.g. the usage limit is reached)
        // the rest stay unsorted until the next refresh instead of each failing in turn.
        let claude_code = cfg.ai.uses_claude_code();
        let not_paid = claude_code && !crate::claude_code::status().await.paid;
        let stop_reason: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let results: Vec<anyhow::Result<(bool, crate::ai_classifier::ClassifiedEmail)>> = futures::stream::iter(jobs)
            .map(|(reused, ai_cfg, email, projects)| {
                let stop_reason = stop_reason.clone();
                async move {
                    if let Some(c) = reused {
                        return anyhow::Ok((false, c));
                    }
                    if not_paid || stop_reason.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
                        return anyhow::Ok((false, crate::ai_classifier::unclassified()));
                    }
                    let result = crate::ai_classifier::classify(&ai_cfg, &email, &projects).await;
                    if let (true, Err(e)) = (claude_code, &result) {
                        stop_reason.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(|| e.to_string());
                    }
                    anyhow::Ok((true, result?))
                }
            })
            .buffered(concurrency)
            .collect()
            .await;
        let stop_reason = stop_reason.lock().unwrap_or_else(|e| e.into_inner()).take();
        ai_notice = if not_paid {
            Some(Briefing { kind: "ai_not_paid", ..Default::default() })
        } else {
            stop_reason.map(|detail| Briefing { kind: "ai_stopped", detail, ..Default::default() })
        };
        let mut to_store: Vec<(String, crate::ai_classifier::ClassifiedEmail)> = Vec::new();
        let cards: Vec<EmailCard> = new_raw
            .into_iter()
            .zip(results)
            .map(|(raw, result)| {
                let classified = match result {
                    Ok((fresh, c)) => {
                        if fresh && !raw.message_id.is_empty() && crate::ai_classifier::is_real_result(&c) {
                            to_store.push((raw.message_id.clone(), c.clone()));
                        }
                        c
                    }
                    // Left unsorted (empty summary): tried again at the next refresh.
                    Err(e) => {
                        ai_failures += 1;
                        first_ai_error.get_or_insert_with(|| format!("{:#}", e));
                        crate::ai_classifier::unclassified()
                    }
                };
                let (from_name, from_email) = parse_from(&raw.from);
                let date_str =
                    raw.date.map(format_date).unwrap_or_else(|| i18n::unknown_date(i18n::lang()).to_string());
                let date_full = raw.date.map(format_date_full).unwrap_or_default();
                let body_html = raw.body_html.clone().unwrap_or_default();
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
                    references: raw.references,
                    date_str,
                    date_full,
                    date_ts: raw.date.map(|d| d.timestamp()).unwrap_or(0),
                    summary: classified.summary,
                    priority: classified.priority,
                    is_read: raw.seen,
                    body_text: render_body(&raw.body_text, &raw.body_html),
                    body_html,
                    attachments: raw.attachments,
                    html_only: raw.html_only,
                    project: classified.project,
                    deadline: classified.deadline,
                    task: classified.task,
                    meeting_link: classified.meeting_link,
                }
            })
            .collect();
        crate::cloud::store_ai(&to_store).await;
        cards
    };

    // No notification on the very first fill (empty cache): every email would look new.
    // Emails sorted again after an AI failure are not new either.
    if cfg.user.notify_new_mail && !seen_uids.is_empty() {
        let arrived: Vec<EmailCard> = new_cards.iter().filter(|c| !seen_uids.contains(&c.uid)).cloned().collect();
        notify_new_mail(app, &arrived);
    }

    // Merge: cached + newly classified. Remove UIDs no longer in inbox.
    let all_uids: std::collections::HashSet<u64> =
        cached_uids.iter().chain(new_cards.iter().map(|c| &c.uid)).copied().collect();
    // Keep only emails still present on server (by UID range we fetched).
    // Simple approach: keep cached entries whose UID appears in known_raw, plus new.
    let known_uid_set: std::collections::HashSet<u64> = known_raw.iter().map(|r| r.uid).collect();
    cached_cards.retain(|c| known_uid_set.contains(&c.uid));
    let mut cards = cached_cards;
    cards.extend(new_cards);
    let _ = all_uids; // suppress warning

    let sent_cards: Vec<EmailCard> = sent_raw.into_iter().map(raw_to_card).collect();
    let drafts_cards: Vec<EmailCard> = drafts_raw.into_iter().map(raw_to_card).collect();
    let trash_cards: Vec<EmailCard> = trash_raw.into_iter().map(raw_to_card).collect();
    let spam_cards: Vec<EmailCard> = spam_raw.into_iter().map(raw_to_card).collect();

    // Accounts changed while the AI was sorting: these ids are stale. Keep nothing; the
    // refresh runs again with the new settings.
    if accounts_changed(&state.lock().unwrap().cfg, cfg) {
        return Ok(());
    }
    save_cache(&cards);

    if ai_ready.is_ok() {
        if let Err(e) = check_open_tasks(cfg, &cards, &inbox_dates, recent_inbox, recent_sent).await {
            warn!("Task check failed: {:#}", e);
        }
    }

    let urgent_count = cards.iter().filter(|c| c.priority == Priority::Urgent).count();
    let high_count = cards.iter().filter(|c| c.priority == Priority::HighPriority).count();
    let attention = urgent_count + high_count;
    let unsorted = cards.iter().filter(|c| !is_sorted(c)).count();
    use chrono::Timelike;
    let now = Local::now();
    let live_briefing = ai_notice.unwrap_or_else(|| {
        refresh_briefing(&ai_ready, ai_failures, first_ai_error, unsorted, attention, now.hour(), now.minute())
    });

    if accounts_changed(&state.lock().unwrap().cfg, cfg) {
        return Ok(()); // accounts changed during the task check
    }
    populate(cfg, app, state, cards, sent_cards, drafts_cards, trash_cards, spam_cards, Some(live_briefing));
    Ok(())
}

/// Whether the AI has sorted this email (see `ai_classifier::is_real_summary`).
pub fn is_sorted(card: &EmailCard) -> bool {
    crate::ai_classifier::is_real_summary(&card.summary)
}

/// The briefing at the end of a refresh. The AI's state comes first: if it is not set up
/// or it failed, the user must know, otherwise nothing ever shows as important.
pub fn refresh_briefing(
    ai_ready: &Result<()>,
    ai_failures: usize,
    first_ai_error: Option<String>,
    unsorted: usize,
    attention: usize,
    hour: u32,
    minute: u32,
) -> Briefing {
    match ai_ready {
        Err(e) => Briefing { kind: "ai_missing", count: unsorted, detail: format!("{:#}", e), ..Default::default() },
        Ok(()) if ai_failures > 0 => Briefing {
            kind: "ai_error",
            count: ai_failures,
            detail: first_ai_error.unwrap_or_default(),
            ..Default::default()
        },
        Ok(()) => Briefing { kind: "live", hour, minute, count: attention, detail: String::new() },
    }
}

/// Redraw every list from the emails in memory, after a change that needs no fetch
/// (emails filed into a new project…). The briefing line stays as it was.
pub fn redisplay(cfg: &Config, app: &AppHandle, state: &Arc<Mutex<AppState>>) {
    use crate::search::Folder;
    let (index, briefing) = {
        let s = state.lock().unwrap();
        (s.search_index.clone(), s.briefing.clone())
    };
    let folder = |f: Folder| index.iter().filter(|(g, _)| *g == f).map(|(_, c)| c.clone()).collect::<Vec<_>>();
    populate(
        cfg,
        app,
        state,
        folder(Folder::Inbox),
        folder(Folder::Sent),
        folder(Folder::Drafts),
        folder(Folder::Trash),
        folder(Folder::Spam),
        briefing,
    );
}

pub fn load_and_display_cache(cfg: &Config, app: &AppHandle, state: &Arc<Mutex<AppState>>) {
    let json = match std::fs::read_to_string(crate::paths::file(CACHE_PATH)) {
        Ok(s) => s,
        Err(_) => return,
    };
    let cards: Vec<EmailCard> = match serde_json::from_str(&json) {
        Ok(v) => v,
        Err(e) => {
            warn!("Cache parse error: {}", e);
            return;
        }
    };
    info!("Loaded {} emails from cache", cards.len());
    populate(cfg, app, state, cards, vec![], vec![], vec![], vec![], None);
}

fn populate(
    cfg: &Config,
    app: &AppHandle,
    state: &Arc<Mutex<AppState>>,
    cards: Vec<EmailCard>,
    sent: Vec<EmailCard>,
    drafts: Vec<EmailCard>,
    trash: Vec<EmailCard>,
    spam: Vec<EmailCard>,
    briefing_override: Option<Briefing>,
) {
    // Projects the user re-tagged by hand win over the AI's choice.
    let overrides = crate::overrides::Overrides::load();
    let mut cards: Vec<EmailCard> = cards
        .into_iter()
        .map(|mut c| {
            overrides.apply(&mut c);
            c
        })
        .collect();
    // Cached messages and newly classified messages are assembled separately.
    // Always restore chronological order before building inbox-derived lists.
    cards.sort_by(|a, b| b.date_ts.cmp(&a.date_ts).then_with(|| b.uid.cmp(&a.uid)));

    let mut urgent = Vec::new();
    let mut high = Vec::new();
    let mut other = Vec::new();
    let mut info_vec = Vec::new();
    let mut threat = Vec::new();
    let mut task_vec = Vec::new();

    for card in &cards {
        match card.priority {
            Priority::Urgent => urgent.push(card.clone()),
            Priority::HighPriority => high.push(card.clone()),
            Priority::Threat => threat.push(card.clone()),
            Priority::Task => task_vec.push(card.clone()),
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
    let mut all_by_priority = all_emails.clone();
    all_by_priority.sort_by_key(priority_order);

    let dismissed = Dismissals::load();
    // New email action items land in the Todo column of the task board (and email
    // tasks from before dates were kept get the date of their email).
    let needs_sync = Board::load().needs_sync(&cards);
    if needs_sync {
        Board::update(|b| b.sync_from_cards(&cards, &dismissed));
    }
    let board_tasks = Board::load().tasks;
    let important = important_unread(&urgent, &high);
    let meeting_items: Vec<EmailCard> =
        cards.iter().filter(|c| c.meeting_link.is_some() && !dismissed.meetings.contains(&c.uid)).cloned().collect();

    let today = Local::now().date_naive();
    let mut deadlines: Vec<(NaiveDate, EmailCard)> = cards
        .iter()
        .filter(|c| !dismissed.deadlines.contains(&c.uid))
        .filter_map(|c| c.deadline.map(|d| (d, c.clone())))
        .collect();
    deadlines.sort_by_key(|(d, _)| *d);

    let week_days = build_week_days(today, &deadlines);

    let deadline_dtos: Vec<DeadlineDto> = deadlines
        .iter()
        .map(|(date, card)| DeadlineDto {
            uid: card.uid,
            subject: card.subject.clone(),
            deadline_str: i18n::day_month(date, i18n::lang()),
            date_iso: date.format("%Y-%m-%d").to_string(),
            project: card.project.clone().unwrap_or_default(),
            days_until: (*date - today).num_days() as i32,
        })
        .collect();

    let attention = urgent.len() + high.len();
    let briefing = briefing_override.unwrap_or(Briefing { kind: "cached", count: attention, ..Default::default() });
    // English/French text kept as a fallback for older front ends.
    let l = i18n::lang();
    let briefing_text = match briefing.kind {
        "live" => i18n::briefing_live(briefing.hour, briefing.minute, briefing.count, l),
        "classifying" => i18n::classifying(briefing.count, l),
        "up_to_date" => i18n::up_to_date(l).to_string(),
        "ai_missing" | "ai_error" => briefing.detail.clone(),
        _ => i18n::briefing_cached(briefing.count, l),
    };

    use chrono::Datelike;
    let now = Local::now();
    let cal_year = now.year();
    let cal_month = now.month();

    let project_dtos: Vec<ProjectDto> = cfg
        .projects
        .iter()
        .map(|p| {
            let email_count = all_emails.iter().filter(|c| c.project.as_deref() == Some(&p.name)).count() as i32;
            ProjectDto {
                name: p.name.clone(),
                urgency: p.urgency.clone(),
                description: p.description.clone(),
                email_count,
                warning: None,
            }
        })
        .collect();

    // Store all emails in shared state for command lookups.
    {
        let mut s = state.lock().unwrap();
        let mut all: Vec<EmailCard> = all_emails.clone();
        all.extend(sent.clone());
        all.extend(drafts.clone());
        all.extend(trash.clone());
        all.extend(spam.clone());
        s.emails = all;
        use crate::search::Folder;
        let tag = |f: Folder, v: &[EmailCard]| v.iter().cloned().map(move |c| (f, c)).collect::<Vec<_>>();
        let mut index = tag(Folder::Inbox, &all_emails);
        index.extend(tag(Folder::Sent, &sent));
        index.extend(tag(Folder::Drafts, &drafts));
        index.extend(tag(Folder::Trash, &trash));
        index.extend(tag(Folder::Spam, &spam));
        s.search_index = index;
        s.briefing = Some(briefing.clone());
    }

    use chrono::Timelike;
    let hour = Local::now().hour();
    let greeting = i18n::greeting(hour, &cfg.user_name(), i18n::lang());

    let snapshot = AppStateDto {
        greeting,
        briefing_time: briefing_text,
        user_name: cfg.user_name(),
        hour,
        briefing,
        loading: false,
        urgent_emails: to_dto_vec(&urgent),
        high_emails: to_dto_vec(&high),
        other_emails: to_dto_vec(&other),
        info_emails: to_dto_vec(&info_vec),
        threat_emails: to_dto_vec(&threat),
        task_emails: to_dto_vec(&task_vec),
        all_emails: to_dto_vec(&all_emails),
        tasks: board_tasks,
        meeting_items: to_dto_vec(&meeting_items),
        important_emails: to_dto_vec(&important),
        all_emails_by_priority: to_dto_vec(&all_by_priority),
        sent_emails: to_dto_vec(&sent),
        drafts_emails: to_dto_vec(&drafts),
        trash_emails: to_dto_vec(&trash),
        spam_emails: to_dto_vec(&spam),
        deadlines: deadline_dtos,
        week_days,
        projects: project_dtos,
        calendar_year: cal_year,
        calendar_month: cal_month,
    };

    app.emit("state-updated", snapshot).ok();
}

pub fn to_dto_vec(cards: &[EmailCard]) -> Vec<EmailCardDto> {
    cards
        .iter()
        .map(|c| EmailCardDto {
            uid: c.uid,
            subject: c.subject.clone(),
            from_name: c.from_name.clone(),
            from_email: c.from_email.clone(),
            to: c.to.clone(),
            cc: c.cc.clone(),
            reply_to: c.reply_to.clone(),
            bcc: c.bcc.clone(),
            message_id: c.message_id.clone(),
            references: c.references.clone(),
            date_str: c.date_str.clone(),
            date_full: c.date_full.clone(),
            date_ts: c.date_ts,
            summary: c.summary.clone(),
            priority: c.priority.as_str().to_string(),
            is_read: c.is_read,
            project: c.project.clone().unwrap_or_default(),
            deadline_str: c.deadline.map(|d| i18n::day_month(&d, i18n::lang())).unwrap_or_default(),
            task: c.task.clone().unwrap_or_default(),
            body_text: c.body_text.clone(),
            body_html: c.body_html.clone(),
            attachments: c.attachments.clone(),
            html_only: c.html_only,
            meeting_link: c.meeting_link.clone().unwrap_or_default(),
        })
        .collect()
}

/// The Overview strip: today and the 6 following days.
pub fn build_week_days(today: NaiveDate, deadlines: &[(NaiveDate, EmailCard)]) -> Vec<WeekDayDto> {
    use chrono::Datelike;
    (0..7)
        .map(|offset| {
            let day = today + chrono::Duration::days(offset);
            let slots: Vec<&EmailCard> = deadlines.iter().filter(|(d, _)| *d == day).map(|(_, c)| c).take(3).collect();
            let get = |i: usize| slots.get(i).map(|c| (c.subject.clone(), c.uid)).unwrap_or_else(|| (String::new(), 0));
            let (s1, u1) = get(0);
            let (s2, u2) = get(1);
            let (s3, u3) = get(2);
            WeekDayDto {
                day_name: i18n::short_day(&day, i18n::lang()),
                date_iso: day.format("%Y-%m-%d").to_string(),
                day_number: day.day(),
                is_today: day == today,
                dl1_subject: s1,
                dl1_uid: u1,
                dl2_subject: s2,
                dl2_uid: u2,
                dl3_subject: s3,
                dl3_uid: u3,
            }
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
    use chrono::{Local, Timelike};
    let local = dt.with_timezone(&Local);
    let now = Local::now();
    let diff = now.signed_duration_since(local);
    if diff.num_hours() < 24 {
        format!("{:02}:{:02}", local.hour(), local.minute())
    } else if diff.num_days() < 7 {
        i18n::short_day(&local, i18n::lang())
    } else {
        i18n::day_month(&local, i18n::lang())
    }
}

fn format_date_full(dt: chrono::DateTime<Utc>) -> String {
    i18n::full_datetime(&dt.with_timezone(&Local), i18n::lang())
}

fn render_body(plain: &str, html: &Option<String>) -> String {
    if let Some(h) = html {
        if !h.trim().is_empty() {
            return html2text::from_read(h.as_bytes(), 100);
        }
    }
    plain.to_string()
}

/// Older versions numbered every account's emails by bare IMAP UID, like the first account's.
/// Cached ids (below 2^32) that are, by Message-ID, another account's email at the same UID,
/// mapped to that email's id (`models::mail_id`).
pub fn moved_ids(cached: &[(u64, &str)], server: &[(u64, &str)]) -> std::collections::HashMap<u64, u64> {
    let others: std::collections::HashMap<(u64, &str), u64> = server
        .iter()
        .filter(|(id, mid)| id >> 32 > 0 && !mid.is_empty())
        .map(|&(id, mid)| ((id & 0xFFFF_FFFF, mid), id))
        .collect();
    cached
        .iter()
        .filter(|(id, _)| id >> 32 == 0)
        .filter_map(|&(id, mid)| others.get(&(id, mid)).map(|&new| (id, new)))
        .collect()
}

/// Give older versions' cached emails of the second (third…) account their new ids, with their
/// tasks, dismissals and project choices, so they are neither sorted again nor announced as new.
fn migrate_account_ids(cached: &mut [EmailCard], raw: &[RawEmail]) {
    let moved = {
        let cached_ids: Vec<(u64, &str)> = cached.iter().map(|c| (c.uid, c.message_id.as_str())).collect();
        let server_ids: Vec<(u64, &str)> = raw.iter().map(|r| (r.uid, r.message_id.as_str())).collect();
        moved_ids(&cached_ids, &server_ids)
    };
    if moved.is_empty() {
        return;
    }
    info!("Moving {} cached emails to their account's ids", moved.len());
    let id = |u: u64| *moved.get(&u).unwrap_or(&u);
    for c in cached.iter_mut() {
        c.uid = id(c.uid);
    }
    save_cache(cached);
    Board::update(|b| {
        for t in &mut b.tasks {
            t.source_uid = t.source_uid.map(id);
        }
        b.deleted_sources = std::mem::take(&mut b.deleted_sources).into_iter().map(id).collect();
    });
    Dismissals::update(|d| {
        for set in [&mut d.tasks, &mut d.deadlines, &mut d.meetings] {
            *set = std::mem::take(set).into_iter().map(id).collect();
        }
        d.task_check = std::mem::take(&mut d.task_check).into_iter().map(|(u, v)| (id(u), v)).collect();
    });
    crate::overrides::Overrides::update(|o| {
        o.projects = std::mem::take(&mut o.projects).into_iter().map(|(u, v)| (id(u), v)).collect();
    });
}

/// Whether the accounts (server, login, mailbox, in order) differ: a refresh begun with the
/// old ones has ids that no longer match (`models::mail_id`).
fn accounts_changed(now: &Config, began: &Config) -> bool {
    let key = |c: &Config| {
        c.accounts
            .iter()
            .map(|a| (a.imap.host.to_lowercase(), a.imap.username.to_lowercase(), a.imap.mailbox.clone()))
            .collect::<Vec<_>>()
    };
    key(now) != key(began)
}

/// An email id once accounts were removed or reordered: `new_index[a]` is the new position of
/// the account at position `a`, None when it was removed (then the email goes too).
pub fn remap_id(new_index: &[Option<usize>], id: u64) -> Option<u64> {
    let (account, uid) = crate::models::split_mail_id(id);
    new_index.get(account).copied().flatten().map(|n| crate::models::mail_id(n, uid))
}

/// After accounts were removed or reordered in Settings: forget what came from a removed
/// account (its emails, the tasks taken from them, dismissals, project choices) and give the
/// other accounts' emails their new ids. Hand-made tasks stay.
pub fn forget_accounts(new_index: &[Option<usize>], cfg: &Config, app: &AppHandle, state: &Arc<Mutex<AppState>>) {
    let id = |u: u64| remap_id(new_index, u);
    let cache: Vec<EmailCard> = load_cache()
        .into_iter()
        .filter_map(|mut c| {
            c.uid = id(c.uid)?;
            Some(c)
        })
        .collect();
    save_cache(&cache);
    Board::update(|b| {
        b.tasks.retain_mut(|t| match t.source_uid {
            Some(u) => id(u).map(|n| t.source_uid = Some(n)).is_some(),
            None => true,
        });
        b.deleted_sources = std::mem::take(&mut b.deleted_sources).into_iter().filter_map(id).collect();
    });
    Dismissals::update(|d| {
        for set in [&mut d.tasks, &mut d.deadlines, &mut d.meetings] {
            *set = std::mem::take(set).into_iter().filter_map(id).collect();
        }
        d.task_check = std::mem::take(&mut d.task_check).into_iter().filter_map(|(u, v)| Some((id(u)?, v))).collect();
    });
    crate::overrides::Overrides::update(|o| {
        o.projects = std::mem::take(&mut o.projects).into_iter().filter_map(|(u, v)| Some((id(u)?, v))).collect();
    });
    {
        let mut s = state.lock().unwrap();
        s.emails = std::mem::take(&mut s.emails)
            .into_iter()
            .filter_map(|mut c| {
                c.uid = id(c.uid)?;
                Some(c)
            })
            .collect();
        s.search_index = std::mem::take(&mut s.search_index)
            .into_iter()
            .filter_map(|(f, mut c)| {
                c.uid = id(c.uid)?;
                Some((f, c))
            })
            .collect();
        let mut kept: Vec<_> = std::mem::take(&mut s.mailboxes)
            .into_iter()
            .enumerate()
            .filter_map(|(a, m)| Some((new_index.get(a).copied().flatten()?, m)))
            .collect();
        kept.sort_by_key(|(n, _)| *n);
        s.mailboxes = kept.into_iter().map(|(_, m)| m).collect();
        let mut sent: Vec<_> = std::mem::take(&mut s.sent_folders)
            .into_iter()
            .enumerate()
            .filter_map(|(a, f)| Some((new_index.get(a).copied().flatten()?, f)))
            .collect();
        sent.sort_by_key(|(n, _)| *n);
        s.sent_folders = sent.into_iter().map(|(_, f)| f).collect();
    }
    redisplay(cfg, app, state);
}

fn load_cache() -> Vec<EmailCard> {
    match std::fs::read_to_string(crate::paths::file(CACHE_PATH)) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => vec![],
    }
}

fn save_cache(cards: &[EmailCard]) {
    match serde_json::to_string(cards) {
        Ok(json) => {
            std::fs::write(crate::paths::file(CACHE_PATH), json).ok();
        }
        Err(e) => warn!("Cache serialize error: {}", e),
    }
}

/// The special folders found on the server. `spam` is None when the mailbox has none
/// (it is then created the first time an email is marked as spam).
#[derive(Debug, Clone, PartialEq)]
pub struct SpecialFolders {
    pub sent: String,
    pub drafts: String,
    pub trash: String,
    pub spam: Option<String>,
    /// Where "Archive" moves emails; None: created on first use.
    pub archive: Option<String>,
}

/// Pick the special folders from a LIST result: the role the server declares
/// (RFC 6154 \Sent, \Drafts, \Trash, \Junk) first, then well-known names. Names are
/// as the server sends them, so accented ones are in modified UTF-7 ("&AOk-" = "é").
pub fn choose_folders(list: &[(String, Vec<&'static str>)]) -> SpecialFolders {
    let by_role = |role: &str| list.iter().find(|(_, roles)| roles.contains(&role)).map(|(n, _)| n.clone());
    let by_name = |candidates: &[&str]| -> Option<String> {
        for c in candidates {
            if let Some((n, _)) = list.iter().find(|(n, _)| n.eq_ignore_ascii_case(c)) {
                return Some(n.clone());
            }
        }
        // Also as the last path segment: "INBOX.Junk", "[Gmail]/Spam"…
        for c in candidates {
            if let Some((n, _)) =
                list.iter().find(|(n, _)| n.rsplit(['/', '.']).next().is_some_and(|last| last.eq_ignore_ascii_case(c)))
            {
                return Some(n.clone());
            }
        }
        None
    };
    SpecialFolders {
        sent: by_role("sent")
            .or_else(|| by_name(&["Sent", "Sent Items", "Sent Messages", "Messages envoy&AOk-s", "Envoy&AOk-s"]))
            .unwrap_or_else(|| "Sent".into()),
        drafts: by_role("drafts")
            .or_else(|| by_name(&["Drafts", "Draft", "Brouillons"]))
            .unwrap_or_else(|| "Drafts".into()),
        trash: by_role("trash")
            .or_else(|| by_name(&["Trash", "Deleted Items", "Deleted Messages", "Corbeille"]))
            .unwrap_or_else(|| "Trash".into()),
        spam: by_role("junk").or_else(|| {
            by_name(&[
                "Junk",
                "Spam",
                "Junk E-mail",
                "Junk Email",
                "Bulk Mail",
                "Courrier ind&AOk-sirable",
                "Ind&AOk-sirables",
                "Pourriel",
                "Pourriels",
            ])
        }),
        archive: by_role("archive").or_else(|| by_name(&["Archive", "Archives", "Archived"])),
    }
}

async fn guess_folder_names(cfg: &ImapConfig) -> SpecialFolders {
    use async_imap::types::NameAttribute;
    use futures::StreamExt;

    let fallback = SpecialFolders {
        sent: "Sent".into(),
        drafts: "Drafts".into(),
        trash: "Trash".into(),
        spam: None,
        archive: None,
    };
    let mut session = match connect(cfg).await {
        Ok(s) => s,
        Err(_) => return fallback,
    };
    let mut list: Vec<(String, Vec<&'static str>)> = Vec::new();
    if let Ok(stream) = session.list(Some(""), Some("*")).await {
        futures::pin_mut!(stream);
        while let Some(item) = stream.next().await {
            if let Ok(n) = item {
                let roles = n
                    .attributes()
                    .iter()
                    .filter_map(|a| match a {
                        NameAttribute::Sent => Some("sent"),
                        NameAttribute::Drafts => Some("drafts"),
                        NameAttribute::Trash => Some("trash"),
                        NameAttribute::Junk => Some("junk"),
                        NameAttribute::Archive => Some("archive"),
                        _ => None,
                    })
                    .collect();
                list.push((n.name().to_string(), roles));
            }
        }
    }
    session.logout().await.ok();
    let folders = choose_folders(&list);
    info!("Folders: sent={} drafts={} trash={} spam={:?}", folders.sent, folders.drafts, folders.trash, folders.spam);
    folders
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
    inbox_dates: &std::collections::HashMap<u64, chrono::DateTime<Utc>>,
    recent_inbox: Vec<(chrono::DateTime<Utc>, RecentMail)>,
    recent_sent: Vec<(chrono::DateTime<Utc>, RecentMail)>,
) -> Result<()> {
    // Make sure new email tasks are on the board before checking them.
    let dismissed = Dismissals::load();
    let board = Board::update(|b| {
        b.sync_from_cards(cards, &dismissed);
        b.tasks.clone()
    });
    let open: Vec<OpenTask> = board
        .iter()
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

    let open_uids: std::collections::HashSet<u64> = open.iter().map(|t| t.uid).collect();
    let oldest = open.iter().filter_map(|t| inbox_dates.get(&t.uid)).min().copied();
    let sent_cutoff = Utc::now() - chrono::Duration::days(14);

    let mut recent: Vec<(chrono::DateTime<Utc>, RecentMail)> =
        recent_sent.into_iter().filter(|(d, _)| *d >= sent_cutoff).collect();
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
    let verdicts = crate::ai_classifier::check_tasks(&cfg.ai, &open, &recent).await?;
    if verdicts.is_empty() {
        return Ok(());
    }
    Board::update(|b| {
        for (uid, verdict) in verdicts {
            if !open_uids.contains(&uid) {
                continue;
            }
            // Only move tasks still open: the user may have moved one meanwhile.
            if let Some(t) = b.tasks.iter_mut().find(|t| t.source_uid == Some(uid) && t.status.is_open()) {
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
pub fn set_read_in_cache(uid: u64, read: bool) {
    let mut cards = load_cache();
    if let Some(c) = cards.iter_mut().find(|c| c.uid == uid) {
        c.is_read = read;
        save_cache(&cards);
    }
}

/// Drop one email from the on-disk cache (after it was moved to the trash).
pub fn remove_from_cache(uid: u64) {
    let mut cards = load_cache();
    cards.retain(|c| c.uid != uid);
    save_cache(&cards);
}
