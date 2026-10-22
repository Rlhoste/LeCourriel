//! Auto-scan: looking through the other mail apps on this computer for accounts the user
//! can add with their server settings already filled in. Thunderbird and Betterbird keep
//! those settings in plain text in their profile. Their passwords sit in other files,
//! encrypted, and those are never opened here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::auto_setup::usable_port;
use crate::dto::{FoundAccountDto, ServerDto};

/// The accounts of a profile, keyed by their pref name (always lowercase).
type Prefs = BTreeMap<String, String>;

/// Every account the mail apps on this computer hold, without any password. An app that
/// is not installed, a profile without a prefs.js: both simply give nothing.
pub fn scan() -> Vec<FoundAccountDto> {
    let mut found: Vec<FoundAccountDto> = Vec::new();
    for (root, client) in roots() {
        let dirs = profile_dirs(&read(&root.join("profiles.ini")), &root);
        // A root without a profiles.ini is itself a profile directory.
        let dirs = if dirs.is_empty() { vec![root.clone()] } else { dirs };
        for dir in dirs {
            let prefs = read(&dir.join("prefs.js"));
            if !prefs.is_empty() {
                found.extend(parse_prefs(&prefs, client));
            }
        }
    }
    dedupe(found)
}

/// The directories that hold a `profiles.ini`, with the app that owns each one. Betterbird
/// uses Thunderbird's directory, so it is not scanned twice there.
fn roots() -> Vec<(PathBuf, &'static str)> {
    let mut out: Vec<(PathBuf, &'static str)> = Vec::new();
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let appdata = PathBuf::from(appdata);
            out.push((appdata.join("Thunderbird"), "Thunderbird"));
            out.push((PathBuf::from(appdata).join("Betterbird"), "Betterbird"));
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            for (dir, client) in [
                (".thunderbird", "Thunderbird"),
                (".betterbird", "Betterbird"),
                ("snap/thunderbird/common/.thunderbird", "Thunderbird"),
                (".var/app/org.mozilla.Thunderbird/.thunderbird", "Thunderbird"),
                (".var/app/eu.betterbird.Betterbird/.thunderbird", "Betterbird"),
            ] {
                out.push((home.join(dir), client));
            }
        }
    }
    out
}

/// A whole small text file, "" when it is not there or cannot be read.
fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// The profiles a `profiles.ini` names, as directories. `IsRelative=0` writes a path of
/// its own, anything else a path under `root` (the folder holding the file). The
/// `[Install…]` sections only point at the default profile, so they bring no second one.
pub fn profile_dirs(profiles_ini: &str, root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut in_profile = false;
    let mut relative = true;
    for line in profiles_ini.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            let section = line.trim_start_matches('[').split(']').next().unwrap_or("");
            in_profile = section.as_bytes().get(..7).is_some_and(|b| b.eq_ignore_ascii_case(b"profile"));
            relative = true;   // a section that says nothing writes a relative path
            continue;
        }
        if !in_profile {
            continue;
        }
        if let Some(value) = line.strip_prefix("IsRelative=") {
            relative = value.trim() != "0";
        } else if let Some(value) = line.strip_prefix("Path=") {
            let value = value.trim().trim_matches('"');
            if value.is_empty() {
                continue;
            }
            let dir = if relative {
                root.join(value.replace('/', std::path::MAIN_SEPARATOR_STR))
            } else {
                PathBuf::from(value)
            };
            if !out.contains(&dir) {
                out.push(dir);
            }
        }
    }
    out
}

