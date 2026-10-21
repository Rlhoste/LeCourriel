//! Instant local search over the emails the app holds in memory (no AI, no network).
//! Stage 1 of the search dialog: ranks emails on every keystroke; the best
//! candidates are then handed to the AI assistant (stage 2) on Enter.

use chrono::{NaiveDate, TimeZone, Utc};
use serde::Serialize;
use std::collections::HashMap;

use crate::models::EmailCard;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Folder {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Spam,
    /// Found on the IMAP server (older than the cached emails).
    Server,
}

impl Folder {
    pub fn as_str(self) -> &'static str {
        match self {
            Folder::Inbox => "inbox",
            Folder::Sent => "sent",
            Folder::Drafts => "drafts",
            Folder::Trash => "trash",
            Folder::Spam => "spam",
            Folder::Server => "server",
        }
    }

    /// Folder names in English or French (accents optional).
    pub fn parse(s: &str) -> Option<Folder> {
        match normalize(s).as_str() {
            "inbox" | "reception" | "boite" => Some(Folder::Inbox),
            "sent" | "envoyes" | "envoye" => Some(Folder::Sent),
            "drafts" | "draft" | "brouillons" | "brouillon" => Some(Folder::Drafts),
            "trash" | "corbeille" => Some(Folder::Trash),
            "spam" | "junk" | "indesirables" | "indesirable" => Some(Folder::Spam),
            "server" | "serveur" => Some(Folder::Server),
            _ => None,
        }
    }
}

/// Stable id for a result: "inbox:123", "sent:45"… (UIDs repeat across folders).
pub fn make_id(folder: Folder, uid: u32) -> String {
    format!("{}:{}", folder.as_str(), uid)
}

pub fn parse_id(id: &str) -> Option<(Folder, u32)> {
    let (f, uid) = id.split_once(':')?;
    Some((Folder::parse(f)?, uid.parse().ok()?))
}

/// Lowercase and strip accents, one output char per input char (so positions line up).
pub fn normalize(s: &str) -> String {
    s.chars().map(normalize_char).collect()
}

fn normalize_char(c: char) -> char {
    match c {
        'à' | 'â' | 'ä' | 'á' | 'ã' | 'À' | 'Â' | 'Ä' | 'Á' | 'æ' | 'Æ' => 'a',
        'ç' | 'Ç' => 'c',
        'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
        'î' | 'ï' | 'í' | 'ì' | 'Î' | 'Ï' => 'i',
        'ô' | 'ö' | 'ó' | 'ò' | 'õ' | 'Ô' | 'Ö' | 'œ' | 'Œ' => 'o',
        'ù' | 'û' | 'ü' | 'ú' | 'Ù' | 'Û' | 'Ü' => 'u',
        'ÿ' | 'Ÿ' => 'y',
        'ñ' | 'Ñ' => 'n',
        '’' => '\'',
        c => c.to_lowercase().next().unwrap_or(c),
    }
}

/// Words that carry no meaning for search (natural-language requests are full of them).
const STOPWORDS: &[&str] = &[
    "le", "la", "les", "de", "des", "du", "un", "une", "et", "ou", "a", "au", "aux", "en", "pour", "par",
    "sur", "avec", "dans", "que", "qui", "quoi", "mes", "mon", "ma", "me", "moi", "ce", "ces", "cet",
    "cette", "est", "sont", "il", "elle", "on", "nous", "vous", "tous", "tout", "toutes",
    "mail", "mails", "email", "emails", "e-mail", "e-mails", "courriel", "courriels", "message", "messages",
    "the", "an", "and", "or", "of", "to", "for", "from", "in", "on", "with", "my", "me", "all", "about",
    "is", "are", "what", "which", "that", "this", "these", "please",
];

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Filters {
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub project: Option<String>,
    pub unread: bool,
    pub before: Option<NaiveDate>,
    pub after: Option<NaiveDate>,
    pub folder: Option<Folder>,
}

impl Filters {
    pub fn is_empty(&self) -> bool {
        *self == Filters::default()
    }
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(s, "%d/%m/%Y"))
        .ok()
}

