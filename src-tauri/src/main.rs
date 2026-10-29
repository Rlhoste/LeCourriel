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
// Tauri commands take their arguments by name from the UI; Priority's variant names are
// saved in caches and sent to the UI; `sort_by(|a, b| b.cmp(a))` reads clearer than `Reverse`.
#![allow(clippy::too_many_arguments, clippy::enum_variant_names, clippy::module_inception, clippy::unnecessary_sort_by)]

use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};
use tracing::info;

mod ai;
mod ai_assistant;
mod ai_classifier;
mod ai_project;
mod ai_writer;
mod app_state;
mod attachments;
mod auto_scan;
mod auto_setup;
mod claude_code;
mod cloud;
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
mod settings;
mod signature;
mod smtp_sender;
mod state;
mod tasks;
#[cfg(test)]
mod tests;

use config::Config;
use state::AppState;

type SharedState = Arc<Mutex<AppState>>;

#[derive(serde::Serialize)]
struct UpdateInfo {
    current_version: String,
    available: bool,
    version: String,
    notes: String,
    date: String,
}

// ── Commands ─────────────────────────────────────────────────────────────────

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

/// Opens a composer in its own window, so several emails can be written at once.
/// `args` are the composer's options (mode, email, to, subject…), handed to the new page.
/// Async: creating a window from a synchronous command can deadlock on Windows.
#[tauri::command]
async fn open_compose_window(app: AppHandle, args: serde_json::Value, title: String) -> Result<(), String> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let label = format!("compose-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let script = format!("window.__COMPOSE__ = {};", args);
    tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::App("index.html".into()))
        .title(title)
        .inner_size(760.0, 640.0)
        .min_inner_size(420.0, 360.0)
        .disable_drag_drop_handler()
        .initialization_script(&script)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn check_for_update(app: AppHandle) -> Result<UpdateInfo, String> {
    use tauri_plugin_updater::UpdaterExt;
    let current_version = app.package_info().version.to_string();
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    Ok(match update {
        Some(update) => UpdateInfo {
            current_version,
            available: true,
            version: update.version,
            notes: update.body.unwrap_or_default(),
            date: update.date.map(|date| date.to_string()).unwrap_or_default(),
        },
        None => UpdateInfo {
            current_version,
            available: false,
            version: String::new(),
            notes: String::new(),
            date: String::new(),
        },
    })
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("No update is available")?;
    let progress_app = app.clone();
    let finished_app = app.clone();
    let mut downloaded = 0u64;
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                progress_app
                    .emit(
                        "update-progress",
                        serde_json::json!({
                            "downloaded": downloaded,
                            "total": total,
                        }),
                    )
                    .ok();
            },
            move || {
                finished_app
                    .emit(
                        "update-progress",
                        serde_json::json!({
                            "finished": true,
                        }),
                    )
                    .ok();
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    app.restart()
}

#[tauri::command]
async fn app_ready(app: AppHandle, shared: State<'_, SharedState>) -> Result<(), String> {
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
        app_state::refresh(cfg, app.clone(), shared_clone.clone()).await;
        start_auto_refresh(app, shared_clone);
    });
    Ok(())
}

/// Checks for new mail every 2 minutes, with the current settings. Started once, even if
/// the page is reloaded and calls app_ready again.
fn start_auto_refresh(app: AppHandle, shared: SharedState) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(120));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await; // the first tick is immediate; the startup refresh just ran
        loop {
            tick.tick().await;
            let cfg = shared.lock().unwrap().cfg.clone();
            if !cfg.accounts.is_empty() {
                app_state::refresh(cfg, app.clone(), shared.clone()).await;
            }
        }
    });
}

