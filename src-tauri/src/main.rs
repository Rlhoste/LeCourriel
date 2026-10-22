// Le courriel — an AI-assisted email client.
// Copyright (C) 2026 Remi Lhoste
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{Arc, Mutex};
use tauri::{AppHandle, State};
use tracing::info;

mod ai;
mod ai_assistant;
mod ai_classifier;
mod ai_writer;
mod app_state;
mod auto_scan;
mod auto_setup;
#[cfg(test)]
mod tests;
mod config;
mod dismissals;
mod dto;
mod gifs;
mod i18n;
mod imap_client;
mod models;
mod overrides;
mod paths;
mod search;
mod cloud;
mod settings;
mod signature;
mod smtp_sender;
mod state;
mod tasks;

use config::Config;
use state::AppState;

type SharedState = Arc<Mutex<AppState>>;

// ── Commands ─────────────────────────────────────────────────────────────────

#[tauri::command]
async fn app_ready(
    app: AppHandle,
    shared: State<'_, SharedState>,
) -> Result<(), String> {
    use tauri::Emitter;
    let (cfg, startup_error) = {
        let s = shared.lock().unwrap();
        (s.cfg.clone(), s.startup_error.clone())
    };
    let shared_clone = shared.inner().clone();
    if let Some(e) = startup_error {
        app.emit("error", format!("config.toml: {}", e)).ok();
    }
    // Send cache immediately (fast, synchronous read)
    app_state::load_and_display_cache(&cfg, &app, &shared_clone);
    if cfg.accounts.is_empty() {
        // First start: nothing to fetch until an account is added in Settings.
        app.emit("loading", false).ok();
        return Ok(());
    }
    tauri::async_runtime::spawn(async move {
        let cfg = sync_from_database(&app, &shared_clone).await.unwrap_or(cfg);
        // Kick off live refresh in background
        app_state::refresh(cfg, app, shared_clone).await;
    });
    Ok(())
}

/// Startup sync with Supabase. Returns the reloaded config when
/// anything was pulled (preferences may have changed), None otherwise.
async fn sync_from_database(app: &AppHandle, shared: &SharedState) -> Option<Arc<Config>> {
    use tauri::Emitter;
    if !cloud::is_configured() {
        return None;
    }
    match cloud::pull_all().await {
        Ok(summary) => {
            info!("Cloud sync: {}", summary);
            let cfg = Arc::new(Config::load().ok()?);
            i18n::set_lang(&cfg.user.language);
            shared.lock().unwrap().cfg = cfg.clone();
            // Re-show the cache: tasks or dismissals may have come from another computer.
            app_state::load_and_display_cache(&cfg, app, shared);
            Some(cfg)
        }
        Err(e) => {
            tracing::warn!("Cloud sync failed: {:#}", e);
            app.emit("error", format!("Cloud sync failed: {:#}", e)).ok();
            None
        }
    }
}

#[tauri::command]
async fn refresh_emails(
    app: AppHandle,
    shared: State<'_, SharedState>,
) -> Result<(), String> {
    let cfg = {
        let s = shared.lock().unwrap();
        s.cfg.clone()
    };
    let shared_clone = shared.inner().clone();
    app_state::refresh(cfg, app, shared_clone).await;
    Ok(())
}

#[tauri::command]
fn get_email(uid: u32, shared: State<'_, SharedState>) -> Option<dto::EmailCardDto> {
    let s = shared.lock().unwrap();
    s.emails.iter().find(|e| e.uid == uid).map(|c| dto::EmailCardDto {
        uid: c.uid,
        subject: c.subject.clone(),
        from_name: c.from_name.clone(),
        from_email: c.from_email.clone(),
        to: c.to.clone(),
        cc: c.cc.clone(),
        reply_to: c.reply_to.clone(),
        bcc: c.bcc.clone(),
        message_id: c.message_id.clone(),
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
        meeting_link: c.meeting_link.clone().unwrap_or_default(),
    })
}

