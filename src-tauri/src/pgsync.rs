//! Sync app data with the user's own PostgreSQL server.
//!
//! Local files stay the working copy (the app works offline); PostgreSQL is a sync
//! target. At startup each document is pulled or pushed, whichever side is newer;
//! after every local save the document is pushed in the background. AI results are
//! stored per email (Message-ID) so another computer doesn't re-classify them.
//!
//! Secrets never leave this computer: mail passwords, API keys and the database URL
//! are stripped from the synced preferences.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::SystemTime;
use tracing::{info, warn};

use crate::ai_classifier::ClassifiedEmail;
use crate::config::Config;

/// A synced document and the local file backing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doc {
    Tasks,
    Dismissed,
    Overrides,
    /// Non-secret settings from config.toml (projects, theme, language…).
    Prefs,
}

impl Doc {
    pub const ALL: [Doc; 4] = [Doc::Tasks, Doc::Dismissed, Doc::Overrides, Doc::Prefs];

    pub fn name(self) -> &'static str {
        match self {
            Doc::Tasks => "tasks",
            Doc::Dismissed => "dismissed",
            Doc::Overrides => "overrides",
            Doc::Prefs => "prefs",
        }
    }

    fn path(self) -> &'static str {
        match self {
            Doc::Tasks => "tasks.json",
            Doc::Dismissed => "dismissed.json",
            Doc::Overrides => "overrides.json",
            Doc::Prefs => "config.toml",
        }
    }
}

/// Connection settings, set from config at startup and after settings are saved.
#[derive(Debug, Clone)]
struct Target {
    url: String,
    accept_invalid_certs: bool,
    mailbox: String,
}

static TARGET: RwLock<Option<Target>> = RwLock::new(None);

/// Rows are keyed by the first account ("user@host"), so several mailboxes can share a database.
pub fn mailbox_key(cfg: &Config) -> String {
    cfg.accounts
        .first()
        .map(|a| format!("{}@{}", a.imap.username, a.imap.host))
        .unwrap_or_else(|| "default".to_string())
}

pub fn configure(cfg: &Config) {
    let target = cfg.database_url().map(|url| Target {
        url: url.to_string(),
        accept_invalid_certs: cfg.database.as_ref().map_or(false, |d| d.accept_invalid_certs),
        mailbox: mailbox_key(cfg),
    });
    *TARGET.write().unwrap_or_else(|e| e.into_inner()) = target;
}

fn target() -> Option<Target> {
    TARGET.read().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn is_configured() -> bool {
    target().is_some()
}

// ── Connection ───────────────────────────────────────────────────────────────

async fn connect_raw(url: &str, accept_invalid_certs: bool) -> Result<tokio_postgres::Client> {
    let tls = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(accept_invalid_certs)
        .build()
        .context("Failed to build TLS connector")?;
    let tls = postgres_native_tls::MakeTlsConnector::new(tls);
    let (client, connection) = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio_postgres::connect(url, tls),
    )
    .await
    .context("Timed out connecting to PostgreSQL")?
    .context("Could not connect to PostgreSQL")?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            warn!("PostgreSQL connection closed: {}", e);
        }
    });
    ensure_schema(&client).await?;
    Ok(client)
}

async fn connect(t: &Target) -> Result<tokio_postgres::Client> {
    connect_raw(&t.url, t.accept_invalid_certs).await
}

async fn ensure_schema(client: &tokio_postgres::Client) -> Result<()> {
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS courriel_documents (
               mailbox    text        NOT NULL,
               name       text        NOT NULL,
               data       jsonb       NOT NULL,
               updated_at timestamptz NOT NULL,
               PRIMARY KEY (mailbox, name)
             );
             CREATE TABLE IF NOT EXISTS courriel_ai_results (
               mailbox    text        NOT NULL,
               message_id text        NOT NULL,
               result     jsonb       NOT NULL,
               updated_at timestamptz NOT NULL DEFAULT now(),
               PRIMARY KEY (mailbox, message_id)
             );",
        )
        .await
        .context("Could not create the courriel tables")
}

/// Connect, create the tables if needed, and report the server version.
pub async fn test_connection(url: &str, accept_invalid_certs: bool) -> Result<String> {
    let client = connect_raw(url, accept_invalid_certs).await?;
    let row = client.query_one("SHOW server_version", &[]).await?;
    let version: String = row.get(0);
    Ok(format!("PostgreSQL {}", version))
}