/// The accounts of one `prefs.js`, as Auto-scan needs them. `client` is the app's name,
/// which the file itself does not hold.
pub fn parse_prefs(prefs_js: &str, client: &str) -> Vec<FoundAccountDto> {
    let prefs = values(prefs_js);
    let mut out = Vec::new();
    for account in account_order(&prefs) {
        let server = name(&prefs, &format!("mail.account.{account}.server"));
        let stype = name(&prefs, &format!("mail.server.{server}.type"));
        // "none" is Local Folders, "rss" a news feed and "nntp" a discussion group.
        if stype != "imap" && stype != "pop3" {
            continue;
        }
        let identity = list(&prefs, &format!("mail.account.{account}.identities"))
            .into_iter()
            .next()
            .unwrap_or_default();
        let email = text(&prefs, &format!("mail.identity.{identity}.useremail"));
        if email.is_empty() {
            continue;
        }
        let label = text(&prefs, &format!("mail.server.{server}.name"));
        let mut found = FoundAccountDto {
            client: client.to_string(),
            label: if label.is_empty() { email.clone() } else { label },
            name: text(&prefs, &format!("mail.identity.{identity}.fullname")),
            email: email.clone(),
            pop3: stype == "pop3",
            oauth: number(&prefs, &format!("mail.server.{server}.authmethod")) == Some(10),
            ..Default::default()
        };
        // A POP3 server brings nothing this app can use, and says so by being POP3.
        if !found.pop3 {
            let host = text(&prefs, &format!("mail.server.{server}.hostname"));
            let socket = socket_name(number(&prefs, &format!("mail.server.{server}.sockettype")));
            let port = number(&prefs, &format!("mail.server.{server}.port")).map(|p| p.to_string()).unwrap_or_default();
            let user = or_else(text(&prefs, &format!("mail.server.{server}.username")), &email);
            match usable_port("imap", &socket, &port) {
                Ok(port) => found.imap = Some(ServerDto { host, port, user }),
                Err(why) => found.imap_issue = why.to_string(),
            }
        }
        if let Some(smtp) = smtp_server(&prefs, &identity) {
            let host = text(&prefs, &format!("mail.smtpserver.{smtp}.hostname"));
            let socket = socket_name(number(&prefs, &format!("mail.smtpserver.{smtp}.try_ssl")));
            let port = number(&prefs, &format!("mail.smtpserver.{smtp}.port")).map(|p| p.to_string()).unwrap_or_default();
            let user = or_else(text(&prefs, &format!("mail.smtpserver.{smtp}.username")), &email);
            match usable_port("smtp", &socket, &port) {
                Ok(port) => found.smtp = Some(ServerDto { host, port, user }),
                Err(why) => found.smtp_issue = why.to_string(),
            }
            found.oauth |= number(&prefs, &format!("mail.smtpserver.{smtp}.authmethod")) == Some(10);
        }
        out.push(found);
    }
    out
}

/// One account per email address: the first spelling wins, and an IMAP entry beats a POP3
/// one, since the same profile is often seen through both apps.
pub fn dedupe(list: Vec<FoundAccountDto>) -> Vec<FoundAccountDto> {
    let mut out: Vec<FoundAccountDto> = Vec::new();
    for account in list {
        match out.iter().position(|o| o.email.eq_ignore_ascii_case(&account.email)) {
            Some(i) if out[i].pop3 && !account.pop3 => out[i] = account,
            Some(_) => {}
            None => out.push(account),
        }
    }
    out
}

/// The accounts of a profile, in the order the app shows them: the ones it lists first,
/// then any other it never mentioned.
fn account_order(prefs: &Prefs) -> Vec<String> {
    let mut known: Vec<String> = prefs
        .keys()
        .filter_map(|k| k.strip_prefix("mail.account."))
        .filter_map(|rest| rest.split_once('.'))
        .filter(|(_, rest)| *rest == "server")
        .map(|(account, _)| account.to_string())
        .collect();
    known.sort_by_key(|a| (a.parse::<u32>().unwrap_or(u32::MAX), a.clone()));
    let mut out: Vec<String> = list(prefs, "mail.accountmanager.accounts")
        .into_iter()
        .filter(|a| known.contains(a))
        .collect();
    let rest: Vec<String> = known.into_iter().filter(|a| !out.contains(a)).collect();
    out.extend(rest);
    out
}