#[tauri::command]
async fn send_email(
    to: String,
    cc: String,
    bcc: String,
    subject: String,
    html: String,
    in_reply_to: Option<String>,
    account: Option<usize>,
    shared: State<'_, SharedState>,
) -> Result<(), String> {
    // The account chosen in the composer's "From" (the first one by default).
    let index = account.unwrap_or(0);
    let (cfg, sent_folder) = {
        let s = shared.lock().unwrap();
        (s.cfg.clone(), s.sent_folders.get(index).cloned().unwrap_or_else(|| "Sent".into()))
    };
    let account = cfg.accounts.get(index).ok_or("No accounts configured")?;
    let mail = smtp_sender::OutgoingMail { to, cc, bcc, subject, html, in_reply_to };
    let smtp = account.smtp_for_sending(&cfg.user_name());
    let raw = smtp_sender::send_message(&smtp, &mail)
        .await
        .map_err(|e| format!("{:#}", e))?;
    // The SMTP relay doesn't keep a copy, so store one in Sent ourselves.
    if let Err(e) = imap_client::append_sent(&account.imap, &sent_folder, &raw).await {
        tracing::warn!("Could not save a copy to {}: {:#}", sent_folder, e);
    }
    Ok(())
}

#[tauri::command]
fn get_settings(shared: State<'_, SharedState>) -> dto::SettingsDto {
    let s = shared.lock().unwrap();
    settings::config_to_dto(&s.cfg)
}

#[tauri::command]
fn save_settings(
    display_name: Option<String>,
    accounts: Vec<dto::AccountDto>,
    ai: dto::AiSettingsDto,
    giphy_key: Option<String>,
    font_scale: f32,
    theme: Option<String>,
    language: Option<String>,
    shared: State<'_, SharedState>,
) -> Result<(), String> {
    let giphy_key = giphy_key.unwrap_or_default();
    settings::save_config(display_name.as_deref(), &accounts, &ai, &giphy_key, font_scale, theme.as_deref(), language.as_deref())
        .map_err(|e| e.to_string())?;
    // Reload config into shared state.
    match Config::load() {
        Ok(new_cfg) => {
            signature::remove_unused(&signature::data_dir(), &new_cfg);
            i18n::set_lang(&new_cfg.user.language);
            cloud::configure(&new_cfg);
            let mut s = shared.lock().unwrap();
            s.cfg = Arc::new(new_cfg);
        }
        Err(e) => return Err(e.to_string()),
    }
    Ok(())
}

// ── Signature ──────────────────────────────────────────────────────────────

/// Import an HTML signature file (local images are embedded). Returns its new file name
/// (set on the account when Settings are saved) and its HTML.
#[tauri::command]
fn signature_import(path: String) -> Result<(String, String), String> {
    signature::import(std::path::Path::new(&path), &signature::data_dir()).map_err(|e| format!("{:#}", e))
}

/// The HTML of a signature file ("" when there is none).
#[tauri::command]
fn signature_get(file: String) -> String {
    signature::load(&signature::data_dir(), &file)
}

/// Each account's sender and signature, for the composer's "From" choice.
#[tauri::command]
fn compose_accounts(shared: State<'_, SharedState>) -> Vec<dto::ComposeAccountDto> {
    let cfg = shared.lock().unwrap().cfg.clone();
    let dir = signature::data_dir();
    cfg.accounts.iter().map(|a| {
        let from = a.smtp_for_sending(&cfg.user_name()).from_address;
        dto::ComposeAccountDto {
            from: if from.trim().is_empty() { a.name.clone() } else { from },
            signature_html: signature::load(&dir, &a.signature.file),
            signature_on_new: a.signature.on_new,
            signature_on_reply: a.signature.on_reply,
        }
    }).collect()
}

// ── AI provider ────────────────────────────────────────────────────────────

/// An AiConfig from the values currently typed in Settings (not saved yet).
fn ai_config_from(provider: &str, model: &str, base_url: &str, api_key: &str) -> config::AiConfig {
    let mut ai = config::AiConfig {
        provider: provider.to_string(),
        model: model.trim().to_string(),
        base_url: base_url.trim().to_string(),
        ..Default::default()
    };
    if !api_key.trim().is_empty() {
        ai.keys.insert(ai.provider().to_string(), api_key.trim().to_string());
    }
    ai
}

#[tauri::command]
async fn ai_list_models(provider: String, base_url: String, api_key: String) -> Result<Vec<String>, String> {
    ai::list_models(&ai_config_from(&provider, "", &base_url, &api_key))
        .await
        .map_err(|e| format!("{:#}", e))
}