// ── Documents ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncAction {
    Pull,
    Push,
    Nothing,
}

/// Newer side wins. Within 2 s counts as the same (filesystem timestamp precision).
pub fn decide(local: Option<DateTime<Utc>>, remote: Option<DateTime<Utc>>) -> SyncAction {
    match (local, remote) {
        (None, None) => SyncAction::Nothing,
        (Some(_), None) => SyncAction::Push,
        (None, Some(_)) => SyncAction::Pull,
        (Some(l), Some(r)) => {
            let diff = (r - l).num_milliseconds();
            if diff > 2000 {
                SyncAction::Pull
            } else if diff < -2000 {
                SyncAction::Push
            } else {
                SyncAction::Nothing
            }
        }
    }
}

fn local_mtime(path: &str) -> Option<DateTime<Utc>> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().map(DateTime::<Utc>::from)
}

fn set_mtime(path: &str, when: DateTime<Utc>) {
    let when: SystemTime = when.into();
    if let Err(e) = std::fs::File::options().write(true).open(path).and_then(|f| f.set_modified(when)) {
        warn!("Could not set the modification time of {}: {}", path, e);
    }
}

/// The settings that sync: everything except accounts, secrets and the database itself.
pub fn prefs_from_config(cfg: &Config) -> Value {
    json!({
        "user": {
            "display_name": cfg.user.display_name,
            "font_scale": cfg.user.font_scale,
            "theme": cfg.user.theme,
            "language": cfg.user.language,
        },
        "claude_model": cfg.claude.model,
        "projects": cfg.projects,
    })
}

/// Apply synced settings onto the local config, keeping accounts, keys and the database URL.
pub fn merge_prefs(cfg: &mut Config, prefs: &Value) {
    let user = &prefs["user"];
    if let Some(v) = user["display_name"].as_str() { cfg.user.display_name = v.to_string(); }
    if let Some(v) = user["font_scale"].as_f64() { cfg.user.font_scale = v as f32; }
    if let Some(v) = user["theme"].as_str() { cfg.user.theme = v.to_string(); }
    if let Some(v) = user["language"].as_str() { cfg.user.language = v.to_string(); }
    if let Some(v) = prefs["claude_model"].as_str() { cfg.claude.model = v.to_string(); }
    if let Ok(projects) = serde_json::from_value(prefs["projects"].clone()) {
        cfg.projects = projects;
    }
}

