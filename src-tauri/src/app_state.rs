use anyhow::Result;
use chrono::{Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tauri::AppHandle;
use tauri::Emitter;
use tracing::{error, info, warn};

use crate::ai_classifier::{ClassificationOutcome, OpenTask, RecentMail, TaskUpdate};
use crate::config::{Config, ImapConfig};
use crate::dismissals::Dismissals;
use crate::dto::*;
use crate::i18n;
use crate::imap_client::{
    connect, fetch_all_contact_headers, fetch_all_signature_messages, fetch_folder, fetch_inbox_delta,
};
use crate::models::{EmailCard, Priority, RawEmail};
use crate::state::AppState;
use crate::tasks::{Board, Status};

// v2: keyed by real IMAP UIDs (v1 used sequence numbers, which shift on delete).
const CACHE_PATH: &str = "cache-v2.json";
const SYNC_STATE_PATH: &str = "sync-state-v1.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefreshMode {
    Light,
    Full,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct FolderCursor {
    #[serde(default)]
    initialized: bool,
    uid_validity: Option<u32>,
    highest_uid: u32,
    #[serde(default)]
    folders: Option<SpecialFolders>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SyncState {
    #[serde(default)]
    accounts: HashMap<String, FolderCursor>,
    #[serde(default)]
    last_check: i64,
    #[serde(default)]
    last_full_refresh: i64,
    #[serde(default)]
    last_new_count: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RefreshStatus {
    pub last_check: i64,
    pub last_full_refresh: i64,
    pub last_new_count: usize,
}

fn account_sync_key(imap: &ImapConfig) -> String {
    format!("{}\n{}\n{}", imap.host.trim().to_lowercase(), imap.username.trim().to_lowercase(), imap.mailbox.trim())
}

fn load_sync_state() -> SyncState {
    std::fs::read_to_string(crate::paths::file(SYNC_STATE_PATH))
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn save_sync_state(sync: &SyncState) -> Result<()> {
    let json = serde_json::to_vec(sync)?;
    let path = crate::paths::file(SYNC_STATE_PATH);
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, json)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

pub fn refresh_status() -> RefreshStatus {
    let sync = load_sync_state();
    RefreshStatus {
        last_check: sync.last_check,
        last_full_refresh: sync.last_full_refresh,
        last_new_count: sync.last_new_count,
    }
}

/// Fetch one secondary mailbox on first use during this app session. Routine
/// refreshes maintain Inbox only; opening Sent, Drafts, Trash, or Spam pays for
/// exactly that folder and then reuses the in-memory result.
pub async fn load_secondary_folder(
    cfg: Arc<Config>,
    app: AppHandle,
    state: Arc<Mutex<AppState>>,
    folder: crate::search::Folder,
) -> Result<bool> {
    use crate::search::Folder;
    static FOLDER_LOAD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    if matches!(folder, Folder::Inbox | Folder::Server) {
        return Ok(false);
    }
    let _load_guard = FOLDER_LOAD.lock().await;
    if state.lock().unwrap().loaded_folders.contains(&folder) {
        return Ok(false);
    }

    let mut sync = load_sync_state();
    let remembered = state.lock().unwrap().mailboxes.clone();
    let mut mailboxes = Vec::with_capacity(cfg.accounts.len());
    for (index, account) in cfg.accounts.iter().enumerate() {
        let key = account_sync_key(&account.imap);
        let known = remembered
            .get(index)
            .filter(|(saved, _)| account_sync_key(&saved.imap) == key)
            .map(|(_, folders)| folders.clone())
            .or_else(|| sync.accounts.get(&key).and_then(|cursor| cursor.folders.clone()));
        let folders = match known {
            Some(folders) => folders,
            None => guess_folder_names(&account.imap).await,
        };
        sync.accounts.entry(key).or_default().folders = Some(folders.clone());
        mailboxes.push((account.clone(), folders));
    }

    let jobs = mailboxes.iter().enumerate().map(|(index, (account, folders))| async move {
        let mailbox = match folder {
            Folder::Sent => Some(folders.sent.as_str()),
            Folder::Drafts => Some(folders.drafts.as_str()),
            Folder::Trash => Some(folders.trash.as_str()),
            Folder::Spam => folders.spam.as_deref(),
            Folder::Inbox | Folder::Server => None,
        };
        let Some(mailbox) = mailbox else { return Ok(Vec::new()) };
        let mut emails = fetch_folder(&account.imap, mailbox).await?;
        for email in &mut emails {
            email.uid = crate::models::mail_id(index, email.uid as u32);
        }
        Ok::<_, anyhow::Error>(emails)
    });
    let mut raw = Vec::new();
    for result in futures::future::join_all(jobs).await {
        raw.extend(result?);
    }
    if accounts_changed(&state.lock().unwrap().cfg, &cfg) {
        return Ok(false);
    }

    let loaded: Vec<EmailCard> = raw.into_iter().map(raw_to_card).collect();
    let (inbox, mut sent, mut drafts, mut trash, mut spam, briefing) = {
        let s = state.lock().unwrap();
        let cards = |wanted| {
            s.search_index
                .iter()
                .filter(|(current, _)| *current == wanted)
                .map(|(_, card)| card.clone())
                .collect::<Vec<_>>()
        };
        (
            cards(Folder::Inbox),
            cards(Folder::Sent),
            cards(Folder::Drafts),
            cards(Folder::Trash),
            cards(Folder::Spam),
            s.briefing.clone(),
        )
    };
    match folder {
        Folder::Sent => sent = loaded,
        Folder::Drafts => drafts = loaded,
        Folder::Trash => trash = loaded,
        Folder::Spam => spam = loaded,
        Folder::Inbox | Folder::Server => {}
    }
    save_sync_state(&sync)?;
    {
        let mut s = state.lock().unwrap();
        s.mailboxes = mailboxes;
        s.loaded_folders.insert(folder);
    }
    populate(&cfg, &app, &state, inbox, sent, drafts, trash, spam, briefing);
    Ok(true)
}

pub async fn refresh(
    cfg: Arc<Config>,
    app: AppHandle,
    state: Arc<Mutex<AppState>>,
) -> std::result::Result<bool, String> {
    run_refresh(cfg, app, state, RefreshMode::Light).await
}

pub async fn full_refresh(
    cfg: Arc<Config>,
    app: AppHandle,
    state: Arc<Mutex<AppState>>,
) -> std::result::Result<bool, String> {
    run_refresh(cfg, app, state, RefreshMode::Full).await
}

async fn run_refresh(
    _cfg: Arc<Config>,
    app: AppHandle,
    state: Arc<Mutex<AppState>>,
    requested: RefreshMode,
) -> std::result::Result<bool, String> {
    // Serialize refreshes without dropping requests. In particular, a Settings
    // full refresh waits for any running automatic check and returns only after
    // its own reconciliation is complete.
    static REFRESH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _refresh_guard = REFRESH_LOCK.lock().await;
    let cfg = state.lock().unwrap().cfg.clone();
    app.emit("loading", true).ok();
    let result = do_refresh(&cfg, &app, &state, requested).await.map_err(|error| format!("{error:#}"));
    match &result {
        Ok(changed) => info!(
            "{} refresh complete{}",
            if requested == RefreshMode::Full { "Full" } else { "Light" },
            if *changed { "" } else { " (no changes)" }
        ),
        Err(refresh_error) => {
            error!("Refresh failed: {}", refresh_error);
            app.emit("error", format!("Refresh failed: {refresh_error}")).ok();
        }
    }
    // Hide the loading bar whether the refresh succeeded or failed.
    app.emit("loading", false).ok();
    result
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
        project: None,
        deadline: None,
        task: None,
        meeting_link: None,
    }
}

async fn do_refresh(cfg: &Config, app: &AppHandle, state: &Arc<Mutex<AppState>>, mode: RefreshMode) -> Result<bool> {
    let mut raw_emails = Vec::new();
    let mut sent_raw = Vec::new();
    let mut drafts_raw = Vec::new();
    let mut trash_raw = Vec::new();
    let mut spam_raw = Vec::new();
    let mut secondary_ok = [true; 4]; // Sent, Drafts, Trash, Spam
    let mut inbox_ok = false;
    let mut mailboxes = Vec::new();
    let mut reconciled_accounts = HashSet::new();
    let mut repair_only = mode == RefreshMode::Full;
    let mut sync = load_sync_state();
    let cache_available = state.lock().unwrap().cache_loaded;
    let missing_cache = mode == RefreshMode::Light && !cache_available;
    // Kept lazy so the steady-state no-change path does not read and parse the
    // potentially large body cache at all. It is needed once when upgrading an
    // installation without cursors, and after a real change.
    let mut cached_cards: Option<Vec<EmailCard>> = None;
    let own_addresses: std::collections::HashSet<String> = cfg
        .accounts
        .iter()
        .flat_map(|account| {
            [account.imap.username.as_str(), account.smtp.username.as_str(), account.smtp.from_address.as_str()]
                .into_iter()
                .flat_map(crate::search::split_addresses)
        })
        .filter_map(|entry| {
            let address = entry
                .rsplit_once('<')
                .map(|(_, tail)| tail.trim_end_matches('>'))
                .unwrap_or(&entry)
                .trim()
                .to_lowercase();
            address.contains('@').then_some(address)
        })
        .collect();

    for (index, account) in cfg.accounts.iter().enumerate() {
        let key = account_sync_key(&account.imap);
        let had_saved_cursor = sync.accounts.get(&key).is_some_and(|cursor| cursor.initialized);
        let saved = sync.accounts.get(&key).cloned().unwrap_or_default();
        let remembered_folders = state
            .lock()
            .unwrap()
            .mailboxes
            .get(index)
            .and_then(|(remembered, folders)| (account_sync_key(&remembered.imap) == key).then(|| folders.clone()));
        let folders = if mode == RefreshMode::Full {
            guess_folder_names(&account.imap).await
        } else if let Some(folders) = saved.folders.clone().or(remembered_folders) {
            folders
        } else {
            // One LIST on first use records provider-specific folder names. Future
            // light checks reuse them and perform only the Inbox SELECT.
            guess_folder_names(&account.imap).await
        };
        let (sent_name, drafts_name, trash_name) =
            (folders.sent.clone(), folders.drafts.clone(), folders.trash.clone());
        let spam_name = folders.spam.clone();
        {
            let mut s = state.lock().unwrap();
            s.sent_folders.resize(cfg.accounts.len(), "Sent".into());
            s.sent_folders[index] = sent_name.clone();
        }
        mailboxes.push((account.clone(), folders));
        let cached_highest = if mode == RefreshMode::Light && !had_saved_cursor {
            cached_cards
                .get_or_insert_with(load_cache)
                .iter()
                .filter_map(|card| {
                    let (card_account, uid) = crate::models::split_mail_id(card.uid);
                    (card_account == index).then_some(uid)
                })
                .max()
        } else {
            None
        };
        let saved_highest = if mode == RefreshMode::Full || missing_cache {
            None
        } else if had_saved_cursor {
            Some(saved.highest_uid)
        } else {
            // Upgrade path: an existing cache is already a valid baseline. The first
            // incremental SELECT records UIDVALIDITY without redownloading those bodies.
            cached_highest
        };
        let saved_validity = (mode == RefreshMode::Light).then_some(saved.uid_validity).flatten();
        let (inbox_res, sent_res, drafts_res, trash_res, spam_res) = if mode == RefreshMode::Full {
            tokio::join!(
                fetch_inbox_delta(&account.imap, None, None),
                fetch_folder(&account.imap, &sent_name),
                fetch_folder(&account.imap, &drafts_name),
                fetch_folder(&account.imap, &trash_name),
                async {
                    match &spam_name {
                        Some(name) => fetch_folder(&account.imap, name).await,
                        None => Ok(Vec::new()),
                    }
                },
            )
        } else {
            (
                fetch_inbox_delta(&account.imap, saved_validity, saved_highest).await,
                Ok(Vec::new()),
                Ok(Vec::new()),
                Ok(Vec::new()),
                Ok(Vec::new()),
            )
        };
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
            (inbox_res, tag(sent_res), tag(drafts_res), tag(trash_res), tag(spam_res));
        match inbox_res {
            Ok(mut delta) => {
                info!("Inbox[{}]: {} fetched email(s)", account.name, delta.emails.len());
                for raw in &mut delta.emails {
                    raw.uid = crate::models::mail_id(index, raw.uid as u32);
                }
                if mode == RefreshMode::Full || delta.reset || missing_cache {
                    reconciled_accounts.insert(index);
                }
                if delta.reset || (missing_cache && had_saved_cursor) {
                    repair_only = true;
                }
                raw_emails.append(&mut delta.emails);
                sync.accounts.insert(
                    key,
                    FolderCursor {
                        initialized: true,
                        uid_validity: delta.uid_validity,
                        highest_uid: delta.highest_uid,
                        folders: Some(mailboxes[index].1.clone()),
                    },
                );
                inbox_ok = true;
            }
            Err(e) => warn!("Inbox[{}]: {}", account.name, e),
        }
        match sent_res {
            Ok(mut v) => {
                sent_raw.append(&mut v);
            }
            Err(e) => {
                secondary_ok[0] = false;
                warn!("Sent[{}]: {}", account.name, e);
            }
        }
        match drafts_res {
            Ok(mut v) => {
                drafts_raw.append(&mut v);
            }
            Err(e) => {
                secondary_ok[1] = false;
                warn!("Drafts[{}]: {}", account.name, e);
            }
        }
        match trash_res {
            Ok(mut v) => {
                trash_raw.append(&mut v);
            }
            Err(e) => {
                secondary_ok[2] = false;
                warn!("Trash[{}]: {}", account.name, e);
            }
        }
        match spam_res {
            Ok(mut v) => {
                spam_raw.append(&mut v);
            }
            Err(e) => {
                secondary_ok[3] = false;
                warn!("Spam[{}]: {}", account.name, e);
            }
        }

        // Historical scans are repair/enrichment work, never part of the two-minute light check.
        if mode != RefreshMode::Full {
            continue;
        }
        // A first-run address-book scan reads only headers and covers the complete Sent history.
        // Incoming history is kept separate as untrusted potential contacts.
        let sent_scan = crate::contacts::scan_key("sent", &account.imap.host, &account.imap.username, &sent_name);
        if crate::contacts::scan_needed(&sent_scan) {
            match fetch_all_contact_headers(&account.imap, &sent_name).await {
                Ok(headers) => {
                    if let Err(error) = crate::contacts::complete_sent_scan(&sent_scan, &headers, &own_addresses) {
                        warn!("Could not save complete Sent contact scan: {}", error);
                    }
                }
                Err(error) => warn!("Sent contact scan[{}]: {:#}", account.name, error),
            }
        }
        let inbox_scan =
            crate::contacts::scan_key("potential", &account.imap.host, &account.imap.username, &account.imap.mailbox);
        if crate::contacts::scan_needed(&inbox_scan) {
            match fetch_all_contact_headers(&account.imap, &account.imap.mailbox).await {
                Ok(mut headers) => {
                    headers.retain(|email| !crate::spam_guard::server_marks_spam(&email.security));
                    if let Err(error) = crate::contacts::complete_potential_scan(&inbox_scan, &headers, &own_addresses)
                    {
                        warn!("Could not save potential-contact scan: {}", error);
                    }
                }
                Err(error) => warn!("Potential contact scan[{}]: {:#}", account.name, error),
            }
        }
        // Older installations already completed the header scan. This independently versioned
        // pass reads bounded message text once so signatures can enrich those cached contacts.
        let details_scan =
            crate::contacts::scan_key("details-v5", &account.imap.host, &account.imap.username, &account.imap.mailbox);
        if crate::contacts::scan_needed(&details_scan) {
            match fetch_all_signature_messages(&account.imap, &account.imap.mailbox).await {
                Ok(mut messages) => {
                    messages.retain(|email| !crate::spam_guard::server_marks_spam(&email.security));
                    if let Err(error) =
                        crate::contacts::complete_potential_scan(&details_scan, &messages, &own_addresses)
                    {
                        warn!("Could not save historical contact-detail scan: {}", error);
                    }
                }
                Err(error) => warn!("Contact detail scan[{}]: {:#}", account.name, error),
            }
        }
    }

    // If no inbox could be fetched, keep the cache untouched rather than wiping it.
    if !inbox_ok {
        anyhow::bail!("could not fetch any inbox (check IMAP settings)");
    }

    let new_fetch_count = raw_emails.len();
    sync.last_check = Utc::now().timestamp();
    if mode == RefreshMode::Light {
        sync.last_new_count = if repair_only { 0 } else { new_fetch_count };
    }
    if mode == RefreshMode::Full {
        sync.last_full_refresh = sync.last_check;
    }
    // The hot no-change path ends here: no MIME parsing happened, and there is no
    // cache rewrite, AI call, task check, DTO clone, or state-updated event.
    if mode == RefreshMode::Light && raw_emails.is_empty() && reconciled_accounts.is_empty() {
        save_sync_state(&sync)?;
        return Ok(false);
    }

    let mut cached_cards = cached_cards.unwrap_or_else(load_cache);
    // Secondary folders are session-only and loaded on demand. Clone them only
    // when Inbox actually changed and a new state snapshot must preserve them.
    let preserved_folders = if mode == RefreshMode::Light {
        use crate::search::Folder;
        let index = state.lock().unwrap().search_index.clone();
        let cards = |folder| index.iter().filter(|(f, _)| *f == folder).map(|(_, c)| c.clone()).collect::<Vec<_>>();
        Some((cards(Folder::Sent), cards(Folder::Drafts), cards(Folder::Trash), cards(Folder::Spam)))
    } else {
        None
    };

    // Load previously classified emails from cache — reuse results for known UIDs.
    // Emails the AI has not sorted yet (it was not set up, or it failed) are sorted again.
    migrate_account_ids(&mut cached_cards, &raw_emails);
    let seen_uids: std::collections::HashSet<u64> = cached_cards.iter().map(|c| c.uid).collect();
    let classifications_by_message_id: HashMap<String, EmailCard> = cached_cards
        .iter()
        .filter(|card| !card.message_id.is_empty())
        .map(|card| (card.message_id.clone(), card.clone()))
        .collect();
    // A cached email whose Message-ID differs is another email under the same id (older
    // versions gave two accounts' emails the same one): it is sorted again.
    let server_ids: std::collections::HashMap<u64, &str> =
        raw_emails.iter().map(|r| (r.uid, r.message_id.as_str())).collect();
    cached_cards.retain(|c| match server_ids.get(&c.uid) {
        Some(id) => id.is_empty() || c.message_id.is_empty() || *id == c.message_id,
        None => true,
    });
    // UIDVALIDITY can change every UID while Message-ID remains stable. Carry the
    // existing AI result to the fresh UID before deciding which messages are new.
    for raw in &raw_emails {
        if cached_cards.iter().any(|card| card.uid == raw.uid) || raw.message_id.is_empty() {
            continue;
        }
        if let Some(previous) = classifications_by_message_id.get(&raw.message_id) {
            let mut restored = previous.clone();
            restored.uid = raw.uid;
            cached_cards.push(restored);
        }
    }
    let cached_uids: std::collections::HashSet<u64> = cached_cards.iter().map(|c| c.uid).collect();
    let ai_ready = crate::ai::readiness(&cfg.ai);

    // Split inbox into already-known (reuse cache) and new (need AI).
    let (known_raw, new_raw): (Vec<_>, Vec<_>) = raw_emails.into_iter().partition(|r| cached_uids.contains(&r.uid));

    // Sent recipients become known contacts. Trust affects sender/domain burst checks and the
    // adaptive model only when the receiving server reports aligned DMARC; it never bypasses the
    // global emergency ceiling or a server spam verdict.
    crate::contacts::ingest_sent(&sent_raw, &own_addresses);
    let directory = crate::contacts::load();
    let trust = crate::contacts::trust_signals(&directory);
    let guard = crate::spam_guard::assess_batch(&cfg.accounts, &new_raw, &trust, chrono::Utc::now().timestamp());
    let mut adaptive_rejected = crate::adaptive_spam::marked_in_batch(&new_raw);
    adaptive_rejected.retain(|uid, _| {
        !matches!(guard.get(uid), Some(crate::spam_guard::Decision::Spam(_)))
            && new_raw
                .iter()
                .find(|email| email.uid == *uid)
                .is_none_or(|email| !crate::spam_guard::authenticated_trusted(email, &trust))
    });
    let mut rejected: std::collections::HashSet<u64> = guard
        .iter()
        .filter_map(|(uid, decision)| matches!(decision, crate::spam_guard::Decision::Spam(_)).then_some(*uid))
        .collect();
    rejected.extend(adaptive_rejected.keys().copied());
    let accepted_incoming: Vec<RawEmail> = new_raw
        .iter()
        .filter(|email| !rejected.contains(&email.uid))
        .cloned()
        .chain(known_raw.iter().filter(|email| !crate::spam_guard::server_marks_spam(&email.security)).cloned())
        .collect();
    crate::contacts::ingest_potential(&accepted_incoming, &own_addresses);
    let mut classify_raw = Vec::new();
    let mut guarded_raw = Vec::new();
    let mut ensured_spam = std::collections::HashSet::new();
    for raw in new_raw {
        let hard_reason = match guard.get(&raw.uid) {
            Some(crate::spam_guard::Decision::Spam(reason)) => Some(*reason),
            _ => None,
        };
        let reason = hard_reason.map(str::to_string).or_else(|| {
            adaptive_rejected.get(&raw.uid).map(|probability| format!("local score {:.1}%", probability * 100.0))
        });
        let Some(reason) = reason else {
            classify_raw.push(raw);
            continue;
        };
        let (account_index, imap_uid) = crate::models::split_mail_id(raw.uid);
        let Some((account, folders)) = mailboxes.get(account_index) else {
            guarded_raw.push(raw);
            continue;
        };
        let folder = folders.spam.clone().unwrap_or_else(|| "Junk".into());
        let folder_ready = if folders.spam.is_some() || ensured_spam.contains(&account_index) {
            Ok(())
        } else {
            let result = crate::imap_client::ensure_folder(&account.imap, &folder).await;
            if result.is_ok() {
                ensured_spam.insert(account_index);
            }
            result
        };
        let moved = match folder_ready {
            Ok(()) => crate::imap_client::move_message(&account.imap, imap_uid, &folder).await,
            Err(e) => Err(e),
        };
        match moved {
            Ok(()) => {
                info!("Spam guard moved uid={} to {} ({})", raw.uid, folder, reason);
                if hard_reason.is_some() {
                    crate::spam_guard::mark_moved(account, imap_uid);
                }
            }
            Err(e) => {
                // Fail closed for token use: show it unsorted, retry the move next refresh,
                // but never let a move failure send attacker-controlled content to the AI.
                warn!("Spam guard could not move uid={} ({}): {:#}", raw.uid, reason, e);
                guarded_raw.push(raw);
            }
        }
    }
    let new_raw = classify_raw;

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
            c.references = raw.references.clone();
            c.date_full = raw.date.map(format_date_full).unwrap_or_default();
            c.date_ts = raw.date.map(|d| d.timestamp()).unwrap_or(0);
            c.date_str = raw.date.map(format_date).unwrap_or_else(|| i18n::unknown_date(i18n::lang()).to_string());
            c.is_read = raw.seen;
        }
    }

    // Emit immediately with cached data + raw new emails (unclassified) so UI isn't blank.
    let new_raw_cards: Vec<EmailCard> =
        new_raw.iter().chain(guarded_raw.iter()).map(|r| raw_to_card(r.clone())).collect();
    let (sent_cards_pre, drafts_cards_pre, trash_cards_pre, spam_cards_pre) = match &preserved_folders {
        Some((sent, drafts, trash, spam)) => (sent.clone(), drafts.clone(), trash.clone(), spam.clone()),
        None => (
            sent_raw.iter().map(|r| raw_to_card(r.clone())).collect(),
            drafts_raw.iter().map(|r| raw_to_card(r.clone())).collect(),
            trash_raw.iter().map(|r| raw_to_card(r.clone())).collect(),
            spam_raw.iter().map(|r| raw_to_card(r.clone())).collect(),
        ),
    };
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
            return Ok(false); // accounts changed while fetching: the refresh runs again
        }
        s.mailboxes = mailboxes;
    }
    // Only a light refresh can spend time classifying arrivals. Show those new
    // messages immediately while classification runs. A full repair does not call
    // AI and therefore needs only its final snapshot.
    if mode == RefreshMode::Light {
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
    }

    // Classify only new emails.
    let mut ai_failures = 0usize;
    let mut first_ai_error: Option<String> = None;
    // Why a Claude subscription sorted nothing, or stopped early (shown in the briefing line).
    let mut ai_notice: Option<Briefing> = None;
    let mut new_cards: Vec<EmailCard> = if new_raw.is_empty() {
        vec![]
    } else {
        let projects = Arc::new(cfg.projects.clone());
        // Emails already classified on another computer: reuse the stored result (no AI call).
        let ids: Vec<String> = new_raw.iter().map(|r| r.message_id.clone()).filter(|id| !id.is_empty()).collect();
        let stored = crate::cloud::lookup_ai(&ids).await;
        let concurrency = crate::ai::parallel_requests(&cfg.ai);
        use futures::StreamExt;
        // Owned jobs first: the stream must not borrow from this function across awaits.
        let board = Board::load();
        let jobs: Vec<_> = new_raw
            .into_iter()
            .map(|email| {
                let candidates = related_open_tasks(&board, &cached_cards, IncomingOrOutgoing::Incoming(&email));
                (stored.get(&email.message_id).cloned(), cfg.ai.clone(), email, projects.clone(), candidates)
            })
            .collect();
        // With a Claude subscription, only a paid plan is used (a free plan or an API-key
        // login would bill the API), and after one email fails (e.g. the usage limit is reached)
        // the rest stay unsorted until the next refresh instead of each failing in turn.
        let claude_code = cfg.ai.uses_claude_code();
        let not_paid = claude_code && !crate::claude_code::status().await.paid;
        let requested = jobs.iter().filter(|(reused, _, _, _, _)| reused.is_none()).count();
        let mut slots = if mode == RefreshMode::Light && !repair_only && !not_paid && ai_ready.is_ok() {
            crate::spam_guard::reserve_ai_slots(requested, chrono::Utc::now().timestamp())
        } else {
            0
        };
        let jobs: Vec<_> = jobs
            .into_iter()
            .map(|(reused, ai_cfg, email, projects, candidates)| {
                let may_call_ai = if reused.is_some() {
                    true
                } else if mode == RefreshMode::Light && !repair_only && slots > 0 {
                    slots -= 1;
                    true
                } else {
                    false
                };
                (reused, ai_cfg, email, projects, candidates, may_call_ai)
            })
            .collect();
        let stop_reason: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let results: Vec<(RawEmail, anyhow::Result<(bool, ClassificationOutcome)>)> = futures::stream::iter(jobs)
            .map(|(reused, ai_cfg, email, projects, candidates, may_call_ai)| {
                let stop_reason = stop_reason.clone();
                async move {
                    let result = if let Some(c) = reused {
                        anyhow::Ok((false, ClassificationOutcome { email: c, task_updates: vec![] }))
                    } else if !may_call_ai
                        || not_paid
                        || stop_reason.lock().unwrap_or_else(|e| e.into_inner()).is_some()
                    {
                        anyhow::Ok((
                            false,
                            ClassificationOutcome { email: crate::ai_classifier::unclassified(), task_updates: vec![] },
                        ))
                    } else {
                        let classified =
                            crate::ai_classifier::classify_with_tasks(&ai_cfg, &email, &projects, &candidates).await;
                        if let (true, Err(e)) = (claude_code, &classified) {
                            stop_reason.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(|| e.to_string());
                        }
                        classified.map(|outcome| (true, outcome))
                    };
                    (email, result)
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
        let mut task_updates = Vec::new();
        let cards: Vec<EmailCard> = results
            .into_iter()
            .map(|(raw, result)| {
                let classified = match result {
                    Ok((fresh, outcome)) => {
                        if fresh && !raw.message_id.is_empty() && crate::ai_classifier::is_real_result(&outcome.email) {
                            to_store.push((raw.message_id.clone(), outcome.email.clone()));
                        }
                        task_updates.extend(outcome.task_updates);
                        outcome.email
                    }
                    // Left unsorted (empty summary); routine refreshes do not reprocess it.
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
                    project: classified.project,
                    deadline: classified.deadline,
                    task: classified.task,
                    meeting_link: classified.meeting_link,
                }
            })
            .collect();
        crate::cloud::store_ai(&to_store).await;
        apply_task_updates(&task_updates);
        cards
    };

    // No notification on the very first fill (empty cache): every email would look new.
    // Emails sorted again after an AI failure are not new either.
    if cfg.user.notify_new_mail && !seen_uids.is_empty() {
        let arrived: Vec<EmailCard> = new_cards.iter().filter(|c| !seen_uids.contains(&c.uid)).cloned().collect();
        notify_new_mail(app, &arrived);
    }
    // Guarded messages whose server move failed remain visible but unclassified. They are
    // remembered by spam_guard and retried without AI on the next refresh.
    new_cards.extend(guarded_raw.into_iter().map(raw_to_card));

    // A full refresh reconciles the fetched recent window. A light refresh only
    // appends arrivals and deliberately leaves every processed cache entry alone.
    if mode == RefreshMode::Full || !reconciled_accounts.is_empty() {
        let known_uid_set: HashSet<u64> = known_raw.iter().map(|r| r.uid).collect();
        cached_cards.retain(|card| {
            let (account, _) = crate::models::split_mail_id(card.uid);
            !reconciled_accounts.contains(&account) || known_uid_set.contains(&card.uid)
        });
    }
    let mut cards = cached_cards;
    cards.extend(new_cards);

    let (sent_cards, drafts_cards, trash_cards, spam_cards) = match preserved_folders {
        Some(cards) => cards,
        None => (
            sent_raw.into_iter().map(raw_to_card).collect(),
            drafts_raw.into_iter().map(raw_to_card).collect(),
            trash_raw.into_iter().map(raw_to_card).collect(),
            spam_raw.into_iter().map(raw_to_card).collect(),
        ),
    };

    // Accounts changed while the AI was sorting: these ids are stale. Keep nothing; the
    // refresh runs again with the new settings.
    if accounts_changed(&state.lock().unwrap().cfg, cfg) {
        return Ok(false);
    }
    save_cache_checked(&cards)?;
    state.lock().unwrap().cache_loaded = true;
    // Advance the cursor only after the corresponding processed cache is durable.
    save_sync_state(&sync)?;

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
        return Ok(false); // accounts changed during the task check
    }
    if mode == RefreshMode::Full {
        let mut s = state.lock().unwrap();
        for (loaded, folder) in secondary_ok.into_iter().zip([
            crate::search::Folder::Sent,
            crate::search::Folder::Drafts,
            crate::search::Folder::Trash,
            crate::search::Folder::Spam,
        ]) {
            if loaded {
                s.loaded_folders.insert(folder);
            } else {
                s.loaded_folders.remove(&folder);
            }
        }
    }
    populate(cfg, app, state, cards, sent_cards, drafts_cards, trash_cards, spam_cards, Some(live_briefing));
    Ok(true)
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
    state.lock().unwrap().cache_loaded = true;
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

    // Derived views borrow the canonical cards. Cloning here used to retain several
    // additional copies of every HTML/text body while each snapshot was assembled.
    let mut urgent: Vec<&EmailCard> = Vec::new();
    let mut high: Vec<&EmailCard> = Vec::new();
    let mut other: Vec<&EmailCard> = Vec::new();
    let mut info_vec: Vec<&EmailCard> = Vec::new();
    let mut threat: Vec<&EmailCard> = Vec::new();
    let mut task_vec: Vec<&EmailCard> = Vec::new();

    for card in &cards {
        match card.priority {
            Priority::Urgent => urgent.push(card),
            Priority::HighPriority => high.push(card),
            Priority::Threat => threat.push(card),
            Priority::Task => task_vec.push(card),
            Priority::Information => info_vec.push(card),
            Priority::OtherPriority => other.push(card),
        }
    }

    let priority_order = |c: &EmailCard| match c.priority {
        Priority::Urgent => 0u8,
        Priority::HighPriority => 1,
        Priority::Task => 2,
        Priority::OtherPriority => 3,
        Priority::Threat => 4,
        Priority::Information => 5,
    };
    let mut all_by_priority: Vec<&EmailCard> = cards.iter().collect();
    all_by_priority.sort_by_key(|card| priority_order(card));

    let dismissed = Dismissals::load();
    // New email action items land in the Todo column of the task board (and email
    // tasks from before dates were kept get the date of their email).
    let needs_sync = Board::load().needs_sync(&cards);
    if needs_sync {
        Board::update(|b| b.sync_from_cards(&cards, &dismissed));
    }
    let board_tasks = Board::load().tasks;
    let now_ts = Local::now().timestamp();
    let detected_verifications: Vec<VerificationDto> = cards.iter().filter_map(crate::verification::extract).collect();
    let verification_uids: std::collections::HashSet<u64> =
        detected_verifications.iter().map(|item| item.uid).collect();
    let verification_items: Vec<VerificationDto> = detected_verifications
        .into_iter()
        .filter(|item| {
            item.expires_ts > now_ts
                && item.expires_ts <= now_ts + crate::verification::LIFETIME_SECONDS + 5 * 60
                && !dismissed.verifications.contains(&item.uid)
        })
        .collect();
    // Verification mail remains in Inbox but never competes with genuinely important work on
    // Overview, even after its short-lived code has expired.
    urgent.retain(|card| !verification_uids.contains(&card.uid));
    high.retain(|card| !verification_uids.contains(&card.uid));
    let important: Vec<&EmailCard> =
        urgent.iter().chain(high.iter()).copied().filter(|card| !card.is_read).take(5).collect();
    let meeting_items: Vec<&EmailCard> =
        cards.iter().filter(|c| c.meeting_link.is_some() && !dismissed.meetings.contains(&c.uid)).collect();

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
            let email_count = cards.iter().filter(|c| c.project.as_deref() == Some(&p.name)).count() as i32;
            ProjectDto {
                name: p.name.clone(),
                urgency: p.urgency.clone(),
                description: p.description.clone(),
                email_count,
                warning: None,
            }
        })
        .collect();
    let directory = crate::contacts::load();

    // Store all emails in shared state for command lookups.
    {
        let mut s = state.lock().unwrap();
        let mut all: Vec<EmailCard> = cards.clone();
        all.extend(sent.clone());
        all.extend(drafts.clone());
        all.extend(trash.clone());
        all.extend(spam.clone());
        s.emails = all;
        use crate::search::Folder;
        let tag = |f: Folder, v: &[EmailCard]| v.iter().cloned().map(move |c| (f, c)).collect::<Vec<_>>();
        let mut index = tag(Folder::Inbox, &cards);
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
        emails: summary_dtos(&cards, &sent, &drafts, &trash, &spam),
        urgent_emails: email_ref_keys("inbox", &urgent),
        high_emails: email_ref_keys("inbox", &high),
        other_emails: email_ref_keys("inbox", &other),
        info_emails: email_ref_keys("inbox", &info_vec),
        threat_emails: email_ref_keys("inbox", &threat),
        task_emails: email_ref_keys("inbox", &task_vec),
        all_emails: email_keys("inbox", &cards),
        tasks: board_tasks,
        meeting_items: email_ref_keys("inbox", &meeting_items),
        verification_items,
        important_emails: email_ref_keys("inbox", &important),
        all_emails_by_priority: email_ref_keys("inbox", &all_by_priority),
        sent_emails: email_keys("sent", &sent),
        drafts_emails: email_keys("drafts", &drafts),
        trash_emails: email_keys("trash", &trash),
        spam_emails: email_keys("spam", &spam),
        deadlines: deadline_dtos,
        week_days,
        projects: project_dtos,
        contacts: directory.contacts,
        potential_contacts: directory.potential_contacts,
        companies: directory.companies,
        calendar_year: cal_year,
        calendar_month: cal_month,
    };

    app.emit("state-updated", snapshot).ok();
}

