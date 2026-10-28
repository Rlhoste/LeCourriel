//! Sync app data with Supabase (PostgreSQL + accounts + row-level security).
//!
//! Local files stay the working copy (the app works offline). At startup each
//! document is pulled or pushed, whichever side is newer; after every local save
//! the document is pushed in the background. AI results are stored per email
//! (Message-ID) so another computer doesn't re-classify them.
//!
//! The app signs in with the user's Supabase account and calls the SQL functions
//! from `supabase/schema.sql`; row-level security makes PostgreSQL itself refuse
//! access to other users' rows. Mail passwords, API keys and the Supabase settings
//! never leave this computer.

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::SystemTime;
use tracing::{info, warn};

use crate::ai_classifier::ClassifiedEmail;
use crate::config::Config;

const SESSION_PATH: &str = "session.json";

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

    /// Full path of the local file.
    fn file(self) -> std::path::PathBuf {
        crate::paths::file(self.path())
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

/// Supabase project, set from config at startup and when the settings change.
#[derive(Debug, Clone)]
struct Target {
    /// e.g. https://abcd.supabase.co (no trailing slash)
    url: String,
    anon_key: String,
    mailbox: String,
}

/// A signed-in Supabase session (kept in session.json on this computer only).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix time (seconds) when the access token expires.
    pub expires_at: i64,
    pub user_id: String,
    pub email: String,
}

static TARGET: RwLock<Option<Target>> = RwLock::new(None);
static SESSION: RwLock<Option<Session>> = RwLock::new(None);
/// Supabase rotates refresh tokens: never refresh twice at the same time.
static REFRESH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Rows are keyed by the first account ("user@host"), so several mailboxes can share a database.
pub fn mailbox_key(cfg: &Config) -> String {
    cfg.accounts
        .first()
        .map(|a| format!("{}@{}", a.imap.username, a.imap.host))
        .unwrap_or_else(|| "default".to_string())
}

pub fn configure(cfg: &Config) {
    let target = cfg.supabase_settings().map(|(url, anon_key)| Target {
        url: url.trim_end_matches('/').to_string(),
        anon_key: anon_key.to_string(),
        mailbox: mailbox_key(cfg),
    });
    let has_target = target.is_some();
    *TARGET.write().unwrap_or_else(|e| e.into_inner()) = target;
    let session = if has_target { load_session() } else { None };
    *SESSION.write().unwrap_or_else(|e| e.into_inner()) = session;
}

fn target() -> Option<Target> {
    TARGET.read().unwrap_or_else(|e| e.into_inner()).clone()
}

fn session() -> Option<Session> {
    SESSION.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Sync is active: a project is configured and someone is signed in.
pub fn is_configured() -> bool {
    target().is_some() && session().is_some()
}

#[derive(Debug, Clone, Serialize)]
pub struct CloudStatus {
    pub configured: bool,
    pub signed_in: bool,
    pub email: String,
}

pub fn status() -> CloudStatus {
    let s = session();
    CloudStatus {
        configured: target().is_some(),
        signed_in: s.is_some(),
        email: s.map(|s| s.email).unwrap_or_default(),
    }
}

// ── Session storage ──────────────────────────────────────────────────────────

fn load_session() -> Option<Session> {
    let text = std::fs::read_to_string(crate::paths::file(SESSION_PATH)).ok()?;
    serde_json::from_str(&text).ok()
}

fn store_session(s: Option<Session>) {
    match &s {
        Some(s) => {
            if let Ok(json) = serde_json::to_string_pretty(s) {
                if let Err(e) = write_private(&crate::paths::file(SESSION_PATH), &json) {
                    warn!("Could not save the Supabase session: {}", e);
                }
            }
        }
        None => {
            let _ = std::fs::remove_file(crate::paths::file(SESSION_PATH));
        }
    }
    *SESSION.write().unwrap_or_else(|e| e.into_inner()) = s;
}

/// Write a file readable only by the current user (it holds a refresh token).
fn write_private(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    std::fs::write(path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

// ── HTTP: Supabase Auth and the SQL functions ───────────────────────────────

fn http() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build()?)
}

/// A readable message from a Supabase Auth or PostgREST error body.
pub fn error_message(status: reqwest::StatusCode, body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        for key in ["error_description", "msg", "message", "error"] {
            if let Some(m) = v.get(key).and_then(Value::as_str) {
                if !m.is_empty() {
                    return m.to_string();
                }
            }
        }
    }
    let body: String = body.chars().take(200).collect();
    format!("Supabase error {}: {}", status, body)
}

/// Read a session from an Auth response (sign-in, sign-up with auto-confirm, refresh).
pub fn parse_session(v: &Value, now: i64) -> Option<Session> {
    let access_token = v.get("access_token")?.as_str()?.to_string();
    let refresh_token = v.get("refresh_token")?.as_str()?.to_string();
    let expires_at = v
        .get("expires_at")
        .and_then(Value::as_i64)
        .or_else(|| v.get("expires_in").and_then(Value::as_i64).map(|s| now + s))
        .unwrap_or(now + 3600);
    let user = v.get("user")?;
    Some(Session {
        access_token,
        refresh_token,
        expires_at,
        user_id: user.get("id")?.as_str()?.to_string(),
        email: user.get("email").and_then(Value::as_str).unwrap_or_default().to_string(),
    })
}