/// Settings → Accounts → Test: log in to the incoming server and open the folder.
/// Returns the number of emails in it. Uses the values typed, not the saved ones.
#[tauri::command]
async fn account_test_imap(account: dto::AccountDto) -> Result<u32, dto::ConnError> {
    let acc = settings::account_from_dto(&account);
    match tokio::time::timeout(CONN_TEST_TIMEOUT, imap_client::check_connection(&acc.imap)).await {
        Ok(r) => r.map_err(|e| dto::ConnError::from_error(&e)),
        Err(_) => Err(dto::ConnError::timeout()),
    }
}

/// Settings → Accounts → Test: connect and log in to the outgoing server (no email is sent).
#[tauri::command]
async fn account_test_smtp(account: dto::AccountDto) -> Result<(), dto::ConnError> {
    let smtp = settings::account_from_dto(&account).smtp_for_sending("");
    match tokio::time::timeout(CONN_TEST_TIMEOUT, smtp_sender::check_connection(&smtp)).await {
        Ok(r) => r.map_err(|e| dto::ConnError::from_error(&e)),
        Err(_) => Err(dto::ConnError::timeout()),
    }
}

const CONN_TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Account setup → Auto-setup: look up the server settings of an email address.
/// Only its domain is sent. Ok(None) means no source knows that domain.
#[tauri::command]
async fn lookup_mail_settings(email: String) -> Result<Option<dto::MailSetupDto>, String> {
    auto_setup::lookup(&email).await.map_err(|e| format!("{:#}", e))
}

/// Account setup → Auto-scan: the accounts Thunderbird or Betterbird holds on this
/// computer, with their server settings. Never a password; no app installed means nothing
/// found rather than an error.
#[tauri::command]
fn scan_mail_clients() -> Vec<dto::FoundAccountDto> {
    auto_scan::scan()
}

/// Send a tiny request; returns the answer and how long it took.
#[tauri::command]
async fn ai_test(provider: String, model: String, base_url: String, api_key: String) -> Result<(String, u64), String> {
    let cfg = ai_config_from(&provider, &model, &base_url, &api_key);
    let started = std::time::Instant::now();
    let answer = ai::complete(&cfg, ai::Request {
        system: "You are a connectivity check. Reply with the single word OK.",
        user: "Reply with OK.",
        max_tokens: 20,
        json: false,
    })
    .await
    .map_err(|e| format!("{:#}", e))?;
    Ok((answer.trim().chars().take(60).collect(), started.elapsed().as_millis() as u64))
}

// ── Search & AI assistant ───────────────────────────────────────────────────

/// Instant local search (stage 1), over cached emails and the last server results.
#[tauri::command]
fn search_local(query: String, shared: State<'_, SharedState>) -> Vec<search::SearchHit> {
    let s = shared.lock().unwrap();
    search::search_local(&s.search_index, &query, 40)
}

/// Search the whole mailbox on the IMAP server; results join the index so they can be opened.
#[tauri::command]
async fn search_server(query: String, shared: State<'_, SharedState>) -> Result<Vec<search::SearchHit>, String> {
    let cfg = shared.lock().unwrap().cfg.clone();
    let account = cfg.accounts.first().ok_or("No accounts configured")?;
    let (terms, filters) = search::parse_query(&query);
    if terms.is_empty() && filters.after.is_none() {
        return Err("Type at least one word to search on the server".into());
    }
    let raws = imap_client::search_server(&account.imap, &terms, filters.after, 20)
        .await
        .map_err(|e| format!("{:#}", e))?;
    let mut s = shared.lock().unwrap();
    let cached: std::collections::HashSet<u32> = s.search_index.iter()
        .filter(|(f, _)| *f == search::Folder::Inbox).map(|(_, c)| c.uid).collect();
    s.search_index.retain(|(f, _)| *f != search::Folder::Server);
    let mut hits = Vec::new();
    for raw in raws {
        if cached.contains(&raw.uid) {
            continue;   // already in the local results
        }
        let card = app_state::raw_to_card(raw);
        let (matched, score) = search::score(&card, &terms);
        hits.push(search::make_hit(search::Folder::Server, &card, &terms, matched, score));
        s.search_index.push((search::Folder::Server, card));
    }
    Ok(hits)
}

