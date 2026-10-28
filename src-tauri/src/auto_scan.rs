//! Auto-scan: looking through the other mail apps on this computer for accounts the user
//! can add with their server settings already filled in. Thunderbird and Betterbird keep
//! those settings in plain text in their profile, classic Outlook in the registry and
//! Evolution in one keyfile per account. Their passwords sit in other places, encrypted,
//! and those are never opened here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::auto_setup::usable_port;
use crate::dto::{FoundAccountDto, ServerDto};

/// The accounts of a profile, keyed by their pref name (always lowercase).
type Prefs = BTreeMap<String, String>;

/// The values of one registry key, by value name (lowercased): its type and its data as
/// `reg query` writes them.
type RegKey = BTreeMap<String, (String, String)>;

/// The sections of a keyfile, by section name then by key name, both lowercased.
type Sections = BTreeMap<String, BTreeMap<String, String>>;

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
    found.extend(outlook_accounts());
    found.extend(evolution_accounts());
    dedupe(found)
}

/// The accounts classic Outlook keeps in the registry, none on the other systems. Its
/// passwords sit under other keys, protected by DPAPI: never read here.
fn outlook_accounts() -> Vec<FoundAccountDto> {
    let mut out: Vec<FoundAccountDto> = Vec::new();
    #[cfg(windows)]
    for key in outlook_keys() {
        out.extend(parse_outlook_reg(&reg(&["query", &key, "/s"])));
    }
    out
}

/// The keys holding the Outlook accounts: one per Office version installed, plus the one
/// Outlook 2003-2010 uses. Nothing is known about a version that is not installed, and
/// `reg.exe` answers nothing about a key that is not there.
#[cfg(windows)]
fn outlook_keys() -> Vec<String> {
    const ROOT: &str = r"HKCU\Software\Microsoft\Office";
    let mut keys: Vec<String> = reg(&["query", ROOT])
        .lines()
        .filter_map(|line| line.trim().strip_prefix(&format!("{ROOT}\\")))
        .map(|rest| rest.split('\\').next().unwrap_or("").to_string())
        // The version keys are "16.0", "15.0"…; the others hold the apps' own settings.
        .filter(|v| v.contains('.') && v.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|v| format!(r"{ROOT}\{v}\Outlook\Profiles"))
        .collect();
    keys.push(r"HKCU\Software\Microsoft\Windows NT\CurrentVersion\Windows Messaging Subsystem\Profiles".to_string());
    keys
}

/// `reg.exe` on a key: its whole output, "" when it is missing, answers nothing at all or
/// is not there. The console window is hidden: this runs from the app, not a terminal.
#[cfg(windows)]
fn reg(args: &[&str]) -> String {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    match std::process::Command::new("reg.exe").args(args).creation_flags(CREATE_NO_WINDOW).output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
        _ => String::new(),
    }
}

/// The accounts of the Evolution sources on this computer, none on Windows.
fn evolution_accounts() -> Vec<FoundAccountDto> {
    let mut files: Vec<(String, String)> = Vec::new();
    #[cfg(not(windows))]
    for dir in evolution_dirs() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("source")) {
                    let uid = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
                    files.push((uid, read(&path)));
                }
            }
        }
    }
    files.sort();   // a folder lists its files in no order of its own
    parse_evolution(&files)
}

/// The folders Evolution writes its sources into, the Flatpak app among them.
#[cfg(not(windows))]
fn evolution_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let Some(home) = std::env::var_os("HOME") else { return out };
    let home = PathBuf::from(home);
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    out.push(config.join("evolution").join("sources"));
    out.push(home.join(".var/app/org.gnome.Evolution/config/evolution/sources"));
    out
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
            out.push((appdata.join("Betterbird"), "Betterbird"));
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

// ── Classic Outlook (registry) ────────────────────────────────────────────────