/// The document's current local content as JSON (None if the file doesn't exist).
fn local_value(doc: Doc) -> Result<Option<Value>> {
    if doc == Doc::Prefs {
        if std::fs::metadata(doc.path()).is_err() {
            return Ok(None);
        }
        return Ok(Some(prefs_from_config(&Config::load()?)));
    }
    match std::fs::read_to_string(doc.path()) {
        Ok(s) => Ok(Some(serde_json::from_str(&s).with_context(|| format!("{} is not valid JSON", doc.path()))?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn write_local(doc: Doc, data: &Value, updated_at: DateTime<Utc>) -> Result<()> {
    if doc == Doc::Prefs {
        let mut cfg = Config::load()?;
        merge_prefs(&mut cfg, data);
        crate::config::write_local(&cfg)?;
    } else {
        std::fs::write(doc.path(), serde_json::to_string_pretty(data)?)?;
    }
    // Same timestamp as the database, so the next startup sees both sides as equal.
    set_mtime(doc.path(), updated_at);
    Ok(())
}

async fn push_doc(client: &tokio_postgres::Client, mailbox: &str, doc: Doc) -> Result<()> {
    let Some(data) = local_value(doc)? else { return Ok(()) };
    let updated_at = local_mtime(doc.path()).unwrap_or_else(Utc::now);
    client
        .execute(
            "INSERT INTO courriel_documents (mailbox, name, data, updated_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (mailbox, name) DO UPDATE
               SET data = EXCLUDED.data, updated_at = EXCLUDED.updated_at
               WHERE courriel_documents.updated_at <= EXCLUDED.updated_at",
            &[&mailbox, &doc.name(), &data, &updated_at],
        )
        .await
        .with_context(|| format!("Could not upload {}", doc.name()))?;
    Ok(())
}

/// Startup sync: for each document, pull or push whichever side is newer.
/// Returns a short summary for the log.
pub async fn pull_all() -> Result<String> {
    let Some(t) = target() else { return Ok("database sync off".into()) };
    let client = connect(&t).await?;
    let mut pulled = vec![];
    let mut pushed = vec![];
    for doc in Doc::ALL {
        let row = client
            .query_opt(
                "SELECT data, updated_at FROM courriel_documents WHERE mailbox = $1 AND name = $2",
                &[&t.mailbox, &doc.name()],
            )
            .await?;
        let remote: Option<(Value, DateTime<Utc>)> = row.map(|r| (r.get(0), r.get(1)));
        match decide(local_mtime(doc.path()), remote.as_ref().map(|(_, at)| *at)) {
            SyncAction::Pull => {
                let (data, at) = remote.expect("decide() only pulls when a remote row exists");
                write_local(doc, &data, at)?;
                pulled.push(doc.name());
            }
            SyncAction::Push => {
                push_doc(&client, &t.mailbox, doc).await?;
                pushed.push(doc.name());
            }
            SyncAction::Nothing => {}
        }
    }
    Ok(format!("pulled [{}], pushed [{}]", pulled.join(", "), pushed.join(", ")))
}

/// Upload a document in the background after a local save. No-op without a database.
pub fn schedule_push(doc: Doc) {
    let Some(t) = target() else { return };
    tauri::async_runtime::spawn(async move {
        let result = async {
            let client = connect(&t).await?;
            push_doc(&client, &t.mailbox, doc).await
        }
        .await;
        if let Err(e) = result {
            warn!("Database sync of {} failed: {:#}", doc.name(), e);
        }
    });
}

// ── AI results ───────────────────────────────────────────────────────────────

/// Stored AI results for these Message-IDs (empty map without a database or on error).
pub async fn lookup_ai(message_ids: &[String]) -> HashMap<String, ClassifiedEmail> {
    let Some(t) = target() else { return HashMap::new() };
    if message_ids.is_empty() {
        return HashMap::new();
    }
    let result: Result<HashMap<String, ClassifiedEmail>> = async {
        let client = connect(&t).await?;
        let rows = client
            .query(
                "SELECT message_id, result FROM courriel_ai_results
                 WHERE mailbox = $1 AND message_id = ANY($2)",
                &[&t.mailbox, &message_ids],
            )
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let id: String = r.get(0);
                let value: Value = r.get(1);
                serde_json::from_value(value).ok().map(|c| (id, c))
            })
            .collect())
    }
    .await;
    match result {
        Ok(map) => {
            if !map.is_empty() {
                info!("Reused {} AI result(s) from the database", map.len());
            }
            map
        }
        Err(e) => {
            warn!("AI results lookup failed: {:#}", e);
            HashMap::new()
        }
    }
}

/// Save AI results so other computers can reuse them. Errors are only logged.
pub async fn store_ai(results: &[(String, ClassifiedEmail)]) {
    let Some(t) = target() else { return };
    if results.is_empty() {
        return;
    }
    let outcome: Result<()> = async {
        let mut client = connect(&t).await?;
        let tx = client.transaction().await?;
        let stmt = tx
            .prepare(
                "INSERT INTO courriel_ai_results (mailbox, message_id, result, updated_at)
                 VALUES ($1, $2, $3, now())
                 ON CONFLICT (mailbox, message_id) DO UPDATE
                   SET result = EXCLUDED.result, updated_at = now()",
            )
            .await?;
        for (id, classified) in results {
            let value = serde_json::to_value(classified)?;
            tx.execute(&stmt, &[&t.mailbox, id, &value]).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    .await;
    if let Err(e) = outcome {
        warn!("Saving AI results to the database failed: {:#}", e);
    }
}

/// Remove every row of the current mailbox (integration tests clean up after themselves).
#[cfg(test)]
pub async fn delete_mailbox_rows() -> Result<()> {
    let Some(t) = target() else { return Ok(()) };
    let client = connect(&t).await?;
    client.execute("DELETE FROM courriel_documents WHERE mailbox = $1", &[&t.mailbox]).await?;
    client.execute("DELETE FROM courriel_ai_results WHERE mailbox = $1", &[&t.mailbox]).await?;
    Ok(())
}
