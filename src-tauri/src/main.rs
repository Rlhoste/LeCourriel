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

mod ai_classifier;
mod ai_writer;
mod app_state;
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
mod pgsync;
mod settings;
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
    let cfg = { shared.lock().unwrap().cfg.clone() };
    let shared_clone = shared.inner().clone();
    // Send cache immediately (fast, synchronous read)
    app_state::load_and_display_cache(&cfg, &app, &shared_clone);
    tauri::async_runtime::spawn(async move {
        let cfg = sync_from_database(&app, &shared_clone).await.unwrap_or(cfg);
        // Kick off live refresh in background
        app_state::refresh(cfg, app, shared_clone).await;
    });
    Ok(())
}

/// Startup sync with the user's PostgreSQL. Returns the reloaded config when
/// anything was pulled (preferences may have changed), None otherwise.
async fn sync_from_database(app: &AppHandle, shared: &SharedState) -> Option<Arc<Config>> {
    use tauri::Emitter;
    if !pgsync::is_configured() {
        return None;
    }
    match pgsync::pull_all().await {
        Ok(summary) => {
            info!("Database sync: {}", summary);
            let cfg = Arc::new(Config::load().ok()?);
            i18n::set_lang(&cfg.user.language);
            shared.lock().unwrap().cfg = cfg.clone();
            // Re-show the cache: tasks or dismissals may have come from another computer.
            app_state::load_and_display_cache(&cfg, app, shared);
            Some(cfg)
        }
        Err(e) => {
            tracing::warn!("Database sync failed: {:#}", e);
            app.emit("error", format!("Database sync failed: {:#}", e)).ok();
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
    shared: State<'_, SharedState>,
) -> Result<(), String> {
    let (cfg, sent_folder) = {
        let s = shared.lock().unwrap();
        (s.cfg.clone(), s.sent_folder.clone())
    };
    let account = cfg.accounts.first().ok_or("No accounts configured")?;
    let mail = smtp_sender::OutgoingMail { to, cc, bcc, subject, html, in_reply_to };
    let raw = smtp_sender::send_message(&account.smtp, &mail)
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
    display_name: String,
    accounts: Vec<dto::AccountDto>,
    claude_key: String,
    claude_model: String,
    giphy_key: Option<String>,
    font_scale: f32,
    theme: Option<String>,
    language: Option<String>,
    shared: State<'_, SharedState>,
) -> Result<(), String> {
    let giphy_key = giphy_key.unwrap_or_default();
    settings::save_config(&display_name, &accounts, &claude_key, &claude_model, &giphy_key, font_scale, theme.as_deref(), language.as_deref())
        .map_err(|e| e.to_string())?;
    // Reload config into shared state.
    match Config::load() {
        Ok(new_cfg) => {
            i18n::set_lang(&new_cfg.user.language);
            pgsync::configure(&new_cfg);
            let mut s = shared.lock().unwrap();
            s.cfg = Arc::new(new_cfg);
        }
        Err(e) => return Err(e.to_string()),
    }
    Ok(())
}

// ── PostgreSQL sync ────────────────────────────────────────────────────────

/// Try a connection without saving anything. Returns e.g. "PostgreSQL 16.4".
#[tauri::command]
async fn test_database(url: String, accept_invalid_certs: bool) -> Result<String, String> {
    if url.trim().is_empty() {
        return Err("Enter a PostgreSQL URL first".into());
    }
    pgsync::test_connection(url.trim(), accept_invalid_certs)
        .await
        .map_err(|e| format!("{:#}", e))
}

/// Save (or clear, with an empty URL) the database settings, then sync right away.
#[tauri::command]
async fn set_database(
    url: String,
    accept_invalid_certs: bool,
    app: AppHandle,
    shared: State<'_, SharedState>,
) -> Result<(), String> {
    let mut cfg = (*shared.lock().unwrap().cfg).clone();
    let url = url.trim().to_string();
    cfg.database = if url.is_empty() {
        None
    } else {
        Some(config::DatabaseConfig { url, accept_invalid_certs })
    };
    // Local only: the database settings are never synced.
    config::write_local(&cfg).map_err(|e| e.to_string())?;
    pgsync::configure(&cfg);
    shared.lock().unwrap().cfg = Arc::new(cfg);
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
    shared.lock().unwrap().emails.retain(|e| e.uid != uid);
    app_state::remove_from_cache(uid);
    Ok(())
}

/// Move an inbox email to the Trash folder on the server.
#[tauri::command]
async fn trash_email(uid: u32, shared: State<'_, SharedState>) -> Result<(), String> {
    let folder = shared.lock().unwrap().trash_folder.clone();
    move_email(shared.inner(), uid, folder).await
}

/// Move an inbox email to the Spam/Junk folder on the server.
#[tauri::command]
async fn spam_email(uid: u32, shared: State<'_, SharedState>) -> Result<(), String> {
    let folder = shared.lock().unwrap().spam_folder.clone();
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
    if let Some(c) = shared.lock().unwrap().emails.iter_mut().find(|e| e.uid == uid) {
        c.is_read = read;
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
    if let Some(c) = shared.lock().unwrap().emails.iter_mut().find(|e| e.uid == uid) {
        c.project = project.clone();
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
    let key = shared.lock().unwrap().cfg.gifs.giphy_api_key.clone();
    gifs::search(&key, &query).await.map_err(|e| format!("{:#}", e))
}

#[tauri::command]
async fn fetch_gif(url: String) -> Result<String, String> {
    gifs::fetch_as_data_uri(&url).await.map_err(|e| format!("{:#}", e))
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

    let cfg = Config::load().expect("Failed to load config");
    i18n::set_lang(&cfg.user.language);
    pgsync::configure(&cfg);
    let cfg = Arc::new(cfg);

    let shared: SharedState = Arc::new(Mutex::new(AppState {
        cfg: cfg.clone(),
        emails: vec![],
        trash_folder: "Trash".into(),
        spam_folder: "Junk".into(),
        sent_folder: "Sent".into(),
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
            set_read,
            set_email_project,
            save_email,
            test_database,
            set_database,
            search_gifs,
            ai_compose,
            fetch_gif,
            dismiss_item,
            create_task,
            set_task_status,
            delete_task,
        ])
        .setup(|_app| Ok(()))
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
