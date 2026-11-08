//! Token-free abuse protection before AI classification.
//!
//! This is deliberately metadata-only: server spam headers, sender/domain bursts,
//! authenticated mailing lists and an hourly AI-call budget. State is persisted so an
//! attacker cannot reset the limits by waiting for the next two-minute refresh.

use crate::config::AccountConfig;
use crate::models::{split_mail_id, MailSecurity, RawEmail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

const STATE_FILE: &str = "spam-guard.json";
const TEN_MINUTES: i64 = 10 * 60;
const ONE_HOUR: i64 = 60 * 60;

// Aggressive profile selected by the user.
pub const SENDER_LIMIT_10_MIN: usize = 5;
pub const DOMAIN_LIMIT_10_MIN: usize = 15;
pub const GLOBAL_LIMIT_HOUR: usize = 25;
pub const AI_LIMIT_HOUR: usize = 10;

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Arrival {
    at: i64,
    account: String,
    identity: String,
    domain: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct GuardState {
    /// Highest IMAP UID observed for each account. A missing entry is an initial sync.
    #[serde(default)]
    last_uid: BTreeMap<String, u32>,
    #[serde(default)]
    arrivals: Vec<Arrival>,
    #[serde(default)]
    ai_calls: Vec<i64>,
    /// Messages that crossed a hard rule but could not yet be moved on the server.
    #[serde(default)]
    blocked: BTreeMap<String, Vec<u32>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Spam(&'static str),
}

fn account_key(account: &AccountConfig) -> String {
    format!(
        "{}\n{}\n{}",
        account.imap.host.to_ascii_lowercase(),
        account.imap.username.to_ascii_lowercase(),
        account.imap.mailbox.to_ascii_lowercase()
    )
}

fn load() -> GuardState {
    std::fs::read_to_string(crate::paths::file(STATE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save(state: &GuardState) {
    let path = crate::paths::file(STATE_FILE);
    if let Ok(json) = serde_json::to_string_pretty(state) {
        if let Err(e) = std::fs::write(path, json) {
            tracing::warn!("Could not save spam guard state: {}", e);
        }
    }
}

fn address(value: &str) -> String {
    let value = value.trim();
    let candidate = value
        .rfind('<')
        .and_then(|start| value[start + 1..].find('>').map(|end| &value[start + 1..start + 1 + end]))
        .unwrap_or(value)
        .trim()
        .trim_matches('"');
    candidate.to_ascii_lowercase()
}

pub fn addresses(value: &str) -> Vec<String> {
    value.split([',', ';']).map(address).filter(|entry| entry.contains('@')).collect()
}

fn domain(sender: &str) -> String {
    sender.rsplit_once('@').map(|(_, domain)| domain.trim_end_matches('.').to_string()).unwrap_or_default()
}

fn dmarc_passes(security: &MailSecurity) -> bool {
    let auth = security.authentication_results.to_ascii_lowercase();
    auth.contains("dmarc=pass")
}

fn is_authenticated_bulk(security: &MailSecurity) -> bool {
    let auth = security.authentication_results.to_ascii_lowercase();
    (auth.contains("dmarc=pass") || auth.contains("dkim=pass"))
        && (!security.list_id.trim().is_empty()
            || !security.list_unsubscribe.trim().is_empty()
            || matches!(security.precedence.trim().to_ascii_lowercase().as_str(), "bulk" | "list"))
}

fn number(value: &str) -> Option<f32> {
    value
        .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .find(|part| !part.is_empty())
        .and_then(|part| part.parse().ok())
}

/// Strong verdicts already produced by the receiving mail server. A forged positive verdict
/// can only make the sender's own message be treated as spam, so it cannot expose other mail.
pub fn server_marks_spam(security: &MailSecurity) -> bool {
    security.spam_flag.trim().eq_ignore_ascii_case("yes")
        || security.spam_status.trim_start().to_ascii_lowercase().starts_with("yes")
        || number(&security.spam_score).is_some_and(|score| score >= 5.0)
        || number(&security.microsoft_scl).is_some_and(|score| score >= 5.0)
}

/// Evaluate messages that are absent from the classified cache. Initial synchronization only
/// establishes a UID baseline; it never treats an old mailbox backlog as a live attack.
pub fn assess_batch(
    accounts: &[AccountConfig],
    emails: &[RawEmail],
    trusted_correspondents: &HashSet<String>,
    now: i64,
) -> HashMap<u64, Decision> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut state = load();
    state.arrivals.retain(|event| event.at > now - ONE_HOUR);
    state.ai_calls.retain(|at| *at > now - ONE_HOUR);
    let mut decisions = HashMap::new();

    for (account_index, account) in accounts.iter().enumerate() {
        let key = account_key(account);
        let mut account_mail: Vec<&RawEmail> =
            emails.iter().filter(|email| split_mail_id(email.uid).0 == account_index).collect();
        account_mail.sort_by_key(|email| split_mail_id(email.uid).1);
        let Some(max_uid) = account_mail.last().map(|email| split_mail_id(email.uid).1) else { continue };
        let previous = state.last_uid.get(&key).copied();
        let baseline = previous.is_none() || previous.is_some_and(|last| max_uid < last);
        if baseline {
            state.arrivals.retain(|event| event.account != key);
            state.blocked.remove(&key);
        }

        for email in account_mail {
            let uid = split_mail_id(email.uid).1;
            let already_blocked = state.blocked.get(&key).is_some_and(|uids| uids.contains(&uid));
            if already_blocked {
                decisions.insert(email.uid, Decision::Spam("previously blocked burst"));
                continue;
            }
            if server_marks_spam(&email.security) {
                state.blocked.entry(key.clone()).or_default().push(uid);
                decisions.insert(email.uid, Decision::Spam("mail server marked it as spam"));
                continue;
            }
            if baseline || previous.is_some_and(|last| uid <= last) {
                decisions.insert(email.uid, Decision::Allow);
                continue;
            }

            let sender = address(&email.from);
            let sender_domain = domain(&sender);
            let trusted = trusted_correspondents.contains(&sender) && dmarc_passes(&email.security);
            let bulk = is_authenticated_bulk(&email.security);
            let identity = if bulk {
                format!("list:{}", email.security.list_id.trim().to_ascii_lowercase())
            } else {
                sender.clone()
            };
            let recent = now - TEN_MINUTES;
            let sender_count = state
                .arrivals
                .iter()
                .filter(|event| event.at > recent && event.account == key && event.identity == identity)
                .count();
            let domain_count = state
                .arrivals
                .iter()
                .filter(|event| event.at > recent && event.account == key && event.domain == sender_domain)
                .count();
            let global_count = state.arrivals.iter().filter(|event| event.at > now - ONE_HOUR).count();
            let reason = if global_count >= GLOBAL_LIMIT_HOUR {
                Some("global arrival limit")
            } else if !trusted && !bulk && sender_count >= SENDER_LIMIT_10_MIN {
                Some("sender burst limit")
            } else if !trusted && !bulk && !sender_domain.is_empty() && domain_count >= DOMAIN_LIMIT_10_MIN {
                Some("domain burst limit")
            } else {
                None
            };
            state.arrivals.push(Arrival { at: now, account: key.clone(), identity, domain: sender_domain });
            if let Some(reason) = reason {
                let blocked = state.blocked.entry(key.clone()).or_default();
                if !blocked.contains(&uid) {
                    blocked.push(uid);
                }
                decisions.insert(email.uid, Decision::Spam(reason));
            } else {
                decisions.insert(email.uid, Decision::Allow);
            }
        }
        state.last_uid.insert(key, max_uid);
    }
    save(&state);
    decisions
}

/// Reserve model calls before starting concurrent requests. Returns how many may run.
pub fn reserve_ai_slots(requested: usize, now: i64) -> usize {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut state = load();
    state.ai_calls.retain(|at| *at > now - ONE_HOUR);
    let available = AI_LIMIT_HOUR.saturating_sub(state.ai_calls.len());
    let granted = requested.min(available);
    state.ai_calls.extend(std::iter::repeat_n(now, granted));
    save(&state);
    granted
}

/// A successful server move releases the remembered quarantine entry.
pub fn mark_moved(account: &AccountConfig, uid: u32) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut state = load();
    let key = account_key(account);
    if let Some(blocked) = state.blocked.get_mut(&key) {
        blocked.retain(|stored| *stored != uid);
        if blocked.is_empty() {
            state.blocked.remove(&key);
        }
    }
    save(&state);
}

#[cfg(test)]
pub fn reset() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = std::fs::remove_file(crate::paths::file(STATE_FILE));
}