/// Split a query into search terms and filters. Quoted text is one term.
/// Filters: de:/from:, à:/to:, projet:/project:, avant:/before:, après:/after:,
/// dossier:/folder:, and "non lu" / "unread".
pub fn parse_query(q: &str) -> (Vec<String>, Filters) {
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in q.chars() {
        match c {
            '"' | '«' | '»' => {
                if quoted || !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
                quoted = !quoted && c != '»';
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }

    let mut filters = Filters::default();
    let mut terms = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let raw = tokens[i].trim();
        let norm = normalize(raw);
        // "non lu" (two words), "non-lu", "nonlu", "unread", "is:unread"
        if norm == "non" && tokens.get(i + 1).map(|t| normalize(t)) == Some("lu".into()) {
            filters.unread = true;
            i += 2;
            continue;
        }
        if matches!(norm.as_str(), "non-lu" | "nonlu" | "unread" | "is:unread" | "non-lus" | "nonlus") {
            filters.unread = true;
            i += 1;
            continue;
        }
        if let Some((key, value)) = norm.split_once(':') {
            let value = value.trim();
            if !value.is_empty() {
                let handled = match key {
                    "de" | "from" => { filters.from.push(value.to_string()); true }
                    "a" | "to" => { filters.to.push(value.to_string()); true }
                    "projet" | "project" => { filters.project = Some(value.to_string()); true }
                    "avant" | "before" => { filters.before = parse_date(value); filters.before.is_some() }
                    "apres" | "after" => { filters.after = parse_date(value); filters.after.is_some() }
                    "dossier" | "folder" => { filters.folder = Folder::parse(value); filters.folder.is_some() }
                    _ => false,
                };
                if handled {
                    i += 1;
                    continue;
                }
            }
        }
        // Plain word: drop French elisions (l', d', qu'…) and punctuation, then stopwords.
        let mut word = norm.as_str();
        for prefix in ["l'", "d'", "j'", "qu'", "m'", "t'", "s'", "n'", "c'"] {
            if let Some(rest) = word.strip_prefix(prefix) {
                word = rest;
            }
        }
        let word = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '@' && c != '.' && c != ' ');
        let word = word.trim_end_matches('.');
        if !word.is_empty() && (word.contains(' ') || !STOPWORDS.contains(&word)) {
            terms.push(word.to_string());
        }
        i += 1;
    }
    (terms, filters)
}

/// One field of an email with its search weight.
struct Field {
    text: String,
    weight: f32,
}

fn fields(card: &EmailCard) -> Vec<Field> {
    let f = |s: &str, w: f32| Field { text: normalize(s), weight: w };
    vec![
        f(&card.subject, 3.0),
        f(&format!("{} {}", card.from_name, card.from_email), 3.0),
        f(&card.summary, 2.0),
        f(card.task.as_deref().unwrap_or(""), 2.0),
        f(&format!("{} {} {}", card.to, card.cc, card.bcc), 1.5),
        f(card.project.as_deref().unwrap_or(""), 1.5),
        f(&card.body_text, 1.0),
    ]
}

/// How well a term matches a field: whole word or word start counts fully,
/// a match inside a word counts half.
fn term_weight(field: &str, term: &str) -> f32 {
    let mut best = 0.0f32;
    for (pos, _) in field.match_indices(term) {
        let at_word_start = pos == 0
            || !field[..pos].chars().next_back().map_or(false, |c| c.is_alphanumeric());
        best = best.max(if at_word_start { 1.0 } else { 0.5 });
        if best == 1.0 {
            break;
        }
    }
    best
}

/// (number of terms matched, weighted score). Terms are already normalized.
pub fn score(card: &EmailCard, terms: &[String]) -> (usize, f32) {
    if terms.is_empty() {
        return (0, 0.0);
    }
    let fields = fields(card);
    let mut matched = 0;
    let mut total = 0.0;
    for term in terms {
        let s: f32 = fields.iter().map(|f| f.weight * term_weight(&f.text, term)).sum();
        if s > 0.0 {
            matched += 1;
            total += s;
        }
    }
    (matched, total)
}