/// Any search result as a full email (inbox, sent, drafts, trash or server).
#[tauri::command]
fn get_search_email(id: String, shared: State<'_, SharedState>) -> Option<dto::EmailCardDto> {
    let (folder, uid) = search::parse_id(&id)?;
    let s = shared.lock().unwrap();
    let (_, card) = s.search_index.iter().find(|(f, c)| *f == folder && c.uid == uid)?;
    app_state::to_dto_vec(std::slice::from_ref(card)).pop()
}

/// Stage 2: the AI decides what the request means (find, compose, summarize, answer)
/// using the best local candidates, or exactly the selected emails.
#[tauri::command]
async fn ai_assist(
    query: String,
    mode: String,
    selected_ids: Vec<String>,
    shared: State<'_, SharedState>,
) -> Result<ai_assistant::AssistResult, String> {
    let (cfg, index) = {
        let s = shared.lock().unwrap();
        (s.cfg.clone(), s.search_index.clone())
    };
    let find = |id: &str| {
        let (folder, uid) = search::parse_id(id)?;
        index.iter().find(|(f, c)| *f == folder && c.uid == uid)
    };
    let mut picked: Vec<&(search::Folder, models::EmailCard)> = Vec::new();
    if !selected_ids.is_empty() {
        picked.extend(selected_ids.iter().filter_map(|id| find(id)));
    } else {
        for hit in search::search_local(&index, &query, 25) {
            if let Some(e) = find(&hit.id) { picked.push(e); }
        }
        // Few keyword matches (natural-language request): add the most recent emails.
        if picked.len() < 8 {
            let mut recent: Vec<&(search::Folder, models::EmailCard)> = index.iter()
                .filter(|(f, _)| matches!(f, search::Folder::Inbox | search::Folder::Sent | search::Folder::Server))
                .collect();
            recent.sort_by(|a, b| b.1.date_ts.cmp(&a.1.date_ts));
            for e in recent {
                if picked.len() >= 25 { break; }
                if !picked.iter().any(|p| p.0 == e.0 && p.1.uid == e.1.uid) { picked.push(e); }
            }
        }
    }
    let candidates: Vec<ai_assistant::Candidate> = picked.iter().map(|(f, c)| ai_assistant::Candidate {
        id: search::make_id(*f, c.uid),
        date: if c.date_full.is_empty() { c.date_str.clone() } else { c.date_full.clone() },
        from: if c.from_email.is_empty() { c.from_name.clone() } else { format!("{} <{}>", c.from_name, c.from_email) },
        to: c.to.clone(),
        subject: c.subject.clone(),
        summary: c.summary.clone(),
        body: c.body_text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(700).collect(),
    }).collect();
    let contacts = search::contacts(&index, 60);
    let today = chrono::Local::now().format("%Y-%m-%d (%A)").to_string();
    ai_assistant::assist(&cfg, &query, &mode, &candidates, &contacts, &today)
        .await
        .map_err(|e| format!("{:#}", e))
}

// ── Cloud sync (Supabase) ──────────────────────────────────────────────────

#[tauri::command]
fn cloud_status() -> cloud::CloudStatus {
    cloud::status()
}

/// Save (or clear, with empty values) the Supabase project settings. Local only.
#[tauri::command]
fn set_cloud(url: String, anon_key: String, shared: State<'_, SharedState>) -> Result<cloud::CloudStatus, String> {
    let mut cfg = (*shared.lock().unwrap().cfg).clone();
    let (url, anon_key) = (url.trim().to_string(), anon_key.trim().to_string());
    cfg.supabase = if url.is_empty() && anon_key.is_empty() {
        None
    } else {
        Some(config::SupabaseConfig { url, anon_key })
    };
    config::write_local(&cfg).map_err(|e| e.to_string())?;
    cloud::configure(&cfg);
    shared.lock().unwrap().cfg = Arc::new(cfg);
    Ok(cloud::status())
}

/// Sign in, then sync right away.
#[tauri::command]
async fn cloud_sign_in(
    email: String,
    password: String,
    app: AppHandle,
    shared: State<'_, SharedState>,
) -> Result<cloud::CloudStatus, String> {
    cloud::sign_in(&email, &password).await.map_err(|e| format!("{:#}", e))?;
    sync_from_database(&app, shared.inner()).await;
    Ok(cloud::status())
}