async fn auth_post(t: &Target, path: &str, body: Value) -> Result<Value> {
    let resp = http()?
        .post(format!("{}/auth/v1/{}", t.url, path))
        .header("apikey", &t.anon_key)
        .json(&body)
        .send()
        .await
        .context("Could not reach Supabase")?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!(error_message(status, &text));
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

pub async fn sign_in(email: &str, password: &str) -> Result<CloudStatus> {
    let t = target().ok_or_else(|| anyhow!("Enter the Supabase project URL and anon key first"))?;
    let v = auth_post(&t, "token?grant_type=password", json!({ "email": email.trim(), "password": password })).await?;
    let s = parse_session(&v, Utc::now().timestamp()).ok_or_else(|| anyhow!("Unexpected sign-in response"))?;
    info!("Signed in to Supabase as {}", s.email);
    store_session(Some(s));
    Ok(status())
}

/// Returns true when signed in straight away, false when Supabase sent a confirmation email.
pub async fn sign_up(email: &str, password: &str) -> Result<bool> {
    let t = target().ok_or_else(|| anyhow!("Enter the Supabase project URL and anon key first"))?;
    let v = auth_post(&t, "signup", json!({ "email": email.trim(), "password": password })).await?;
    match parse_session(&v, Utc::now().timestamp()) {
        Some(s) => {
            store_session(Some(s));
            Ok(true)
        }
        None => Ok(false),
    }
}

pub async fn sign_out() {
    if let (Some(t), Some(s)) = (target(), session()) {
        // Revoke the refresh token on the server; sign out locally even if this fails.
        if let Ok(c) = http() {
            let _ = c
                .post(format!("{}/auth/v1/logout", t.url))
                .header("apikey", &t.anon_key)
                .bearer_auth(&s.access_token)
                .send()
                .await;
        }
    }
    store_session(None);
}

/// A valid access token, refreshing it when it expires within a minute (or when forced).
async fn access_token(t: &Target, force_refresh: bool) -> Result<String> {
    let _guard = REFRESH_LOCK.lock().await;
    let s = session().ok_or_else(|| anyhow!("Not signed in to Supabase"))?;
    if !force_refresh && s.expires_at - 60 > Utc::now().timestamp() {
        return Ok(s.access_token);
    }
    let v = match auth_post(t, "token?grant_type=refresh_token", json!({ "refresh_token": s.refresh_token })).await {
        Ok(v) => v,
        Err(e) => {
            // The refresh token is no longer valid: the user must sign in again.
            store_session(None);
            return Err(e.context("Supabase session expired, please sign in again"));
        }
    };
    let new = parse_session(&v, Utc::now().timestamp()).ok_or_else(|| anyhow!("Unexpected refresh response"))?;
    let token = new.access_token.clone();
    store_session(Some(new));
    Ok(token)
}

/// Call one of the SQL functions from supabase/schema.sql.
async fn rpc(t: &Target, function: &str, args: Value) -> Result<Value> {
    let client = http()?;
    let mut token = access_token(t, false).await?;
    for attempt in 0..2 {
        let resp = client
            .post(format!("{}/rest/v1/rpc/{}", t.url, function))
            .header("apikey", &t.anon_key)
            .bearer_auth(&token)
            .json(&args)
            .send()
            .await
            .context("Could not reach Supabase")?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
            token = access_token(t, true).await?;
            continue;
        }
        if !status.is_success() {
            bail!(error_message(status, &text));
        }
        return Ok(if text.trim().is_empty() { Value::Null } else { serde_json::from_str(&text)? });
    }
    unreachable!("the loop returns or bails on its second attempt")
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

fn local_mtime(path: &std::path::Path) -> Option<DateTime<Utc>> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().map(DateTime::<Utc>::from)
}

fn set_mtime(path: &std::path::Path, when: DateTime<Utc>) {
    let when: SystemTime = when.into();
    if let Err(e) = std::fs::File::options().write(true).open(path).and_then(|f| f.set_modified(when)) {
        warn!("Could not set the modification time of {}: {}", path.display(), e);
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
        "ai_provider": cfg.ai.provider(),
        "ai_model": cfg.ai.model,
        "projects": cfg.projects,
    })
}

/// Apply synced settings onto the local config, keeping accounts, keys and the database URL.
pub fn merge_prefs(cfg: &mut Config, prefs: &Value) {
    let user = &prefs["user"];
    if let Some(v) = user["display_name"].as_str() {
        cfg.user.display_name = v.to_string();
    }
    if let Some(v) = user["font_scale"].as_f64() {
        cfg.user.font_scale = v as f32;
    }
    if let Some(v) = user["theme"].as_str() {
        cfg.user.theme = v.to_string();
    }
    if let Some(v) = user["language"].as_str() {
        cfg.user.language = v.to_string();
    }
    // The provider and model sync; keys and the server URL stay on each computer.
    if let Some(v) = prefs["ai_provider"].as_str().filter(|p| crate::config::AI_PROVIDERS.contains(p)) {
        cfg.ai.provider = v.to_string();
    }
    if let Some(v) = prefs["ai_model"].as_str().or(prefs["claude_model"].as_str()).filter(|m| !m.is_empty()) {
        cfg.ai.model = v.to_string();
    }
    if let Ok(projects) = serde_json::from_value(prefs["projects"].clone()) {
        cfg.projects = projects;
    }
}

