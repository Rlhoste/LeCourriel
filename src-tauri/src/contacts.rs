//! Local address book learned from Sent mail and edited by the user.

use crate::models::RawEmail;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::sync::Mutex;

const FILE: &str = "contacts.json";
const MAX_SEEN_SENT: usize = 20_000;
const PUBLIC_MAIL_DOMAINS: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "outlook.com",
    "hotmail.com",
    "live.com",
    "yahoo.com",
    "icloud.com",
    "me.com",
    "proton.me",
    "protonmail.com",
    "aol.com",
    "gmx.com",
    "gmx.de",
    "mail.com",
];
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Contact {
    pub address: String,
    pub name: String,
    pub company: String,
    #[serde(default)]
    pub sent_count: u32,
    #[serde(default)]
    pub last_sent: i64,
    #[serde(default)]
    pub trusted: bool,
    #[serde(default)]
    pub blocked: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Company {
    pub name: String,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub trusted: bool,
    #[serde(default)]
    pub blocked: bool,
}

/// A sender seen in accepted incoming mail, but never confirmed by outgoing mail.
/// Potential contacts never contribute trust or compose autocomplete.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PotentialContact {
    pub address: String,
    pub name: String,
    #[serde(default)]
    pub received_count: u32,
    #[serde(default)]
    pub last_received: i64,
    #[serde(default)]
    pub authenticated: bool,
    #[serde(default)]
    pub likely_person: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Directory {
    #[serde(default)]
    pub contacts: Vec<Contact>,
    #[serde(default)]
    pub companies: Vec<Company>,
    #[serde(default)]
    pub potential_contacts: Vec<PotentialContact>,
    /// Hashes of Sent messages already counted. No message content is persisted here.
    #[serde(default)]
    seen_sent: BTreeSet<String>,
    #[serde(default)]
    seen_incoming: BTreeSet<String>,
    /// Hashed account/folder identities whose complete header history was scanned.
    #[serde(default)]
    completed_scans: BTreeSet<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DirectoryView {
    pub contacts: Vec<Contact>,
    pub companies: Vec<Company>,
    pub potential_contacts: Vec<PotentialContact>,
}

impl From<Directory> for DirectoryView {
    fn from(directory: Directory) -> Self {
        Self {
            contacts: directory.contacts,
            companies: directory.companies,
            potential_contacts: directory.potential_contacts,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TrustSignals {
    pub trusted_senders: HashSet<String>,
    pub trusted_domains: HashSet<String>,
    pub blocked_senders: HashSet<String>,
    pub blocked_domains: HashSet<String>,
}

fn read_unlocked() -> Directory {
    std::fs::read_to_string(crate::paths::file(FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_unlocked(directory: &Directory) -> Result<(), String> {
    let json = serde_json::to_string_pretty(directory).map_err(|e| e.to_string())?;
    std::fs::write(crate::paths::file(FILE), json).map_err(|e| e.to_string())
}

pub fn load() -> Directory {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read_unlocked()
}

fn normalize_address(value: &str) -> Option<String> {
    let address = value.trim().trim_matches(['<', '>', '"']).to_lowercase();
    let (local, domain) = address.rsplit_once('@')?;
    (!local.is_empty() && !domain.is_empty() && !address.chars().any(char::is_whitespace) && address.len() <= 320)
        .then_some(address)
}

fn normalize_domain(value: &str) -> Option<String> {
    let value = value.trim().trim_start_matches('@').trim_end_matches('.').to_lowercase();
    (!value.is_empty()
        && value.len() <= 253
        && value.contains('.')
        && value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.')))
    .then_some(value)
}

fn parse_entry(value: &str) -> Option<(String, String)> {
    let value = value.trim();
    if let (Some(left), Some(right)) = (value.rfind('<'), value.rfind('>')) {
        if left < right {
            let address = normalize_address(&value[left + 1..right])?;
            let name = value[..left].trim().trim_matches('"').trim().to_string();
            return Some((name, address));
        }
    }
    normalize_address(value).map(|address| (String::new(), address))
}

fn automated_sender(name: &str, address: &str, security: &crate::models::MailSecurity) -> bool {
    let local = address.split('@').next().unwrap_or_default().to_lowercase();
    let compact: String = local.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let name = name.to_lowercase().replace(['-', '_'], " ");
    const AUTOMATED: &[&str] = &[
        "noreply",
        "donotreply",
        "mailerdaemon",
        "postmaster",
        "automated",
        "automatic",
        "notifications",
        "notification",
        "newsletter",
        "bounce",
    ];
    AUTOMATED.iter().any(|word| compact.contains(word))
        || ["no reply", "do not reply", "mailer daemon", "automated message", "newsletter"]
            .iter()
            .any(|phrase| name.contains(phrase))
        || !security.list_id.trim().is_empty()
        || !security.list_unsubscribe.trim().is_empty()
        || matches!(security.precedence.trim().to_ascii_lowercase().as_str(), "bulk" | "list" | "junk")
}

fn likely_person(name: &str, address: &str) -> bool {
    let local = address.split('@').next().unwrap_or_default().to_ascii_lowercase();
    const GENERIC: &[&str] = &[
        "admin", "billing", "contact", "hello", "info", "office", "sales", "security", "service", "support", "team",
        "updates",
    ];
    let generic = GENERIC.iter().any(|value| local == *value || local.starts_with(&format!("{value}+")));
    !generic && !name.trim().is_empty() && name.trim() != address
}

fn hash(value: &str) -> String {
    let mut result = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        result ^= u64::from(*byte);
        result = result.wrapping_mul(0x100000001b3);
    }
    format!("{result:016x}")
}

fn sent_key(email: &RawEmail) -> String {
    if !email.message_id.trim().is_empty() {
        return hash(&email.message_id.trim().to_lowercase());
    }
    hash(&format!("{}\n{}\n{}", email.uid, email.date.map(|d| d.timestamp()).unwrap_or_default(), email.to))
}

pub fn scan_key(kind: &str, host: &str, username: &str, folder: &str) -> String {
    hash(&format!("{kind}\n{}\n{}\n{}", host.to_lowercase(), username.to_lowercase(), folder.to_lowercase()))
}

pub fn scan_needed(key: &str) -> bool {
    !load().completed_scans.contains(key)
}

fn add_recipient(directory: &mut Directory, entry: &str, at: i64, own_addresses: &HashSet<String>) {
    let Some((name, address)) = parse_entry(entry) else { return };
    if own_addresses.contains(&address) {
        return;
    }
    directory.potential_contacts.retain(|potential| potential.address != address);
    if let Some(contact) = directory.contacts.iter_mut().find(|contact| contact.address == address) {
        contact.sent_count = contact.sent_count.saturating_add(1);
        contact.last_sent = contact.last_sent.max(at);
        if contact.name.is_empty() && !name.is_empty() {
            contact.name = name;
        }
        return;
    }
    let company = address
        .rsplit_once('@')
        .and_then(|(_, domain)| {
            directory
                .companies
                .iter()
                .find(|company| company.domains.iter().any(|known| known == domain))
                .map(|company| company.name.clone())
        })
        .unwrap_or_default();
    directory.contacts.push(Contact {
        address,
        name,
        company,
        sent_count: 1,
        last_sent: at,
        trusted: false,
        blocked: false,
    });
}

/// Learn recipients from Sent mail exactly once, across refreshes and restarts.
fn ingest_sent_unlocked(directory: &mut Directory, emails: &[RawEmail], own_addresses: &HashSet<String>) -> bool {
    let mut changed = false;
    for email in emails {
        let key = sent_key(email);
        if !directory.seen_sent.insert(key) {
            continue;
        }
        changed = true;
        let at = email.date.map(|date| date.timestamp()).unwrap_or_default();
        for entry in crate::search::split_addresses(&email.to)
            .into_iter()
            .chain(crate::search::split_addresses(&email.cc))
            .chain(crate::search::split_addresses(&email.bcc))
        {
            add_recipient(directory, &entry, at, own_addresses);
        }
    }
    changed
}

pub fn ingest_sent(emails: &[RawEmail], own_addresses: &HashSet<String>) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut directory = read_unlocked();
    let changed = ingest_sent_unlocked(&mut directory, emails, own_addresses);
    while directory.seen_sent.len() > MAX_SEEN_SENT {
        let Some(first) = directory.seen_sent.iter().next().cloned() else { break };
        directory.seen_sent.remove(&first);
    }
    if changed {
        directory.contacts.sort_by(|a, b| {
            b.last_sent
                .cmp(&a.last_sent)
                .then_with(|| b.sent_count.cmp(&a.sent_count))
                .then_with(|| a.address.cmp(&b.address))
        });
        if let Err(e) = write_unlocked(&directory) {
            tracing::warn!("Could not save contacts: {}", e);
        }
    }
}

pub fn complete_sent_scan(key: &str, emails: &[RawEmail], own_addresses: &HashSet<String>) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut directory = read_unlocked();
    ingest_sent_unlocked(&mut directory, emails, own_addresses);
    directory.completed_scans.insert(key.to_string());
    write_unlocked(&directory)
}

/// Record accepted incoming senders separately. They remain untrusted until the user sends mail
/// to them or explicitly promotes them.
pub fn ingest_potential(emails: &[RawEmail], own_addresses: &HashSet<String>) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut directory = read_unlocked();
    let before = directory.potential_contacts.len();
    directory.potential_contacts.retain(|potential| {
        !automated_sender(&potential.name, &potential.address, &crate::models::MailSecurity::default())
    });
    let mut changed = directory.potential_contacts.len() != before;
    for potential in &mut directory.potential_contacts {
        let is_person = likely_person(&potential.name, &potential.address);
        changed |= potential.likely_person != is_person;
        potential.likely_person = is_person;
    }
    for email in emails {
        let key = sent_key(email);
        if !directory.seen_incoming.insert(key) {
            continue;
        }
        let Some((name, address)) = parse_entry(&email.from) else { continue };
        if own_addresses.contains(&address)
            || directory.contacts.iter().any(|contact| contact.address == address)
            || automated_sender(&name, &address, &email.security)
        {
            continue;
        }
        changed = true;
        let at = email.date.map(|date| date.timestamp()).unwrap_or_default();
        let authenticated = crate::spam_guard::dmarc_passes(&email.security);
        if let Some(potential) = directory.potential_contacts.iter_mut().find(|item| item.address == address) {
            potential.received_count = potential.received_count.saturating_add(1);
            potential.last_received = potential.last_received.max(at);
            potential.authenticated |= authenticated;
            if potential.name.is_empty() && !name.is_empty() {
                potential.name = name;
            }
            potential.likely_person = likely_person(&potential.name, &potential.address);
        } else {
            let is_person = likely_person(&name, &address);
            directory.potential_contacts.push(PotentialContact {
                address,
                name,
                received_count: 1,
                last_received: at,
                authenticated,
                likely_person: is_person,
            });
        }
    }
    while directory.seen_incoming.len() > MAX_SEEN_SENT {
        let Some(first) = directory.seen_incoming.iter().next().cloned() else { break };
        directory.seen_incoming.remove(&first);
    }
    if changed {
        directory.potential_contacts.sort_by(|a, b| {
            b.likely_person
                .cmp(&a.likely_person)
                .then_with(|| b.authenticated.cmp(&a.authenticated))
                .then_with(|| b.last_received.cmp(&a.last_received))
                .then_with(|| a.address.cmp(&b.address))
        });
        if let Err(e) = write_unlocked(&directory) {
            tracing::warn!("Could not save potential contacts: {}", e);
        }
    }
}

pub fn complete_potential_scan(key: &str, emails: &[RawEmail], own_addresses: &HashSet<String>) -> Result<(), String> {
    ingest_potential(emails, own_addresses);
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut directory = read_unlocked();
    directory.completed_scans.insert(key.to_string());
    write_unlocked(&directory)
}

pub fn save_contact(mut contact: Contact) -> Result<Directory, String> {
    let address = normalize_address(&contact.address).ok_or("Invalid email address")?;
    contact.address = address.clone();
    contact.name = contact.name.trim().chars().take(200).collect();
    contact.company = contact.company.trim().chars().take(200).collect();
    if contact.trusted {
        contact.blocked = false;
    }
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut directory = read_unlocked();
    if let Some(existing) = directory.contacts.iter_mut().find(|existing| existing.address == address) {
        contact.sent_count = existing.sent_count;
        contact.last_sent = existing.last_sent;
        *existing = contact;
    } else {
        directory.contacts.push(contact);
    }
    directory.potential_contacts.retain(|potential| potential.address != address);
    write_unlocked(&directory)?;
    Ok(directory)
}

pub fn delete_contact(address: &str) -> Result<Directory, String> {
    let address = normalize_address(address).ok_or("Invalid email address")?;
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut directory = read_unlocked();
    directory.contacts.retain(|contact| contact.address != address);
    write_unlocked(&directory)?;
    Ok(directory)
}

pub fn save_company(mut company: Company) -> Result<Directory, String> {
    company.name = company.name.trim().chars().take(200).collect();
    if company.name.is_empty() {
        return Err("Company name is required".into());
    }
    company.domains = company.domains.iter().filter_map(|domain| normalize_domain(domain)).collect();
    company.domains.sort();
    company.domains.dedup();
    if company.trusted && company.domains.iter().any(|domain| PUBLIC_MAIL_DOMAINS.contains(&domain.as_str())) {
        return Err("A public email provider cannot be trusted as one company".into());
    }
    if company.trusted {
        company.blocked = false;
    }
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut directory = read_unlocked();
    let company_name = company.name.clone();
    let company_domains = company.domains.clone();
    if let Some(existing) =
        directory.companies.iter_mut().find(|existing| existing.name.eq_ignore_ascii_case(&company.name))
    {
        *existing = company;
    } else {
        directory.companies.push(company);
    }
    for contact in &mut directory.contacts {
        let domain = contact.address.rsplit_once('@').map(|(_, domain)| domain);
        if contact.company.is_empty()
            && domain.is_some_and(|domain| company_domains.iter().any(|known| known == domain))
        {
            contact.company.clone_from(&company_name);
        }
    }
    directory.companies.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    write_unlocked(&directory)?;
    Ok(directory)
}

pub fn delete_company(name: &str) -> Result<Directory, String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut directory = read_unlocked();
    directory.companies.retain(|company| !company.name.eq_ignore_ascii_case(name.trim()));
    for contact in &mut directory.contacts {
        if contact.company.eq_ignore_ascii_case(name.trim()) {
            contact.company.clear();
        }
    }
    write_unlocked(&directory)?;
    Ok(directory)
}

pub fn trust_signals(directory: &Directory) -> TrustSignals {
    let mut signals = TrustSignals::default();
    for contact in &directory.contacts {
        if contact.blocked {
            signals.blocked_senders.insert(contact.address.clone());
        } else if contact.trusted || contact.sent_count > 0 {
            signals.trusted_senders.insert(contact.address.clone());
        }
    }
    for company in &directory.companies {
        let target = if company.blocked {
            Some(&mut signals.blocked_domains)
        } else if company.trusted {
            Some(&mut signals.trusted_domains)
        } else {
            None
        };
        if let Some(target) = target {
            target.extend(company.domains.iter().cloned());
        }
    }
    signals
}

#[cfg(test)]
pub fn reset() {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = std::fs::remove_file(crate::paths::file(FILE));
}