/// Create an account. Returns true when signed in straight away, false when a
/// confirmation email was sent (the user confirms, then signs in).
#[tauri::command]
async fn cloud_sign_up(
    email: String,
    password: String,
    app: AppHandle,
    shared: State<'_, SharedState>,
) -> Result<bool, String> {
    let signed_in = cloud::sign_up(&email, &password).await.map_err(|e| format!("{:#}", e))?;
    if signed_in {
        sync_from_database(&app, shared.inner()).await;
    }
    Ok(signed_in)
}

#[tauri::command]
async fn cloud_sign_out() -> cloud::CloudStatus {
    cloud::sign_out().await;
    cloud::status()
}

#[tauri::command]
async fn cloud_sync_now(app: AppHandle, shared: State<'_, SharedState>) -> Result<(), String> {
    if !cloud::is_configured() {
        return Err("Not signed in to Supabase".into());
    }
    sync_from_database(&app, shared.inner()).await;
    Ok(())
}

#[tauri::command]
fn get_qr(shared: State<'_, SharedState>) -> String {
    let s = shared.lock().unwrap();
    settings::generate_qr(&s.cfg)
}

#[tauri::command]
async fn save_project_description(
    name: String,
    description: String,
    shared: State<'_, SharedState>,
) -> Result<(), String> {
    let mut cfg = {
        let s = shared.lock().unwrap();
        (*s.cfg).clone()
    };
    if let Some(p) = cfg.projects.iter_mut().find(|p| p.name == name) {
        p.description = description;
    }
    config::save_config(&cfg).map_err(|e| e.to_string())?;
    let mut s = shared.lock().unwrap();
    s.cfg = Arc::new(cfg);
    Ok(())
}

#[tauri::command]
async fn add_project(
    name: String,
    urgency: String,
    shared: State<'_, SharedState>,
) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Project name cannot be empty".into());
    }
    let mut cfg = {
        let s = shared.lock().unwrap();
        (*s.cfg).clone()
    };
    if cfg.projects.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
        return Err(format!("A project named \"{}\" already exists", name));
    }
    cfg.projects.push(config::ProjectConfig {
        name,
        description: String::new(),
        urgency: if urgency == "high" { "high".into() } else { "normal".into() },
    });
    config::save_config(&cfg).map_err(|e| e.to_string())?;
    let mut s = shared.lock().unwrap();
    s.cfg = Arc::new(cfg);
    Ok(())
}

/// Move an inbox email to another folder, then forget it locally.
async fn move_email(shared: &SharedState, uid: u32, folder: String) -> Result<(), String> {
    let cfg = shared.lock().unwrap().cfg.clone();
    let account = cfg.accounts.first().ok_or("No accounts configured")?;
    imap_client::move_message(&account.imap, uid, &folder)
        .await
        .map_err(|e| format!("{:#}", e))?;
    {
        let mut s = shared.lock().unwrap();
        s.emails.retain(|e| e.uid != uid);
        s.search_index.retain(|(f, e)| !(*f == search::Folder::Inbox && e.uid == uid));
    }
    app_state::remove_from_cache(uid);
    Ok(())
}

/// Move an inbox email to the Trash folder on the server.
#[tauri::command]
async fn trash_email(uid: u32, shared: State<'_, SharedState>) -> Result<(), String> {
    let folder = shared.lock().unwrap().trash_folder.clone();
    move_email(shared.inner(), uid, folder).await
}

/// "Not spam": move an email from the Spam/Junk folder back to the inbox.
#[tauri::command]
async fn not_spam_email(uid: u32, shared: State<'_, SharedState>) -> Result<(), String> {
    let (cfg, spam_folder) = {
        let s = shared.lock().unwrap();
        (s.cfg.clone(), s.spam_folder.clone())
    };
    let account = cfg.accounts.first().ok_or("No accounts configured")?;
    // move_message moves from the configured mailbox: point it at the spam folder.
    let from_spam = config::ImapConfig { mailbox: spam_folder, ..account.imap.clone() };
    imap_client::move_message(&from_spam, uid, &account.imap.mailbox)
        .await
        .map_err(|e| format!("{:#}", e))?;
    // Spam UIDs have their own numbering: forget only the spam entry. The email comes
    // back in the inbox at the next refresh.
    let mut s = shared.lock().unwrap();
    s.search_index.retain(|(f, e)| !(*f == search::Folder::Spam && e.uid == uid));
    Ok(())
}

