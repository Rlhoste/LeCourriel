// Courriel — an AI-assisted email client.
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

use anyhow::Result;
use chrono::Timelike;
use reqwest;
use slint::{Model, SharedString};
use std::sync::Arc;
use tracing::info;

mod ai_classifier;
mod app_state;
mod config;
mod imap_client;
mod models;
mod settings;
mod smtp_sender;

slint::include_modules!();

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("courriel=debug".parse()?),
        )
        .init();

    if std::env::args().any(|a| a == "--check-config") {
        return check_config().await;
    }

    info!("Starting Courriel");

    let cfg = Arc::new(config::Config::load()?);

    let app = AppWindow::new()?;

    // Set greeting.
    let greeting = {
        let hour = chrono::Local::now().hour();
        let time_word = if hour < 12 {
            "morning"
        } else if hour < 17 {
            "afternoon"
        } else {
            "evening"
        };
        format!("Good {}, {}.", time_word, cfg.user.display_name)
    };
    app.set_greeting(SharedString::from(greeting));
    app.set_briefing_time(SharedString::from("Loading your inbox…"));

    // Wire refresh callback.
    {
        let cfg_clone = cfg.clone();
        let weak = app.as_weak();
        app.on_refresh_clicked(move || {
            let cfg = cfg_clone.clone();
            let weak = weak.clone();
            tokio::spawn(async move {
                app_state::refresh(cfg, weak).await;
            });
        });
    }

    // Wire reply callback.
    {
        let weak = app.as_weak();
        app.on_reply_clicked(move |uid| {
            if let Some(handle) = weak.upgrade() {
                let email_to = find_email_field(&handle, uid, |c| c.from_name.to_string());
                let reply_subject = find_email_field(&handle, uid, |c| c.subject.to_string());
                handle.set_compose_to(SharedString::from(email_to));
                handle.set_compose_subject(SharedString::from(format!("Re: {}", reply_subject)));
                handle.set_compose_body(SharedString::from(""));
                handle.set_compose_open(true);
            }
        });
    }

    // Wire view callback — populate reading pane.
    {
        let weak = app.as_weak();
        app.on_view_clicked(move |uid| {
            info!("View email uid={}", uid);
            if let Some(handle) = weak.upgrade() {
                // Search all email lists for this uid.
                let from = find_email_field(&handle, uid, |c| c.from_name.to_string());
                let subject = find_email_field(&handle, uid, |c| c.subject.to_string());
                let date = find_email_field(&handle, uid, |c| c.date_str.to_string());
                let body = find_email_field(&handle, uid, |c| c.body.to_string());

                if !subject.is_empty() {
                    handle.set_active_email_uid(uid);
                    handle.set_reading_from(SharedString::from(from));
                    handle.set_reading_subject(SharedString::from(subject));
                    handle.set_reading_date(SharedString::from(date));
                    handle.set_reading_body(SharedString::from(body));
                }
            }
        });
    }

    // Wire send callback.
    {
        let cfg_clone = cfg.clone();
        let weak = app.as_weak();
        app.on_send_email(move |to, subject, body| {
            let cfg = cfg_clone.clone();
            let weak = weak.clone();
            let to = to.to_string();
            let subject = subject.to_string();
            let body = body.to_string();
            tokio::spawn(async move {
                // Send from the first configured account.
                let smtp = cfg.accounts.first().map(|a| a.smtp.clone());
                let result = if let Some(smtp) = smtp {
                    smtp_sender::send_reply(&smtp, &to, &subject, &body).await
                } else {
                    Err(anyhow::anyhow!("No SMTP account configured"))
                };
                match result {
                    Ok(()) => {
                        slint::invoke_from_event_loop(move || {
                            if let Some(handle) = weak.upgrade() {
                                handle.set_compose_status(SharedString::from("Sent!"));
                                handle.set_compose_open(false);
                                handle.set_compose_to(SharedString::from(""));
                                handle.set_compose_subject(SharedString::from(""));
                                handle.set_compose_body(SharedString::from(""));
                                handle.set_compose_status(SharedString::from(""));
                            }
                        })
                        .ok();
                    }
                    Err(e) => {
                        let msg = format!("Send failed: {}", e);
                        slint::invoke_from_event_loop(move || {
                            if let Some(handle) = weak.upgrade() {
                                handle.set_compose_status(SharedString::from(msg));
                            }
                        })
                        .ok();
                    }
                }
            });
        });
    }

    // Wire save-description callback.
    app.on_save_description(move |name, desc| {
        if let Err(e) = config::save_project_description(&name, &desc) {
            tracing::warn!("Failed to save project description: {}", e);
        }
    });

    // Wire open-url callback.
    app.on_open_url(move |url| {
        open::that(url.as_str()).ok();
    });

    // Populate settings fields from current config.
    {
        let accs = settings::accounts_to_slint(&cfg);
        app.set_settings_accounts(slint::ModelRc::new(slint::VecModel::from(accs)));
        app.set_settings_display_name(SharedString::from(cfg.user.display_name.as_str()));
        app.set_settings_claude_key(SharedString::from(cfg.claude.api_key.as_str()));
        app.set_settings_claude_model(SharedString::from(cfg.claude.model.as_str()));
        // Restore saved font scale.
        app.set_font_scale(cfg.user.font_scale);
    }

    // Wire save-settings callback.
    {
        let weak = app.as_weak();
        app.on_save_settings(move |display_name, accounts, claude_key, claude_model, font_scale| {
            let accs: Vec<AccountData> = (0..accounts.row_count())
                .filter_map(|i| accounts.row_data(i))
                .collect();
            match settings::save_config(&display_name, &accs, &claude_key, &claude_model, font_scale) {
                Ok(()) => {
                    if let Some(handle) = weak.upgrade() {
                        handle.set_settings_save_status(SharedString::from("Saved!"));
                    }
                }
                Err(e) => {
                    let msg = format!("Error: {}", e);
                    if let Some(handle) = weak.upgrade() {
                        handle.set_settings_save_status(SharedString::from(msg));
                    }
                }
            }
        });
    }

    // Wire add-account callback — appends a blank account to the list.
    {
        let weak = app.as_weak();
        app.on_add_account(move || {
            if let Some(handle) = weak.upgrade() {
                let model = handle.get_settings_accounts();
                let mut accs: Vec<AccountData> = (0..model.row_count())
                    .filter_map(|i| model.row_data(i))
                    .collect();
                accs.push(AccountData {
                    label: SharedString::from("New account"),
                    imap_host: SharedString::default(),
                    imap_port: SharedString::from("993"),
                    imap_user: SharedString::default(),
                    imap_pass: SharedString::default(),
                    imap_mailbox: SharedString::from("INBOX"),
                    imap_fetch: SharedString::from("50"),
                    smtp_host: SharedString::default(),
                    smtp_port: SharedString::from("587"),
                    smtp_user: SharedString::default(),
                    smtp_pass: SharedString::default(),
                    smtp_from: SharedString::default(),
                });
                handle.set_settings_accounts(slint::ModelRc::new(slint::VecModel::from(accs)));
            }
        });
    }

    // Wire delete-account callback.
    {
        let weak = app.as_weak();
        app.on_delete_account(move |idx| {
            if let Some(handle) = weak.upgrade() {
                let model = handle.get_settings_accounts();
                let mut accs: Vec<AccountData> = (0..model.row_count())
                    .filter_map(|i| model.row_data(i))
                    .collect();
                let idx = idx as usize;
                if idx < accs.len() {
                    accs.remove(idx);
                    handle.set_settings_accounts(slint::ModelRc::new(slint::VecModel::from(accs)));
                }
            }
        });
    }

    // Wire request-qr callback.
    {
        let cfg_clone = cfg.clone();
        let weak = app.as_weak();
        app.on_request_qr(move || {
            let qr = settings::generate_qr(&cfg_clone);
            if let Some(handle) = weak.upgrade() {
                handle.set_settings_qr_text(SharedString::from(qr));
            }
        });
    }

    // Load cached emails immediately (instant display, no network).
    app_state::load_and_display_cache(&cfg, &app.as_weak());

    // Trigger background refresh to fetch and classify new emails.
    {
        let cfg_clone = cfg.clone();
        let weak = app.as_weak();
        tokio::spawn(async move {
            app_state::refresh(cfg_clone, weak).await;
        });
    }

    app.run()?;
    Ok(())
}