fn email_key(folder: &str, uid: u64) -> String {
    format!("{folder}:{uid}")
}

fn email_keys(folder: &str, cards: &[EmailCard]) -> Vec<String> {
    cards.iter().map(|card| email_key(folder, card.uid)).collect()
}

fn email_ref_keys(folder: &str, cards: &[&EmailCard]) -> Vec<String> {
    cards.iter().map(|card| email_key(folder, card.uid)).collect()
}

fn summary_dtos(
    inbox: &[EmailCard],
    sent: &[EmailCard],
    drafts: &[EmailCard],
    trash: &[EmailCard],
    spam: &[EmailCard],
) -> Vec<crate::dto::EmailSummaryDto> {
    let total = inbox.len() + sent.len() + drafts.len() + trash.len() + spam.len();
    let mut result = Vec::with_capacity(total);
    for (folder, cards) in [("inbox", inbox), ("sent", sent), ("drafts", drafts), ("trash", trash), ("spam", spam)] {
        result.extend(cards.iter().map(|card| crate::dto::EmailSummaryDto {
            key: email_key(folder, card.uid),
            folder: folder.to_string(),
            uid: card.uid,
            subject: card.subject.clone(),
            from_name: card.from_name.clone(),
            from_email: card.from_email.clone(),
            message_id: card.message_id.clone(),
            references: card.references.clone(),
            date_str: card.date_str.clone(),
            date_full: card.date_full.clone(),
            date_ts: card.date_ts,
            summary: card.summary.clone(),
            priority: card.priority.as_str().to_string(),
            is_read: card.is_read,
            project: card.project.clone().unwrap_or_default(),
            attachment_count: card.attachments.len(),
        }));
    }
    result
}