/// Move an inbox email to the Spam/Junk folder on the server.
#[tauri::command]
async fn spam_email(uid: u32, shared: State<'_, SharedState>) -> Result<(), String> {
    let (cfg, folder) = {
        let s = shared.lock().unwrap();
        (s.cfg.clone(), s.spam_folder.clone())
    };
    // Many mailboxes have no spam folder: create it the first time, like other mail clients.
    let account = cfg.accounts.first().ok_or("No accounts configured")?;
    imap_client::ensure_folder(&account.imap, &folder)
        .await
        .map_err(|e| format!("{:#}", e))?;
    move_email(shared.inner(), uid, folder).await
}

/// Mark an inbox email read or unread, on the server and locally.
#[tauri::command]
async fn set_read(uid: u32, read: bool, shared: State<'_, SharedState>) -> Result<(), String> {
    let cfg = shared.lock().unwrap().cfg.clone();
    let account = cfg.accounts.first().ok_or("No accounts configured")?;
    imap_client::set_seen(&account.imap, uid, read)
        .await
        .map_err(|e| format!("{:#}", e))?;
    {
        let mut s = shared.lock().unwrap();
        if let Some(c) = s.emails.iter_mut().find(|e| e.uid == uid) {
            c.is_read = read;
        }
        if let Some((_, c)) = s.search_index.iter_mut().find(|(f, e)| *f == search::Folder::Inbox && e.uid == uid) {
            c.is_read = read;
        }
    }
    app_state::set_read_in_cache(uid, read);
    Ok(())
}

/// Re-tag an inbox email (and the tasks taken from it) to another project, or none.
#[tauri::command]
fn set_email_project(
    uid: u32,
    project: Option<String>,
    shared: State<'_, SharedState>,
) -> Result<(), String> {
    let project = project.filter(|p| !p.is_empty());
    if let Some(p) = &project {
        if !shared.lock().unwrap().cfg.projects.iter().any(|cp| &cp.name == p) {
            return Err(format!("Unknown project: {}", p));
        }
    }
    overrides::Overrides::update(|o| o.set_project(uid, project.clone()));
    {
        let mut s = shared.lock().unwrap();
        if let Some(c) = s.emails.iter_mut().find(|e| e.uid == uid) {
            c.project = project.clone();
        }
        if let Some((_, c)) = s.search_index.iter_mut().find(|(f, e)| *f == search::Folder::Inbox && e.uid == uid) {
            c.project = project.clone();
        }
    }
    tasks::Board::update(|b| {
        for t in b.tasks.iter_mut().filter(|t| t.source_uid == Some(uid)) {
            t.project = project.clone();
        }
    });
    Ok(())
}

/// Save the original message as an .eml file at `path`.
#[tauri::command]
async fn save_email(uid: u32, path: String, shared: State<'_, SharedState>) -> Result<(), String> {
    let cfg = shared.lock().unwrap().cfg.clone();
    let account = cfg.accounts.first().ok_or("No accounts configured")?;
    let raw = imap_client::fetch_raw(&account.imap, uid)
        .await
        .map_err(|e| format!("{:#}", e))?;
    std::fs::write(&path, raw).map_err(|e| format!("Could not write {}: {}", path, e))
}

/// Hide a task, deadline or meeting item without touching the email.
#[tauri::command]
fn dismiss_item(kind: String, uid: u32) -> Result<(), String> {
    dismissals::Dismissals::update(|d| d.dismiss(&kind, uid))
}

// ── AI writing (composer) ─────────────────────────────────────────────────