fn passes(folder: Folder, card: &EmailCard, f: &Filters) -> bool {
    if let Some(want) = f.folder {
        if want != folder {
            return false;
        }
    }
    if f.unread && card.is_read {
        return false;
    }
    let from = normalize(&format!("{} {}", card.from_name, card.from_email));
    if !f.from.iter().all(|v| from.contains(v.as_str())) {
        return false;
    }
    let to = normalize(&format!("{} {} {}", card.to, card.cc, card.bcc));
    if !f.to.iter().all(|v| to.contains(v.as_str())) {
        return false;
    }
    if let Some(p) = &f.project {
        if !normalize(card.project.as_deref().unwrap_or("")).contains(p.as_str()) {
            return false;
        }
    }
    let date = (card.date_ts > 0).then(|| Utc.timestamp_opt(card.date_ts, 0).single()).flatten().map(|d| d.date_naive());
    if let Some(before) = f.before {
        if date.map_or(true, |d| d >= before) {
            return false;
        }
    }
    if let Some(after) = f.after {
        if date.map_or(true, |d| d <= after) {
            return false;
        }
    }
    true
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SearchHit {
    pub id: String,
    pub folder: Folder,
    pub uid: u32,
    pub subject: String,
    pub from_name: String,
    pub date_str: String,
    /// Unix time (0 when unknown): the UI formats it in its language.
    pub date_ts: i64,
    pub summary: String,
    /// A short excerpt around the first match in the body (plain text).
    pub snippet: String,
    pub is_read: bool,
    pub matched: usize,
    pub score: f32,
}

/// ~160 characters of the body around the first term found, or its beginning.
fn snippet(card: &EmailCard, terms: &[String]) -> String {
    let body: Vec<char> = card.body_text.split_whitespace().collect::<Vec<_>>().join(" ").chars().collect();
    let norm: String = body.iter().map(|&c| normalize_char(c)).collect();
    let start = terms
        .iter()
        .filter_map(|t| norm.find(t.as_str()))
        .min()
        .map(|byte_pos| norm[..byte_pos].chars().count())
        .unwrap_or(0);
    let from = start.saturating_sub(50);
    let to = (from + 160).min(body.len());
    let mut s: String = body[from..to].iter().collect();
    if from > 0 {
        s.insert(0, '…');
    }
    if to < body.len() {
        s.push('…');
    }
    s
}

pub fn make_hit(folder: Folder, card: &EmailCard, terms: &[String], matched: usize, score: f32) -> SearchHit {
    SearchHit {
        id: make_id(folder, card.uid),
        folder,
        uid: card.uid,
        subject: card.subject.clone(),
        from_name: card.from_name.clone(),
        date_str: card.date_str.clone(),
        date_ts: card.date_ts,
        summary: card.summary.clone(),
        snippet: snippet(card, terms),
        is_read: card.is_read,
        matched,
        score,
    }
}

/// Rank the emails for a query. Without terms, the most recent emails matching the filters.
pub fn search_local(index: &[(Folder, EmailCard)], query: &str, limit: usize) -> Vec<SearchHit> {
    let (terms, filters) = parse_query(query);
    let mut hits: Vec<(usize, f32, i64, SearchHit)> = index
        .iter()
        .filter(|(folder, card)| passes(*folder, card, &filters))
        .filter_map(|(folder, card)| {
            let (matched, sc) = score(card, &terms);
            if !terms.is_empty() && matched == 0 {
                return None;
            }
            Some((matched, sc, card.date_ts, make_hit(*folder, card, &terms, matched, sc)))
        })
        .collect();
    // Emails matching more of the words first, then the best score, then the most recent.
    hits.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
            .then(b.2.cmp(&a.2))
    });
    hits.into_iter().take(limit).map(|(_, _, _, h)| h).collect()
}

/// Split an address list on commas outside quotes and <…>.
pub fn split_addresses(list: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut in_quote, mut in_angle) = (false, false);
    for c in list.chars() {
        match c {
            '"' => in_quote = !in_quote,
            '<' if !in_quote => in_angle = true,
            '>' if !in_quote => in_angle = false,
            _ => {}
        }
        if c == ',' && !in_quote && !in_angle {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// The address part of "Name <a@b>" (or the whole string), lowercased.
fn address_of(entry: &str) -> String {
    match (entry.rfind('<'), entry.rfind('>')) {
        (Some(l), Some(r)) if l < r => entry[l + 1..r].trim().to_lowercase(),
        _ => entry.trim().to_lowercase(),
    }
}

/// Known correspondents, most frequent first: "Name <address>" (for drafting emails).
pub fn contacts(index: &[(Folder, EmailCard)], limit: usize) -> Vec<String> {
    let mut seen: HashMap<String, (usize, String)> = HashMap::new();
    let mut add = |entry: String| {
        let addr = address_of(&entry);
        if !addr.contains('@') {
            return;
        }
        let e = seen.entry(addr).or_insert((0, entry.clone()));
        e.0 += 1;
        if !e.1.contains('<') && entry.contains('<') {
            e.1 = entry; // prefer the form with a display name
        }
    };
    for (_, c) in index {
        if !c.from_email.is_empty() {
            add(if c.from_name.is_empty() || c.from_name == c.from_email {
                c.from_email.clone()
            } else {
                format!("{} <{}>", c.from_name.trim_matches('"'), c.from_email)
            });
        }
        for e in split_addresses(&c.to).into_iter().chain(split_addresses(&c.cc)) {
            add(e);
        }
    }
    let mut v: Vec<(usize, String)> = seen.into_values().collect();
    v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    v.into_iter().take(limit).map(|(_, s)| s).collect()
}