/// The accounts of a `reg query "…\Outlook\Profiles" /s` output. One key of that tree is
/// one account; the values it holds say which kind it is. The value names are matched
/// whatever their capitals, and a name Outlook never wrote simply brings nothing.
pub fn parse_outlook_reg(output: &str) -> Vec<FoundAccountDto> {
    let mut out: Vec<FoundAccountDto> = Vec::new();
    for key in reg_blocks(output) {
        let account = reg_text(&key, "account name");
        // An account with no address of its own is named by one, when it holds one.
        let email = or_else(reg_text(&key, "email"), &if account.contains('@') { account.clone() } else { String::new() });
        if email.is_empty() {
            continue;
        }
        let host = reg_text(&key, "imap server");
        let pop3 = host.is_empty();
        // Neither an IMAP nor a POP3 server: an Exchange or a Microsoft 365 mailbox, an
        // address book. This app speaks IMAP only, so those are passed over.
        let incoming = if pop3 { reg_text(&key, "pop3 server") } else { host.clone() };
        if incoming.is_empty() {
            continue;
        }
        let mut found = FoundAccountDto {
            client: "Outlook".to_string(),
            label: or_else(account, &email),
            name: reg_text(&key, "display name"),
            email: email.clone(),
            pop3,
            ..Default::default()
        };
        // A POP3 server brings nothing this app can use, and says so by being POP3.
        if !pop3 {
            let socket = if reg_number(&key, "imap use ssl").unwrap_or(0) != 0 { "SSL" } else { "plain" };
            match usable_port("imap", socket, &reg_port(&key, "imap port")) {
                Ok(port) => found.imap = Some(ServerDto { host, port, user: or_else(reg_text(&key, "imap user"), &email) }),
                Err(why) => found.imap_issue = why.to_string(),
            }
        }
        let smtp_host = reg_text(&key, "smtp server");
        if !smtp_host.is_empty() {
            // Outlook writes the encryption as two flags and says no more than "secured".
            // shortcut: `SMTP Secure Connection` set is taken for STARTTLS, which is what
            // the 587 Outlook offers with it means.
            let socket = if reg_number(&key, "smtp use ssl").unwrap_or(0) != 0 {
                "SSL"
            } else if reg_number(&key, "smtp secure connection").unwrap_or(0) != 0 {
                "STARTTLS"
            } else {
                "plain"
            };
            match usable_port("smtp", socket, &reg_port(&key, "smtp port")) {
                Ok(port) => found.smtp = Some(ServerDto { host: smtp_host, port, user: or_else(reg_text(&key, "smtp user"), &email) }),
                Err(why) => found.smtp_issue = why.to_string(),
            }
        }
        out.push(found);
    }
    out
}

/// The keys of a `reg query … /s` output, each as its values. A line holding a key path
/// opens one, an indented line adds a value to the one just opened. Anything else — the
/// blank lines, the "End of search:" line — is passed over.
fn reg_blocks(output: &str) -> Vec<RegKey> {
    let mut out: Vec<RegKey> = Vec::new();
    for line in output.lines() {
        if line.starts_with(char::is_whitespace) {
            if let Some(last) = out.last_mut() {
                if let Some((name, kind, data)) = reg_value(line) {
                    last.insert(name, (kind, data));
                }
            }
        } else if line.starts_with("HKEY_") {
            out.push(BTreeMap::new());
        }
    }
    out
}

/// A value line of `reg query`: `    <name>    <TYPE>    <data>`. A name holds single
/// spaces ("IMAP Use SSL"), never a run of them, so a run is what divides the three parts.
fn reg_value(line: &str) -> Option<(String, String, String)> {
    let (name, rest) = split_gap(line.trim_start())?;
    let (kind, data) = match split_gap(rest) {
        Some((kind, data)) => (kind, data),
        None => (rest.trim(), ""),   // a value with no data at all
    };
    let kind = kind.to_uppercase();
    if name.is_empty() || !kind.starts_with("REG_") {
        return None;
    }
    Some((name.to_lowercase(), kind, data.trim().to_string()))
}

/// The text before the first run of two spaces, and the text after it.
fn split_gap(s: &str) -> Option<(&str, &str)> {
    let i = s.find("  ")?;
    Some((&s[..i], s[i..].trim_start()))
}

/// A value holding text: a REG_BINARY one is UTF-16LE written in hex, the others are text
/// as it stands. A number is no text.
fn reg_text(key: &RegKey, name: &str) -> String {
    match key.get(name) {
        Some((kind, data)) if kind == "REG_BINARY" => utf16_hex(data),
        Some((kind, _)) if kind.starts_with("REG_DWORD") || kind == "REG_QWORD" => String::new(),
        Some((_, data)) => data.clone(),
        None => String::new(),
    }
}