/// action: "formal" | "translate_en" | "correct" | "auto_reply".
/// The `original_*` fields describe the email being answered (auto_reply only).
#[tauri::command]
async fn ai_compose(
    action: String,
    html: String,
    original_subject: Option<String>,
    original_from: Option<String>,
    original_date: Option<String>,
    original_text: Option<String>,
    shared: State<'_, SharedState>,
) -> Result<String, String> {
    let cfg = shared.lock().unwrap().cfg.clone();
    let original = original_text.map(|text| ai_writer::Original {
        subject: original_subject.unwrap_or_default(),
        from: original_from.unwrap_or_default(),
        date: original_date.unwrap_or_default(),
        text,
    });
    ai_writer::run(&cfg, &action, &html, original)
        .await
        .map_err(|e| format!("{:#}", e))
}

// ── GIFs ──────────────────────────────────────────────────────────────────

#[tauri::command]
async fn search_gifs(query: String, shared: State<'_, SharedState>) -> Result<Vec<gifs::GifDto>, String> {
    let key = shared.lock().unwrap().cfg.gifs.key().to_string();
    gifs::search(&key, &query).await.map_err(|e| format!("{:#}", e))
}

// ── Task board ─────────────────────────────────────────────────────────────
// Each command returns the whole board so the UI can re-render from it.

#[tauri::command]
fn create_task(title: String, project: Option<String>) -> Result<Vec<tasks::Task>, String> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("Task title cannot be empty".into());
    }
    let project = project.filter(|p| !p.is_empty());
    Ok(tasks::Board::update(|b| {
        b.add(title, project);
        b.tasks.clone()
    }))
}

#[tauri::command]
fn set_task_status(id: u64, status: String) -> Result<Vec<tasks::Task>, String> {
    let status = tasks::Status::parse(&status).ok_or_else(|| format!("Unknown status: {}", status))?;
    tasks::Board::update(|b| b.set_status(id, status).map(|_| b.tasks.clone()))
}

#[tauri::command]
fn delete_task(id: u64) -> Result<Vec<tasks::Task>, String> {
    tasks::Board::update(|b| b.delete(id).map(|_| b.tasks.clone()))
}

// ── Setup ─────────────────────────────────────────────────────────────────────

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("courriel=debug".parse().unwrap()),
        )
        .init();

    info!("Starting Le courriel (Tauri)");
    match paths::init_data_dir() {
        Ok(dir) => info!("Data directory: {}", dir.display()),
        Err(e) => tracing::warn!("Could not use the data directory ({}); using the current one", e),
    }

    // A broken config.toml must not stop the app from opening: start empty and show why.
    let (mut cfg, config_error) = match Config::load() {
        Ok(cfg) => (cfg, None),
        Err(e) => {
            tracing::error!("Could not load config.toml: {:#}", e);
            (Config::default(), Some(format!("{:#}", e)))
        }
    };
    // The old single signature.html becomes the first account's signature.
    if config_error.is_none() && signature::migrate_legacy(&signature::data_dir(), &mut cfg) {
        match config::save_config(&cfg) {
            Ok(()) => info!("Signature moved to the first account"),
            Err(e) => tracing::warn!("Could not save the migrated signature: {:#}", e),
        }
    }
    i18n::set_lang(&cfg.user.language);
    cloud::configure(&cfg);
    let cfg = Arc::new(cfg);

    let shared: SharedState = Arc::new(Mutex::new(AppState {
        cfg: cfg.clone(),
        emails: vec![],
        search_index: vec![],
        trash_folder: "Trash".into(),
        spam_folder: "Junk".into(),
        sent_folders: Vec::new(),
        startup_error: config_error,
    }));

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(shared.clone())
        .invoke_handler(tauri::generate_handler![
            app_ready,
            refresh_emails,
            get_email,
            send_email,
            get_settings,
            save_settings,
            get_qr,
            save_project_description,
            add_project,
            trash_email,
            spam_email,
            not_spam_email,
            set_read,
            set_email_project,
            save_email,
            signature_import,
            signature_get,
            compose_accounts,
            ai_list_models,
            ai_test,
            account_test_imap,
            account_test_smtp,
            lookup_mail_settings,
            scan_mail_clients,
            search_local,
            search_server,
            get_search_email,
            ai_assist,
            cloud_status,
            set_cloud,
            cloud_sign_in,
            cloud_sign_up,
            cloud_sign_out,
            cloud_sync_now,
            search_gifs,
            ai_compose,
            dismiss_item,
            create_task,
            set_task_status,
            delete_task,
        ])
        .setup(|_app| Ok(()))
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
