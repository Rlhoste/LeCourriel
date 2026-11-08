//! Private, local spam learning from explicit "Spam" and "Not spam" actions.
//!
//! The model is a bounded Bernoulli naive-Bayes classifier. It stores only hashes of
//! message features, not message text, and never calls a network service or consumes AI tokens.

use crate::models::{EmailCard, RawEmail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

const STATE_FILE: &str = "adaptive-spam.json";
const MIN_DOCUMENTS_PER_CLASS: u32 = 3;
const SPAM_THRESHOLD: f64 = 0.99;
const MAX_FEATURES: usize = 50_000;
const MAX_SAMPLES: usize = 50_000;
const MAX_MESSAGE_FEATURES: usize = 750;
const MAX_SCORED_FEATURES: usize = 150;

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    Spam,
    Ham,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Counts {
    spam: u32,
    ham: u32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Model {
    #[serde(default)]
    spam_documents: u32,
    #[serde(default)]
    ham_documents: u32,
    /// Hashed feature -> number of documents in each class containing it.
    #[serde(default)]
    features: BTreeMap<String, Counts>,
    /// Hashed Message-ID/fallback fingerprint -> last explicit label, preventing double training.
    #[serde(default)]
    samples: BTreeMap<String, Label>,
}

fn load() -> Model {
    std::fs::read_to_string(crate::paths::file(STATE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save(model: &Model) {
    if let Ok(json) = serde_json::to_string_pretty(model) {
        if let Err(e) = std::fs::write(crate::paths::file(STATE_FILE), json) {
            tracing::warn!("Could not save adaptive spam model: {}", e);
        }
    }
}

/// Stable FNV-1a: persisted feature keys remain valid between versions and platforms.
fn hash(value: &str) -> String {
    let mut result = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        result ^= u64::from(*byte);
        result = result.wrapping_mul(0x100000001b3);
    }
    format!("{result:016x}")
}

fn add_words(output: &mut HashSet<String>, prefix: &str, value: &str, limit: usize) {
    for word in value
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|word| (2..=40).contains(&word.chars().count()))
        .take(limit)
    {
        if output.len() >= MAX_MESSAGE_FEATURES {
            break;
        }
        output.insert(hash(&format!("{prefix}:{word}")));
    }
}

fn sender_address(value: &str) -> String {
    let trimmed = value.trim();
    trimmed
        .rfind('<')
        .and_then(|start| trimmed[start + 1..].find('>').map(|end| &trimmed[start + 1..start + 1 + end]))
        .unwrap_or(trimmed)
        .trim()
        .trim_matches('"')
        .to_lowercase()
}

fn features(subject: &str, from: &str, reply_to: &str, body: &str) -> HashSet<String> {
    let mut result = HashSet::new();
    add_words(&mut result, "subject", subject, 80);
    add_words(&mut result, "body", body, 600);

    let sender = sender_address(from);
    if !sender.is_empty() {
        result.insert(hash(&format!("sender:{sender}")));
        if let Some((_, domain)) = sender.rsplit_once('@') {
            result.insert(hash(&format!("domain:{domain}")));
        }
    }
    let reply = sender_address(reply_to);
    if !reply.is_empty() {
        result.insert(hash(&format!("reply:{reply}")));
        if let (Some((_, sender_domain)), Some((_, reply_domain))) = (sender.rsplit_once('@'), reply.rsplit_once('@')) {
            if sender_domain != reply_domain {
                result.insert(hash("header:reply-domain-mismatch"));
            }
        }
    }
    result
}

fn sample_key(card: &EmailCard) -> String {
    if !card.message_id.trim().is_empty() {
        return hash(&format!("message-id:{}", card.message_id.trim().to_lowercase()));
    }
    let body_prefix: String = card.body_text.chars().take(256).collect();
    hash(&format!(
        "fallback:{}\n{}\n{}\n{}",
        card.from_email.to_lowercase(),
        card.subject.to_lowercase(),
        card.date_ts,
        body_prefix.to_lowercase()
    ))
}

fn change_document_count(model: &mut Model, label: Label, increase: bool) {
    let count = match label {
        Label::Spam => &mut model.spam_documents,
        Label::Ham => &mut model.ham_documents,
    };
    if increase {
        *count = count.saturating_add(1);
    } else {
        *count = count.saturating_sub(1);
    }
}

fn change_features(model: &mut Model, message_features: &HashSet<String>, label: Label, increase: bool) {
    for feature in message_features {
        if !model.features.contains_key(feature) && (!increase || model.features.len() >= MAX_FEATURES) {
            continue;
        }
        let counts = model.features.entry(feature.clone()).or_default();
        let count = match label {
            Label::Spam => &mut counts.spam,
            Label::Ham => &mut counts.ham,
        };
        if increase {
            *count = count.saturating_add(1);
        } else {
            *count = count.saturating_sub(1);
        }
    }
    model.features.retain(|_, counts| counts.spam > 0 || counts.ham > 0);
}

/// Learn one explicit user decision. Repeating the same decision is a no-op; changing it moves
/// the sample between classes instead of counting the same email twice.
pub fn train(card: &EmailCard, label: Label) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut model = load();
    let key = sample_key(card);
    let message_features = features(&card.subject, &card.from_email, &card.reply_to, &card.body_text);
    if let Some(previous) = model.samples.get(&key).copied() {
        if previous == label {
            return;
        }
        change_document_count(&mut model, previous, false);
        change_features(&mut model, &message_features, previous, false);
    }
    change_document_count(&mut model, label, true);
    change_features(&mut model, &message_features, label, true);
    if !model.samples.contains_key(&key) && model.samples.len() >= MAX_SAMPLES {
        if let Some(oldest) = model.samples.keys().next().cloned() {
            model.samples.remove(&oldest);
        }
    }
    model.samples.insert(key, label);
    save(&model);
}

fn probability(model: &Model, message_features: &HashSet<String>) -> Option<f64> {
    if model.spam_documents < MIN_DOCUMENTS_PER_CLASS || model.ham_documents < MIN_DOCUMENTS_PER_CLASS {
        return None;
    }

    let mut evidence: Vec<f64> = message_features
        .iter()
        .filter_map(|feature| model.features.get(feature))
        .map(|counts| {
            let spam = (f64::from(counts.spam) + 1.0) / (f64::from(model.spam_documents) + 2.0);
            let ham = (f64::from(counts.ham) + 1.0) / (f64::from(model.ham_documents) + 2.0);
            (spam / ham).ln()
        })
        .filter(|weight| weight.abs() >= 0.35)
        .collect();
    evidence.sort_by(|a, b| b.abs().total_cmp(&a.abs()));
    evidence.truncate(MAX_SCORED_FEATURES);

    // User actions are not a representative sample of inbox prevalence: people naturally click
    // Spam more often than Not spam. A balanced prior prevents that imbalance from condemning an
    // otherwise unknown message.
    let log_odds = evidence.into_iter().sum::<f64>().clamp(-60.0, 60.0);
    Some(1.0 / (1.0 + (-log_odds).exp()))
}

/// Return a probability only after both classes have enough explicit examples.
#[cfg(test)]
pub fn score(email: &RawEmail) -> Option<f64> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let model = load();
    probability(&model, &features(&email.subject, &email.from, &email.reply_to, &email.body_text))
}

#[cfg(test)]
pub fn marks_spam(email: &RawEmail) -> Option<f64> {
    score(email).filter(|probability| *probability >= SPAM_THRESHOLD)
}

/// Score one refresh with a single model read, even when many messages arrived together.
pub fn marked_in_batch(emails: &[RawEmail]) -> HashMap<u64, f64> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let model = load();
    emails
        .iter()
        .filter_map(|email| {
            probability(&model, &features(&email.subject, &email.from, &email.reply_to, &email.body_text))
                .filter(|probability| *probability >= SPAM_THRESHOLD)
                .map(|probability| (email.uid, probability))
        })
        .collect()
}

#[cfg(test)]
pub fn reset() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = std::fs::remove_file(crate::paths::file(STATE_FILE));
}