async fn check_config() -> Result<()> {
    let cfg = config::Config::load()?;
    println!("Config loaded OK");
    println!();

    for account in &cfg.accounts {
        print!("IMAP [{}]  {}:{}  {} ... ", account.name, account.imap.host, account.imap.port, account.imap.username);
        match imap_client::fetch_emails(&account.imap).await {
            Ok(emails) => println!("OK ({} emails found)", emails.len()),
            Err(e) => println!("FAILED\n       {:?}", e),
        }
    }

    print!("Claude API  model={}  ... ", cfg.claude.model);
    let client = reqwest::Client::new();
    let resp = client
        .get("https://api.anthropic.com/v1/models")
        .header("x-api-key", &cfg.claude.api_key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await;
    match resp {
        Ok(r) if r.status().is_success() => println!("OK"),
        Ok(r) => println!("FAILED (HTTP {})", r.status()),
        Err(e) => println!("FAILED\n       {}", e),
    }

    Ok(())
}

/// Search all email lists for a uid and extract a field via closure.
fn find_email_field<F>(handle: &AppWindow, uid: i32, extract: F) -> String
where
    F: Fn(&EmailCardData) -> String,
{
    for list_getter in [
        AppWindow::get_all_emails,
        AppWindow::get_urgent_emails,
        AppWindow::get_high_emails,
        AppWindow::get_other_emails,
        AppWindow::get_info_emails,
        AppWindow::get_threat_emails,
        AppWindow::get_task_emails,
        AppWindow::get_task_items,
        AppWindow::get_meeting_items,
        AppWindow::get_important_emails,
        AppWindow::get_sent_emails,
        AppWindow::get_drafts_emails,
        AppWindow::get_trash_emails,
    ] {
        let model = list_getter(handle);
        for i in 0..model.row_count() {
            if let Some(card) = model.row_data(i) {
                if card.uid == uid {
                    return extract(&card);
                }
            }
        }
    }
    String::new()
}