/// A hex string of `reg query`: its bytes read as UTF-16LE text, up to the NUL it ends with.
fn utf16_hex(hex: &str) -> String {
    let hex: String = hex.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let mut units: Vec<u16> = Vec::new();
    let mut i = 0;
    while i + 4 <= hex.len() {
        // Two bytes make one letter, the low one written first: "6a00" is 'j'.
        match (u8::from_str_radix(&hex[i..i + 2], 16), u8::from_str_radix(&hex[i + 2..i + 4], 16)) {
            (Ok(low), Ok(high)) => units.push(u16::from(low) | (u16::from(high) << 8)),
            _ => break,
        }
        i += 4;
    }
    String::from_utf16_lossy(&units)
        .split('\u{0}')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// A REG_DWORD value: `reg query` writes it as `0x3db`.
fn reg_number(key: &RegKey, name: &str) -> Option<u32> {
    let data = key.get(name)?.1.trim().to_string();
    match data.strip_prefix("0x").or(data.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => data.parse().ok(),
    }
}

/// A port as Auto-setup wants it: "" when the key writes none.
fn reg_port(key: &RegKey, name: &str) -> String {
    reg_number(key, name).map(|p| p.to_string()).unwrap_or_default()
}

// ── Evolution (source keyfiles) ───────────────────────────────────────────────

/// The accounts of Evolution's source files. One account is three files: the account
/// itself, the identity it names and the transport it sends through. `files` is one entry
/// per file: its UID (its name without ".source") and its content.
pub fn parse_evolution(files: &[(String, String)]) -> Vec<FoundAccountDto> {
    let sources: Vec<(String, Sections)> =
        files.iter().map(|(uid, content)| (uid.clone(), keyfile(content))).collect();
    let mut out: Vec<FoundAccountDto> = Vec::new();
    for (uid, source) in &sources {
        let backend = get(source, "mail account", "backendname").to_lowercase();
        // "maildir" and "none" are Local Folders, "ews" an Exchange Web Services account.
        let pop3 = backend == "pop";
        if !pop3 && backend != "imapx" {
            continue;
        }
        let identity = linked(&sources, &get(source, "mail account", "identityuid"), uid, "mail identity");
        let email = identity.map(|s| get(s, "mail identity", "address")).unwrap_or_default();
        if email.is_empty() {
            continue;
        }
        let mut found = FoundAccountDto {
            client: "Evolution".to_string(),
            label: or_else(get(source, "data source", "displayname"), &email),
            name: identity.map(|s| get(s, "mail identity", "name")).unwrap_or_default(),
            email: email.clone(),
            pop3,
            oauth: oauth(&get(source, "authentication", "method")),
            ..Default::default()
        };
        // A POP3 account brings nothing this app can use, and says so by being POP3.
        if !pop3 {
            let socket = security(&get(source, "security", "method"));
            let host = get(source, "authentication", "host");
            let port = get(source, "authentication", "port");
            let user = or_else(get(source, "authentication", "user"), &email);
            match usable_port("imap", &socket, &port) {
                Ok(port) => found.imap = Some(ServerDto { host, port, user }),
                Err(why) => found.imap_issue = why.to_string(),
            }
        }
        // The transport: the one the identity names, in one group or the other.
        let transport_uid = identity
            .map(|s| or_else(get(s, "mail submission", "transportuid"), &get(s, "mail identity", "transportuid")))
            .unwrap_or_default();
        if let Some(transport) = linked(&sources, &transport_uid, uid, "mail transport") {
            if get(transport, "mail transport", "backendname").eq_ignore_ascii_case("smtp") {
                let socket = security(&get(transport, "security", "method"));
                let host = get(transport, "authentication", "host");
                let port = get(transport, "authentication", "port");
                let user = or_else(get(transport, "authentication", "user"), &email);
                match usable_port("smtp", &socket, &port) {
                    Ok(port) => found.smtp = Some(ServerDto { host, port, user }),
                    Err(why) => found.smtp_issue = why.to_string(),
                }
                found.oauth |= oauth(&get(transport, "authentication", "method"));
            }
        }
        out.push(found);
    }
    out
}

/// The sections of a keyfile: `[Section]` opens one, `Key=value` fills one. shortcut: the
/// `\s`-style escapes GKeyFile knows are left as written, and the `${USER}` a preconfigured
/// template holds is left alone too — a source Evolution wrote itself holds the real text.
fn keyfile(content: &str) -> Sections {
    let mut out: Sections = BTreeMap::new();
    let mut section = String::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name.to_lowercase();
            out.entry(section.clone()).or_default();
            continue;
        }
        if section.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            out.get_mut(&section).unwrap()
                .insert(key.trim().to_lowercase(), value.trim().to_string());
        }
    }
    out
}

/// A value of a source, "" when the section or the key is not there.
fn get(source: &Sections, section: &str, key: &str) -> String {
    source.get(section).and_then(|s| s.get(key)).cloned().unwrap_or_default()
}

/// The source an account points at: the one whose UID is `uid`, else one of its own
/// children holding `section` — a file of an older version sometimes writes no link.
fn linked<'a>(sources: &'a [(String, Sections)], uid: &str, parent: &str, section: &str) -> Option<&'a Sections> {
    if !uid.is_empty() {
        if let Some((_, source)) = sources.iter().find(|(id, s)| id.eq_ignore_ascii_case(uid) && s.contains_key(section)) {
            return Some(source);
        }
    }
    sources.iter()
        .find(|(id, s)| !id.eq_ignore_ascii_case(parent)
            && get(s, "data source", "parent").eq_ignore_ascii_case(parent)
            && s.contains_key(section))
        .map(|(_, s)| s)
}

/// Evolution's `[Security] Method` as Auto-setup names it. The value is free text; the two
/// spellings the mail side writes are "ssl-on-alternate-port" and "starttls-on-standard-
/// port", so anything holding "starttls" is STARTTLS and anything holding "ssl" is SSL.
fn security(method: &str) -> String {
    let method = method.to_lowercase();
    if method.contains("starttls") { "STARTTLS" } else if method.contains("ssl") { "SSL" } else { "plain" }
        .to_string()
}

/// A login this app cannot fill with a password: Evolution keeps the token in its keyring.
fn oauth(method: &str) -> bool {
    let method = method.to_lowercase();
    method.contains("oauth") || method.contains("google")
}