/// The document's current local content as JSON (None if the file doesn't exist).
fn local_value(doc: Doc) -> Result<Option<Value>> {
    if doc == Doc::Prefs {
        if std::fs::metadata(doc.file()).is_err() {
            return Ok(None);
        }
        return Ok(Some(prefs_from_config(&Config::load()?)));
    }
    match std::fs::read_to_string(doc.file()) {
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
        std::fs::write(doc.file(), serde_json::to_string_pretty(data)?)?;
    }
    // Same timestamp as the database, so the next startup sees both sides as equal.
    set_mtime(&doc.file(), updated_at);
    Ok(())
}

async fn push_doc(t: &Target, doc: Doc) -> Result<()> {
    let Some(data) = local_value(doc)? else { return Ok(()) };
    let updated_at = local_mtime(&doc.file()).unwrap_or_else(Utc::now);
    rpc(
        t,
        "courriel_push",
        json!({
            "p_mailbox": t.mailbox,
            "p_doc": doc.name(),
            "p_data": data,
            "p_updated_at": updated_at.to_rfc3339(),
        }),
    )
    .await
    .with_context(|| format!("Could not upload {}", doc.name()))?;
    Ok(())
}

/// Read a pulled document: {"data": …, "updated_at": "…"} or null.
pub fn parse_pulled(v: &Value) -> Option<(Value, DateTime<Utc>)> {
    let data = v.get("data")?.clone();
    let at = DateTime::parse_from_rfc3339(v.get("updated_at")?.as_str()?).ok()?.with_timezone(&Utc);
    Some((data, at))
}

/// Startup sync: for each document, pull or push whichever side is newer.
pub async fn pull_all() -> Result<String> {
    let Some(t) = target() else { return Ok("cloud sync off".into()) };
    if session().is_none() {
        return Ok("cloud sync: not signed in".into());
    }
    let mut pulled = vec![];
    let mut pushed = vec![];
    for doc in Doc::ALL {
        let v = rpc(&t, "courriel_pull", json!({ "p_mailbox": t.mailbox, "p_doc": doc.name() })).await?;
        let remote = parse_pulled(&v);
        match decide(local_mtime(&doc.file()), remote.as_ref().map(|(_, at)| *at)) {
            SyncAction::Pull => {
                let (data, at) = remote.expect("decide() only pulls when a remote row exists");
                write_local(doc, &data, at)?;
                pulled.push(doc.name());
            }
            SyncAction::Push => {
                push_doc(&t, doc).await?;
                pushed.push(doc.name());
            }
            SyncAction::Nothing => {}
        }
    }
    Ok(format!("pulled [{}], pushed [{}]", pulled.join(", "), pushed.join(", ")))
}

/// Upload a document in the background after a local save. No-op when sync is off.
pub fn schedule_push(doc: Doc) {
    let Some(t) = target() else { return };
    if session().is_none() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        if let Err(e) = push_doc(&t, doc).await {
            warn!("Cloud sync of {} failed: {:#}", doc.name(), e);
        }
    });
}

// ── AI results ───────────────────────────────────────────────────────────────

/// Stored AI results for these Message-IDs (empty map when sync is off or on error).
pub async fn lookup_ai(message_ids: &[String]) -> HashMap<String, ClassifiedEmail> {
    let Some(t) = target() else { return HashMap::new() };
    if session().is_none() || message_ids.is_empty() {
        return HashMap::new();
    }
    match rpc(&t, "courriel_lookup_insights", json!({ "p_mailbox": t.mailbox, "p_ids": message_ids })).await {
        Ok(Value::Object(map)) => {
            let found: HashMap<String, ClassifiedEmail> =
                map.into_iter().filter_map(|(id, v)| serde_json::from_value(v).ok().map(|c| (id, c))).collect();
            if !found.is_empty() {
                info!("Reused {} AI result(s) from Supabase", found.len());
            }
            found
        }
        Ok(_) => HashMap::new(),
        Err(e) => {
            warn!("AI results lookup failed: {:#}", e);
            HashMap::new()
        }
    }
}

/// Save AI results so other computers can reuse them. Errors are only logged.
pub async fn store_ai(results: &[(String, ClassifiedEmail)]) {
    let Some(t) = target() else { return };
    if session().is_none() || results.is_empty() {
        return;
    }
    let items: serde_json::Map<String, Value> =
        results.iter().filter_map(|(id, c)| serde_json::to_value(c).ok().map(|v| (id.clone(), v))).collect();
    if let Err(e) = rpc(&t, "courriel_store_insights", json!({ "p_mailbox": t.mailbox, "p_items": items })).await {
        warn!("Saving AI results to Supabase failed: {:#}", e);
    }
}