/// The number of unread inbox emails, shown on the app icon: a number on Linux and macOS,
/// a red dot on Windows (it has no number badge), and "(n)" in the window title everywhere.
#[tauri::command]
fn set_unread_count(app: AppHandle, count: u32) -> Result<(), String> {
    use tauri::Manager;
    let Some(win) = app.get_webview_window("main") else { return Ok(()) };
    let title = if count > 0 { format!("Le courriel ({count})") } else { "Le courriel".to_string() };
    win.set_title(&title).map_err(|e| e.to_string())?;
    #[cfg(target_os = "windows")]
    {
        let dot = if count > 0 {
            Some(tauri::image::Image::from_bytes(include_bytes!("../icons/badge.png")).map_err(|e| e.to_string())?)
        } else {
            None
        };
        win.set_overlay_icon(dot).map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Not every Linux desktop shows badges; the title still carries the count.
        let _ = win.set_badge_count(if count > 0 { Some(count as i64) } else { None });
    }
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
async fn refresh_emails(app: AppHandle, shared: State<'_, SharedState>) -> Result<(), String> {
    let cfg = {
        let s = shared.lock().unwrap();
        s.cfg.clone()
    };
    let shared_clone = shared.inner().clone();
    app_state::refresh(cfg, app, shared_clone).await;
    Ok(())
}

#[tauri::command]
fn get_email(uid: u64, shared: State<'_, SharedState>) -> Option<dto::EmailCardDto> {
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
    // Paths of the files attached to the message (#43, #44).
    attachments: Option<Vec<String>>,
    shared: State<'_, SharedState>,
) -> Result<(), String> {
    // The account chosen in the composer's "From" (the first one by default).
    let index = account.unwrap_or(0);
    let (cfg, sent_folder) = {
        let s = shared.lock().unwrap();
        (s.cfg.clone(), s.sent_folders.get(index).cloned().unwrap_or_else(|| "Sent".into()))
    };
    let account = cfg.accounts.get(index).ok_or("No accounts configured")?;
    let mail = smtp_sender::OutgoingMail {
        to, cc, bcc, subject, html, in_reply_to,
        attachments: attachments.unwrap_or_default(),
    };
    let smtp = account.smtp_for_sending(&cfg.user_name());
    let raw = smtp_sender::send_message(&smtp, &mail).await.map_err(|e| format!("{:#}", e))?;
    // The SMTP relay doesn't keep a copy, so store one in Sent ourselves.
    if let Err(e) = imap_client::append_sent(&account.imap, &sent_folder, &raw).await {
        tracing::warn!("Could not save a copy to {}: {:#}", sent_folder, e);
    }
    Ok(())
}

/// What the attach picker chose: files as they are, each folder as one `FolderName.zip`.
#[tauri::command]
async fn prepare_attachments(paths: Vec<String>) -> Result<Vec<attachments::AttachmentInfo>, String> {
    // Async: a big folder is zipped off the main thread, the window doesn't freeze.
    attachments::prepare(&paths).map_err(|e| format!("{:#}", e))
}

/// What was dropped on the editor or the reply bar: the webview gives no disk path for a
/// dropped file, so the bytes arrive base64-encoded and are written to a temp folder here.
#[tauri::command]
async fn save_dropped_attachments(items: Vec<attachments::DroppedItem>) -> Result<Vec<attachments::AttachmentInfo>, String> {
    attachments::save_dropped(&items).map_err(|e| format!("{:#}", e))
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
    notify_new_mail: Option<bool>,
    discussion_mode: Option<bool>,
    app: AppHandle,
    shared: State<'_, SharedState>,
) -> Result<(), String> {
    let giphy_key = giphy_key.unwrap_or_default();
    let old_accounts = shared.lock().unwrap().cfg.accounts.clone();
    settings::save_config(
        display_name.as_deref(),
        &accounts,
        &ai,
        &giphy_key,
        font_scale,
        theme.as_deref(),
        language.as_deref(),
        notify_new_mail,
        discussion_mode,
    )
    .map_err(|e| e.to_string())?;
    // Reload config into shared state.
    match Config::load() {
        Ok(new_cfg) => {
            signature::remove_unused(&signature::data_dir(), &new_cfg);
            i18n::set_lang(&new_cfg.user.language);
            cloud::configure(&new_cfg);
            let new_cfg = Arc::new(new_cfg);
            shared.lock().unwrap().cfg = new_cfg.clone();
            // An account is the same one while its server and login are: its emails keep
            // their tasks; a removed account's emails and tasks go with it.
            // (A login set up twice, e.g. for two mailboxes, pairs in order.)
            let mut new_index: Vec<Option<usize>> = Vec::new();
            for old in &old_accounts {
                let found = new_cfg.accounts.iter().enumerate().position(|(i, a)| {
                    !new_index.contains(&Some(i))
                        && a.imap.host.eq_ignore_ascii_case(&old.imap.host)
                        && a.imap.username.eq_ignore_ascii_case(&old.imap.username)
                });
                new_index.push(found);
            }
            if new_index.iter().enumerate().any(|(i, n)| *n != Some(i)) {
                app_state::forget_accounts(&new_index, &new_cfg, &app, shared.inner());
            }
            // Apply the new settings straight away: the first account on a new computer
            // fetches its mail, a newly set up AI sorts the emails waiting for it.
            if !new_cfg.accounts.is_empty() {
                let shared = shared.inner().clone();
                tauri::async_runtime::spawn(async move {
                    app_state::refresh(new_cfg, app.clone(), shared.clone()).await;
                    start_auto_refresh(app, shared);
                });
            }
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
    cfg.accounts
        .iter()
        .map(|a| {
            let from = a.smtp_for_sending(&cfg.user_name()).from_address;
            dto::ComposeAccountDto {
                from: if from.trim().is_empty() { a.name.clone() } else { from },
                signature_html: signature::load(&dir, &a.signature.file),
                signature_on_new: a.signature.on_new,
                signature_on_reply: a.signature.on_reply,
            }
        })
        .collect()
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
    ai::list_models(&ai_config_from(&provider, "", &base_url, &api_key)).await.map_err(|e| format!("{:#}", e))
}

/// Settings → AI → Claude (subscription): is Claude Code installed and signed in?
#[tauri::command]
async fn claude_code_status() -> claude_code::Status {
    claude_code::status().await
}

/// Settings → AI → "Sign in with Claude": starts Claude Code's browser sign-in.
#[tauri::command]
fn claude_code_sign_in() -> Result<(), String> {
    claude_code::sign_in().map_err(|e| format!("{}", e))
}

/// Settings → AI → "Sign out": signs Claude Code out on this computer.
#[tauri::command]
async fn claude_code_sign_out() -> Result<(), String> {
    claude_code::sign_out().await.map_err(|e| format!("{}", e))
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
    let answer = ai::complete(
        &cfg,
        ai::Request {
            system: "You are a connectivity check. Reply with the single word OK.",
            user: "Reply with OK.",
            max_tokens: 20,
            json: false,
        },
    )
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
    if cfg.accounts.is_empty() {
        return Err("No accounts configured".into());
    }
    let (terms, filters) = search::parse_query(&query);
    if terms.is_empty() && filters.after.is_none() {
        return Err("Type at least one word to search on the server".into());
    }
    let mut raws = Vec::new();
    let mut failed = Vec::new();
    for (index, account) in cfg.accounts.iter().enumerate() {
        // One unreachable account keeps the other accounts' results.
        match imap_client::search_server(&account.imap, &terms, filters.after, 20).await {
            Ok(found) => raws.extend(found.into_iter().map(|mut r| {
                r.uid = models::mail_id(index, r.uid as u32);
                r
            })),
            Err(e) => failed.push(format!("{}: {:#}", account.name, e)),
        }
    }
    if failed.len() == cfg.accounts.len() {
        return Err(failed.join("\n"));
    }
    let mut s = shared.lock().unwrap();
    let cached: std::collections::HashSet<u64> =
        s.search_index.iter().filter(|(f, _)| *f == search::Folder::Inbox).map(|(_, c)| c.uid).collect();
    s.search_index.retain(|(f, _)| *f != search::Folder::Server);
    let mut hits = Vec::new();
    for raw in raws {
        if cached.contains(&raw.uid) {
            continue; // already in the local results
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
            if let Some(e) = find(&hit.id) {
                picked.push(e);
            }
        }
        // Few keyword matches (natural-language request): add the most recent emails.
        if picked.len() < 8 {
            let mut recent: Vec<&(search::Folder, models::EmailCard)> = index
                .iter()
                .filter(|(f, _)| matches!(f, search::Folder::Inbox | search::Folder::Sent | search::Folder::Server))
                .collect();
            recent.sort_by(|a, b| b.1.date_ts.cmp(&a.1.date_ts));
            for e in recent {
                if picked.len() >= 25 {
                    break;
                }
                if !picked.iter().any(|p| p.0 == e.0 && p.1.uid == e.1.uid) {
                    picked.push(e);
                }
            }
        }
    }
    let candidates: Vec<ai_assistant::Candidate> = picked
        .iter()
        .map(|(f, c)| ai_assistant::Candidate {
            id: search::make_id(*f, c.uid),
            date: if c.date_full.is_empty() { c.date_str.clone() } else { c.date_full.clone() },
            from: if c.from_email.is_empty() {
                c.from_name.clone()
            } else {
                format!("{} <{}>", c.from_name, c.from_email)
            },
            to: c.to.clone(),
            subject: c.subject.clone(),
            summary: c.summary.clone(),
            body: c.body_text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(700).collect(),
        })
        .collect();
    let contacts = search::contacts(&index, 60);
    let today = chrono::Local::now().format("%Y-%m-%d (%A)").to_string();
    ai_assistant::assist(&cfg, &query, &mode, &candidates, &contacts, &today).await.map_err(|e| format!("{:#}", e))
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
    cfg.supabase =
        if url.is_empty() && anon_key.is_empty() { None } else { Some(config::SupabaseConfig { url, anon_key }) };
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
    app: AppHandle,
    shared: State<'_, SharedState>,
) -> Result<usize, String> {
    let mut cfg = {
        let s = shared.lock().unwrap();
        (*s.cfg).clone()
    };
    if let Some(p) = cfg.projects.iter_mut().find(|p| p.name == name) {
        p.description = description;
    }
    config::save_config(&cfg).map_err(|e| e.to_string())?;
    shared.lock().unwrap().cfg = Arc::new(cfg);
    // A better description finds more of the emails already there.
    file_existing_emails(&app, shared.inner(), &name).await
}

#[tauri::command]
async fn add_project(
    sentence: String,
    app: AppHandle,
    shared: State<'_, SharedState>,
) -> Result<dto::ProjectDto, String> {
    let sentence = sentence.split_whitespace().collect::<Vec<_>>().join(" ");
    if sentence.is_empty() {
        return Err("Project description cannot be empty".into());
    }
    if sentence.chars().count() > 500 {
        return Err("Project description must be 500 characters or fewer".into());
    }

    // The AI reads the emails the request is about, to make sense of it and find
    // the people and keywords of the project.
    let (ai_cfg, me, related) = {
        let s = shared.lock().unwrap();
        let mut me = vec![s.cfg.user_name()];
        for a in &s.cfg.accounts {
            me.push(a.imap.username.clone());
            me.push(a.smtp.from_address.clone());
        }
        me.retain(|m| !m.trim().is_empty());
        me.dedup();
        (s.cfg.ai.clone(), me.join(", "), ai_project::related_emails(&s.search_index, &sentence, 12))
    };
    let ai_project::ProjectDraft { name, description, urgency } = ai_project::draft(&ai_cfg, &sentence, &me, &related)
        .await
        .map_err(|e| format!("The AI could not prepare the project: {:#}", e))?;

    let mut cfg = {
        let s = shared.lock().unwrap();
        (*s.cfg).clone()
    };
    if cfg.projects.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
        return Err(format!("A project named \"{}\" already exists", name));
    }
    cfg.projects.push(config::ProjectConfig {
        name: name.clone(),
        description: description.clone(),
        urgency: urgency.clone(),
    });
    config::save_config(&cfg).map_err(|e| e.to_string())?;
    shared.lock().unwrap().cfg = Arc::new(cfg);
    // The project is kept even if filing fails; the warning says why it is empty.
    let (email_count, warning) = match file_existing_emails(&app, shared.inner(), &name).await {
        Ok(n) => (n as i32, None),
        Err(e) => (0, Some(e)),
    };
    Ok(dto::ProjectDto { name, urgency, description, email_count, warning })
}

#[tauri::command]
fn rename_project(name: String, new_name: String, shared: State<'_, SharedState>) -> Result<String, String> {
    let new_name = new_name.split_whitespace().collect::<Vec<_>>().join(" ");
    if new_name.is_empty() {
        return Err("Project name cannot be empty".into());
    }
    let mut cfg = {
        let s = shared.lock().unwrap();
        (*s.cfg).clone()
    };
    let project =
        cfg.projects.iter().position(|p| p.name == name).ok_or_else(|| format!("Unknown project: {}", name))?;
    if cfg.projects.iter().enumerate().any(|(i, p)| i != project && p.name.eq_ignore_ascii_case(&new_name)) {
        return Err(format!("A project named \"{}\" already exists", new_name));
    }
    cfg.projects[project].name = new_name.clone();
    config::save_config(&cfg).map_err(|e| e.to_string())?;

    overrides::Overrides::update(|o| {
        for project in o.projects.values_mut() {
            if project == &name {
                *project = new_name.clone();
            }
        }
    });
    tasks::Board::update(|b| {
        for task in &mut b.tasks {
            if task.project.as_deref() == Some(name.as_str()) {
                task.project = Some(new_name.clone());
            }
        }
    });

    let mut s = shared.lock().unwrap();
    s.cfg = Arc::new(cfg);
    for email in &mut s.emails {
        if email.project.as_deref() == Some(name.as_str()) {
            email.project = Some(new_name.clone());
        }
    }
    for (_, email) in &mut s.search_index {
        if email.project.as_deref() == Some(name.as_str()) {
            email.project = Some(new_name.clone());
        }
    }
    Ok(new_name)
}

#[tauri::command]
fn delete_project(name: String, shared: State<'_, SharedState>) -> Result<(), String> {
    let mut cfg = {
        let s = shared.lock().unwrap();
        (*s.cfg).clone()
    };
    let old_len = cfg.projects.len();
    cfg.projects.retain(|p| p.name != name);
    if cfg.projects.len() == old_len {
        return Err(format!("Unknown project: {}", name));
    }
    config::save_config(&cfg).map_err(|e| e.to_string())?;

    overrides::Overrides::update(|o| {
        for project in o.projects.values_mut() {
            if project == &name {
                project.clear();
            }
        }
    });
    tasks::Board::update(|b| {
        for task in &mut b.tasks {
            if task.project.as_deref() == Some(name.as_str()) {
                task.project = None;
            }
        }
    });

    let mut s = shared.lock().unwrap();
    s.cfg = Arc::new(cfg);
    for email in &mut s.emails {
        if email.project.as_deref() == Some(name.as_str()) {
            email.project = None;
        }
    }
    for (_, email) in &mut s.search_index {
        if email.project.as_deref() == Some(name.as_str()) {
            email.project = None;
        }
    }
    Ok(())
}

/// The account an email id belongs to (as of the refresh that fetched the email, so an
/// account removed or reordered since cannot catch it), its folders, and the IMAP UID.
fn account_of(
    shared: &SharedState,
    id: u64,
) -> Result<(config::AccountConfig, app_state::SpecialFolders, u32), String> {
    let (index, uid) = models::split_mail_id(id);
    let s = shared.lock().unwrap();
    let found = match s.mailboxes.get(index) {
        Some(m) => Some(m.clone()),
        // Emails shown from the cache before the first refresh: the settings' accounts.
        None if s.mailboxes.is_empty() => s.cfg.accounts.get(index).map(|a| {
            (
                a.clone(),
                app_state::SpecialFolders {
                    sent: "Sent".into(),
                    drafts: "Drafts".into(),
                    trash: "Trash".into(),
                    spam: None,
                    archive: None,
                },
            )
        }),
        None => None,
    };
    let (account, folders) = found.ok_or("This email's account was removed")?;
    Ok((account, folders, uid))
}

/// The spam folder name; "Junk" when the server has none yet (created when first needed).
fn spam_name(folders: &app_state::SpecialFolders) -> String {
    folders.spam.clone().unwrap_or_else(|| "Junk".into())
}

/// The IMAP mailbox behind one of the app's folders ("inbox", "sent", "drafts", "trash" or
/// "spam"; anything else is the inbox). Each folder numbers its UIDs apart.
fn mailbox_of(account: &config::AccountConfig, folders: &app_state::SpecialFolders, folder: &str) -> String {
    match folder {
        "sent" => folders.sent.clone(),
        "drafts" => folders.drafts.clone(),
        "trash" => folders.trash.clone(),
        "spam" => spam_name(folders),
        _ => account.imap.mailbox.clone(),
    }
}

/// Move an email of one of the app's folders (`from`, as in `mailbox_of`) to the mailbox
/// `to`, then forget it locally.
async fn move_email(shared: &SharedState, uid: u64, from: &str, to: String) -> Result<(), String> {
    let (account, folders, imap_uid) = account_of(shared, uid)?;
    let source = config::ImapConfig { mailbox: mailbox_of(&account, &folders, from), ..account.imap.clone() };
    imap_client::move_message(&source, imap_uid, &to).await.map_err(|e| format!("{:#}", e))?;
    let folder = search::Folder::parse(from).unwrap_or(search::Folder::Inbox);
    {
        let mut s = shared.lock().unwrap();
        s.search_index.retain(|(f, e)| !(*f == folder && e.uid == uid));
        if folder == search::Folder::Inbox {
            s.emails.retain(|e| e.uid != uid);
        }
    }
    if folder == search::Folder::Inbox {
        app_state::remove_from_cache(uid);
    }
    Ok(())
}

/// Move an email to the Trash folder on the server: from the inbox, or from `folder`.
#[tauri::command]
async fn trash_email(uid: u64, folder: Option<String>, shared: State<'_, SharedState>) -> Result<(), String> {
    let (_, folders, _) = account_of(shared.inner(), uid)?;
    move_email(shared.inner(), uid, folder.as_deref().unwrap_or("inbox"), folders.trash).await
}

/// Move an email to the Archive folder (created the first time if the server has none):
/// from the inbox, or from `folder`.
#[tauri::command]
async fn archive_email(uid: u64, folder: Option<String>, shared: State<'_, SharedState>) -> Result<(), String> {
    let (account, folders, _) = account_of(shared.inner(), uid)?;
    let archive = folders.archive.unwrap_or_else(|| "Archive".into());
    imap_client::ensure_folder(&account.imap, &archive).await.map_err(|e| format!("{:#}", e))?;
    move_email(shared.inner(), uid, folder.as_deref().unwrap_or("inbox"), archive).await
}

/// "Not spam": move an email from the Spam/Junk folder back to the inbox.
#[tauri::command]
async fn not_spam_email(uid: u64, shared: State<'_, SharedState>) -> Result<(), String> {
    let (account, folders, imap_uid) = account_of(shared.inner(), uid)?;
    // move_message moves from the configured mailbox: point it at the spam folder.
    let from_spam = config::ImapConfig { mailbox: spam_name(&folders), ..account.imap.clone() };
    imap_client::move_message(&from_spam, imap_uid, &account.imap.mailbox).await.map_err(|e| format!("{:#}", e))?;
    // Spam UIDs have their own numbering: forget only the spam entry. The email comes
    // back in the inbox at the next refresh.
    let mut s = shared.lock().unwrap();
    s.search_index.retain(|(f, e)| !(*f == search::Folder::Spam && e.uid == uid));
    Ok(())
}

/// Move an inbox email to the Spam/Junk folder on the server.
#[tauri::command]
async fn spam_email(uid: u64, shared: State<'_, SharedState>) -> Result<(), String> {
    let (account, folders, _) = account_of(shared.inner(), uid)?;
    let folder = spam_name(&folders);
    // Many mailboxes have no spam folder: create it the first time, like other mail clients.
    imap_client::ensure_folder(&account.imap, &folder).await.map_err(|e| format!("{:#}", e))?;
    move_email(shared.inner(), uid, "inbox", folder).await
}

/// Mark an inbox email read or unread, on the server and locally.
#[tauri::command]
async fn set_read(uid: u64, read: bool, shared: State<'_, SharedState>) -> Result<(), String> {
    let (account, _, imap_uid) = account_of(shared.inner(), uid)?;
    imap_client::set_seen(&account.imap, imap_uid, read).await.map_err(|e| format!("{:#}", e))?;
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
fn set_email_project(uid: u64, project: Option<String>, shared: State<'_, SharedState>) -> Result<(), String> {
    let project = project.filter(|p| !p.is_empty());
    if let Some(p) = &project {
        if !shared.lock().unwrap().cfg.projects.iter().any(|cp| &cp.name == p) {
            return Err(format!("Unknown project: {}", p));
        }
    }
    overrides::Overrides::update(|o| o.set_project(uid, project.clone()));
    set_project_in_memory(shared.inner(), &[uid], project.as_deref());
    Ok(())
}

/// Show inbox emails in `project` (None = no project): in memory and on their tasks.
fn set_project_in_memory(shared: &SharedState, uids: &[u64], project: Option<&str>) {
    let project = project.map(str::to_string);
    {
        let mut s = shared.lock().unwrap();
        for c in s.emails.iter_mut().filter(|e| uids.contains(&e.uid)) {
            c.project = project.clone();
        }
        for (_, c) in s.search_index.iter_mut().filter(|(f, e)| *f == search::Folder::Inbox && uids.contains(&e.uid)) {
            c.project = project.clone();
        }
    }
    tasks::Board::update(|b| {
        for t in b.tasks.iter_mut().filter(|t| t.source_uid.is_some_and(|u| uids.contains(&u))) {
            t.project = project.clone();
        }
    });
}

/// File the inbox emails that belong to a project, chosen by the AI from its description.
/// Emails sorted before the project existed would otherwise never be in it. Emails the
/// user placed by hand stay where they are. Returns how many emails the project now has.
async fn file_existing_emails(app: &AppHandle, shared: &SharedState, name: &str) -> Result<usize, String> {
    let (cfg, inbox) = {
        let s = shared.lock().unwrap();
        let inbox: Vec<models::EmailCard> =
            s.search_index.iter().filter(|(f, _)| *f == search::Folder::Inbox).map(|(_, c)| c.clone()).collect();
        (s.cfg.clone(), inbox)
    };
    let Some(project) = cfg.projects.iter().find(|p| p.name == name) else { return Ok(0) };
    let manual = overrides::Overrides::load().projects;
    let candidates: Vec<models::EmailCard> = inbox.iter().filter(|c| !manual.contains_key(&c.uid)).cloned().collect();
    let uids = ai_project::matching_emails(&cfg.ai, &project.name, &project.description, &candidates)
        .await
        .map_err(|e| format!("The AI could not file the emails into the project: {:#}", e))?;
    tracing::info!("Project {}: {} existing email(s) filed", name, uids.len());
    if !uids.is_empty() {
        // Kept like a choice made by hand: a refresh rewrites the cache, not the overrides.
        overrides::Overrides::update(|o| {
            for &uid in &uids {
                o.set_project(uid, Some(name.to_string()));
            }
        });
        set_project_in_memory(shared, &uids, Some(name));
    }
    app_state::redisplay(&cfg, app, shared);
    let count = inbox.iter().filter(|c| uids.contains(&c.uid) || c.project.as_deref() == Some(name)).count();
    Ok(count)
}

/// Save the original message as an .eml file at `path`. `folder` is the list it was opened
/// from ("inbox", "sent", "drafts", "trash" or "spam"): each folder numbers its UIDs apart.
#[tauri::command]
async fn save_email(uid: u64, folder: String, path: String, shared: State<'_, SharedState>) -> Result<(), String> {
    let raw = fetch_raw(shared.inner(), uid, &folder).await?;
    std::fs::write(&path, raw).map_err(|e| format!("Could not write {}: {}", path, e))
}

/// The whole message as the server keeps it. `folder` as in `mailbox_of`.
async fn fetch_raw(shared: &SharedState, uid: u64, folder: &str) -> Result<Vec<u8>, String> {
    let (account, folders, imap_uid) = account_of(shared, uid)?;
    let from = config::ImapConfig { mailbox: mailbox_of(&account, &folders, folder), ..account.imap.clone() };
    imap_client::fetch_raw(&from, imap_uid).await.map_err(|e| format!("{:#}", e))
}

/// Every header of the original message, as written ("Show original").
#[tauri::command]
async fn email_headers(uid: u64, folder: String, shared: State<'_, SharedState>) -> Result<String, String> {
    let raw = fetch_raw(shared.inner(), uid, &folder).await?;
    Ok(models::header_block(&raw))
}

/// Hide a task, deadline or meeting item without touching the email.
#[tauri::command]
fn dismiss_item(kind: String, uid: u64) -> Result<(), String> {
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
    ai_writer::run(&cfg, &action, &html, original).await.map_err(|e| format!("{:#}", e))
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
fn create_task(
    title: String,
    project: Option<String>,
    due_date: Option<String>,
    source_uid: Option<u64>,
    shared: State<'_, SharedState>,
) -> Result<Vec<tasks::Task>, String> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("Task title cannot be empty".into());
    }
    let project = project.filter(|p| !p.is_empty());
    let due_date = due_date.filter(|date| !date.is_empty());
    if due_date.as_deref().is_some_and(|date| chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err()) {
        return Err("Invalid task date".into());
    }
    // Made from an inbox email ("Add as task"): the task links back to it.
    let source = source_uid.and_then(|uid| shared.lock().unwrap().emails.iter().find(|e| e.uid == uid).cloned());
    Ok(tasks::Board::update(|b| {
        match &source {
            Some(c) => b.add_from_email(title, project, due_date, c),
            None => b.add_dated(title, project, due_date),
        };
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
            tracing_subscriber::EnvFilter::from_default_env().add_directive("courriel=debug".parse().unwrap()),
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
        mailboxes: Vec::new(),
        sent_folders: Vec::new(),
        briefing: None,
        startup_error: config_error,
    }));

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(shared.clone())
        .invoke_handler(tauri::generate_handler![
            app_ready,
            app_version,
            open_compose_window,
            set_unread_count,
            check_for_update,
            install_update,
            refresh_emails,
            get_email,
            send_email,
            prepare_attachments,
            save_dropped_attachments,
            get_settings,
            save_settings,
            get_qr,
            save_project_description,
            add_project,
            rename_project,
            delete_project,
            trash_email,
            archive_email,
            email_headers,
            spam_email,
            not_spam_email,
            set_read,
            set_email_project,
            save_email,
            signature_import,
            signature_get,
            compose_accounts,
            ai_list_models,
            claude_code_status,
            claude_code_sign_in,
            claude_code_sign_out,
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