pub fn to_dto_vec(cards: &[EmailCard]) -> Vec<EmailCardDto> {
    cards.iter().map(to_dto).collect()
}

pub fn to_dto(c: &EmailCard) -> EmailCardDto {
    EmailCardDto {
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
        meeting_link: c.meeting_link.clone().unwrap_or_default(),
    }
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
        s.loaded_folders.clear();
    }
    redisplay(cfg, app, state);
}

fn load_cache() -> Vec<EmailCard> {
    match std::fs::read_to_string(crate::paths::file(CACHE_PATH)) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => vec![],
    }
}

fn save_cache_checked(cards: &[EmailCard]) -> Result<()> {
    let json = serde_json::to_vec(cards)?;
    let path = crate::paths::file(CACHE_PATH);
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, json)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

fn save_cache(cards: &[EmailCard]) {
    if let Err(error) = save_cache_checked(cards) {
        warn!("Cache save error: {:#}", error);
    }
}

/// The special folders found on the server. `spam` is None when the mailbox has none
/// (it is then created the first time an email is marked as spam).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecialFolders {
    pub sent: String,
    pub drafts: String,
    pub trash: String,
    pub spam: Option<String>,
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
    }
}

async fn guess_folder_names(cfg: &ImapConfig) -> SpecialFolders {
    use async_imap::types::NameAttribute;
    use futures::StreamExt;

    let fallback = SpecialFolders { sent: "Sent".into(), drafts: "Drafts".into(), trash: "Trash".into(), spam: None };
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

enum IncomingOrOutgoing<'a> {
    Incoming(&'a crate::models::RawEmail),
    Outgoing { to: &'a str, cc: &'a str, bcc: &'a str, subject: &'a str, in_reply_to: Option<&'a str> },
}

fn normalized_subject(subject: &str) -> String {
    let mut value = subject.trim();
    loop {
        let lower = value.to_ascii_lowercase();
        let prefix = ["re:", "fw:", "fwd:", "tr:", "aw:"].into_iter().find(|prefix| lower.starts_with(prefix));
        let Some(prefix) = prefix else { break };
        value = value[prefix.len()..].trim_start();
    }
    value.to_lowercase()
}

fn address_set(values: &[&str]) -> HashSet<String> {
    values
        .iter()
        .flat_map(|value| crate::search::split_addresses(value))
        .filter_map(|entry| {
            let address = entry
                .rsplit_once('<')
                .map(|(_, tail)| tail.trim_end_matches('>'))
                .unwrap_or(&entry)
                .trim()
                .to_lowercase();
            address.contains('@').then_some(address)
        })
        .collect()
}

/// Select only open email-sourced tasks plausibly related to this one incoming
/// or outgoing message. Thread headers rank first, then the same participant and
/// subject, then the same participant. The cap keeps the single prompt small.
fn related_open_tasks(board: &Board, cards: &[EmailCard], mail: IncomingOrOutgoing<'_>) -> Vec<OpenTask> {
    let (participants, subject, references) = match mail {
        IncomingOrOutgoing::Incoming(email) => {
            let (_, from) = parse_from(&email.from);
            ([from.to_lowercase()].into_iter().collect(), normalized_subject(&email.subject), email.references.as_str())
        }
        IncomingOrOutgoing::Outgoing { to, cc, bcc, subject, in_reply_to } => {
            (address_set(&[to, cc, bcc]), normalized_subject(subject), in_reply_to.unwrap_or_default())
        }
    };
    let mut matches = board
        .tasks
        .iter()
        .filter(|task| task.status.is_open())
        .filter_map(|task| {
            let uid = task.source_uid?;
            let card = cards.iter().find(|card| card.uid == uid)?;
            let thread_match = !card.message_id.is_empty() && references.contains(&card.message_id);
            let participant_match = participants.contains(&card.from_email.to_lowercase());
            if !thread_match && !participant_match {
                return None;
            }
            let subject_match = !subject.is_empty() && subject == normalized_subject(&card.subject);
            let score = if thread_match {
                3
            } else if subject_match {
                2
            } else {
                1
            };
            Some((
                score,
                task.created_ts,
                OpenTask {
                    uid,
                    task: task.title.clone(),
                    subject: card.subject.clone(),
                    from: if card.from_name.is_empty() { card.from_email.clone() } else { card.from_name.clone() },
                    date: card.date_full.clone(),
                },
            ))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    matches.into_iter().take(8).map(|(_, _, task)| task).collect()
}

fn apply_task_updates(updates: &[TaskUpdate]) -> bool {
    if updates.is_empty() {
        return false;
    }
    Board::update(|board| {
        let mut changed = false;
        for update in updates {
            let Some(task) =
                board.tasks.iter_mut().find(|task| task.source_uid == Some(update.id) && task.status.is_open())
            else {
                continue;
            };
            task.status = if update.status == "done" { Status::Done } else { Status::GivenUp };
            task.ai_verdict = Some(update.status.clone());
            info!("Task uid={} is {}", update.id, update.status);
            changed = true;
        }
        changed
    })
}

/// A successful outgoing message is the only second task-completion trigger.
/// It makes no model request unless at least one open task matches its thread or
/// recipients, and sends only those candidates plus this one message.
pub async fn check_outgoing_tasks(
    cfg: &Config,
    app: &AppHandle,
    state: &Arc<Mutex<AppState>>,
    mail: &crate::smtp_sender::OutgoingMail,
) -> Result<bool> {
    if crate::ai::readiness(&cfg.ai).is_err() {
        return Ok(false);
    }
    let cards = state.lock().unwrap().emails.clone();
    let board = Board::load();
    let candidates = related_open_tasks(
        &board,
        &cards,
        IncomingOrOutgoing::Outgoing {
            to: &mail.to,
            cc: &mail.cc,
            bcc: &mail.bcc,
            subject: &mail.subject,
            in_reply_to: mail.in_reply_to.as_deref(),
        },
    );
    if candidates.is_empty() {
        return Ok(false);
    }
    if crate::spam_guard::reserve_ai_slots(1, Utc::now().timestamp()) == 0 {
        info!("Skipping outgoing task check: hourly AI budget reached");
        return Ok(false);
    }
    let evidence = RecentMail {
        folder: "sent",
        from: cfg.user_name(),
        to: [&mail.to, &mail.cc, &mail.bcc]
            .into_iter()
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
        subject: mail.subject.clone(),
        date: format_date_full(Utc::now()),
        snippet: html2text::from_read(mail.html.as_bytes(), 80).chars().take(300).collect(),
    };
    info!("Checking outgoing email against {} related open task(s)", candidates.len());
    let verdicts = crate::ai_classifier::check_tasks(&cfg.ai, &candidates, &[evidence]).await?;
    let updates =
        verdicts.into_iter().map(|(id, status)| TaskUpdate { id, status: status.to_string() }).collect::<Vec<_>>();
    let changed = apply_task_updates(&updates);
    if changed {
        redisplay(cfg, app, state);
    }
    Ok(changed)
}

/// Unread urgent + high emails for the Overview, at most 5.
#[cfg(test)]
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