/// The outgoing server of an identity: the one it names, else the profile's default, else
/// its only one. A server with no host is no server at all.
fn smtp_server(prefs: &Prefs, identity: &str) -> Option<String> {
    let known = |key: String| {
        (!key.is_empty() && !text(prefs, &format!("mail.smtpserver.{key}.hostname")).is_empty()).then_some(key)
    };
    if let Some(key) = known(name(prefs, &format!("mail.identity.{identity}.smtpserver"))) {
        return Some(key);
    }
    if let Some(key) = known(name(prefs, "mail.smtp.defaultserver")) {
        return Some(key);
    }
    let listed = list(prefs, "mail.smtpservers");
    if listed.len() == 1 {
        return known(listed[0].clone());
    }
    None
}

/// Thunderbird's socketType / try_ssl number as Auto-setup names it. The old value 1,
/// "try STARTTLS", is the ordinary STARTTLS case today.
fn socket_name(number: Option<u32>) -> String {
    match number {
        Some(3) => "SSL",
        Some(2) | Some(1) => "STARTTLS",
        _ => "plain",
    }
    .to_string()
}

// ── prefs.js ──────────────────────────────────────────────────────────────────

/// Every `user_pref("key", value);` of a prefs.js. Unknown keys are kept and never used,
/// and anything that is not such a line is passed over: a profile from another version
/// must still give its accounts.
fn values(prefs_js: &str) -> Prefs {
    let mut out = Prefs::new();
    for line in prefs_js.lines() {
        let Some(rest) = line.trim_start().strip_prefix("user_pref") else { continue };
        let Some(rest) = rest.trim_start().strip_prefix('(') else { continue };
        let rest = rest.trim_start();
        let Some((key, end)) = js_string(rest) else { continue };
        let value = rest[end..].trim_start().strip_prefix(',').map(|v| v.trim_start()).unwrap_or("");
        let value = match js_string(value) {
            Some((v, _)) => v,
            // A number or a boolean: whatever stands between the comma and the closing ')'
            None => value.split([')', ';', ',']).next().unwrap_or("").trim().to_string(),
        };
        if !key.is_empty() {
            out.insert(key.to_lowercase(), value);
        }
    }
    out
}

/// A JS string at the start of `s`: its text with the escapes resolved, and where it ends.
/// None when `s` does not begin with one.
fn js_string(s: &str) -> Option<(String, usize)> {
    let mut chars = s.char_indices();
    if chars.next()?.1 != '"' {
        return None;
    }
    let mut out = String::new();
    loop {
        let (i, c) = chars.next()?;
        match c {
            '"' => return Some((out, i + 1)),
            '\\' => match chars.next()? {
                (_, 'u') => {
                    // \uXXXX, the way prefs.js writes the letters of a name.
                    let hex = s.get(i + 2..)?.get(..4)?;
                    let code = u32::from_str_radix(hex, 16).ok()?;
                    out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                    for _ in 0..4 {
                        chars.next();
                    }
                }
                (_, 'n') => out.push('\n'),
                (_, 'r') => out.push('\r'),
                (_, 't') => out.push('\t'),
                (_, other) => out.push(other),
            },
            other => out.push(other),
        }
    }
}

/// A value as written, "" when the pref is absent.
fn text(prefs: &Prefs, key: &str) -> String {
    prefs.get(key).cloned().unwrap_or_default()
}

/// A value holding a name or a key: trimmed, and lowercased like the pref names are.
fn name(prefs: &Prefs, key: &str) -> String {
    text(prefs, key).trim().to_lowercase()
}

/// A value holding a number, None when it is absent or is not one.
fn number(prefs: &Prefs, key: &str) -> Option<u32> {
    text(prefs, key).trim().parse().ok()
}

/// A comma-separated list of keys.
fn list(prefs: &Prefs, key: &str) -> Vec<String> {
    text(prefs, key)
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// What a profile left blank, which for a login means the email address.
fn or_else(value: String, fallback: &str) -> String {
    if value.is_empty() { fallback.to_string() } else { value }
}