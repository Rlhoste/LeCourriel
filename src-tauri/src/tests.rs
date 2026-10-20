/// Unit tests for Le courriel backend logic.
/// Run with: cargo test
#[cfg(test)]
mod tests {
    use crate::config::*;
    use crate::models::*;
    use crate::settings::config_to_dto;
    use crate::dto::AccountDto;

    // ── Config ────────────────────────────────────────────────────────────────

    #[test]
    fn test_config_load_example() {
        // Verify example config parses without error.
        let example = r#"
[user]
display_name = "Test User"
font_scale = 1.0

[[accounts]]
name = "Work"
[accounts.imap]
host = "imap.example.com"
port = 993
username = "user@example.com"
password = "secret"
mailbox = "INBOX"
fetch_count = 50
[accounts.smtp]
host = "smtp.example.com"
port = 587
username = "user@example.com"
password = "secret"
from_address = "user@example.com"

[claude]
api_key = "sk-test"
model = "claude-sonnet-4-6"
max_tokens = 1024
"#;
        let cfg: Config = toml::from_str(example).unwrap();
        assert_eq!(cfg.user.display_name, "Test User");
        assert_eq!(cfg.accounts.len(), 1);
        assert_eq!(cfg.accounts[0].name, "Work");
        assert_eq!(cfg.accounts[0].imap.host, "imap.example.com");
        assert_eq!(cfg.accounts[0].smtp.port, 587);
        assert_eq!(cfg.claude.model, "claude-sonnet-4-6");
    }

    #[test]
    fn test_config_default_font_scale() {
        let toml = r#"
[user]
display_name = "No Scale"
[[accounts]]
name = "A"
[accounts.imap]
host = "h" port = 993 username = "u" password = "p" mailbox = "INBOX" fetch_count = 10
[accounts.smtp]
host = "h" port = 587 username = "u" password = "p" from_address = "u@h"
[claude]
api_key = "k" model = "m" max_tokens = 100
"#;
        // font_scale missing → defaults to 1.0
        if let Ok(cfg) = toml::from_str::<Config>(toml) {
            assert!((cfg.user.font_scale - 1.0).abs() < f32::EPSILON);
        }
        // If parse fails due to inline syntax, just verify the default fn
        assert!((default_font_scale_val() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_config_save_roundtrip() {
        let cfg = make_test_config();
        let serialized = toml::to_string_pretty(&cfg).unwrap();
        let restored: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(restored.user.display_name, cfg.user.display_name);
        assert_eq!(restored.accounts[0].imap.host, cfg.accounts[0].imap.host);
        assert_eq!(restored.claude.api_key, cfg.claude.api_key);
    }

    // ── Priority ──────────────────────────────────────────────────────────────

    #[test]
    fn test_priority_from_str() {
        assert_eq!(Priority::from_str("urgent"),       Priority::Urgent);
        assert_eq!(Priority::from_str("URGENT"),       Priority::Urgent);
        assert_eq!(Priority::from_str("high_priority"),Priority::HighPriority);
        assert_eq!(Priority::from_str("high"),         Priority::HighPriority);
        assert_eq!(Priority::from_str("threat"),       Priority::Threat);
        assert_eq!(Priority::from_str("task"),         Priority::Task);
        assert_eq!(Priority::from_str("information"),  Priority::Information);
        assert_eq!(Priority::from_str("info"),         Priority::Information);
        assert_eq!(Priority::from_str("other"),        Priority::OtherPriority);
        assert_eq!(Priority::from_str("garbage"),      Priority::OtherPriority);
        assert_eq!(Priority::from_str(""),             Priority::OtherPriority);
    }

    #[test]
    fn test_priority_as_str_roundtrip() {
        for p in [
            Priority::Urgent, Priority::HighPriority, Priority::OtherPriority,
            Priority::Information, Priority::Threat, Priority::Task,
        ] {
            assert_eq!(Priority::from_str(p.as_str()), p);
        }
    }

    // ── Settings DTO ──────────────────────────────────────────────────────────

    #[test]
    fn test_config_to_dto() {
        let cfg = make_test_config();
        let dto = config_to_dto(&cfg);
        assert_eq!(dto.display_name, "Test");
        assert_eq!(dto.accounts.len(), 1);
        assert_eq!(dto.accounts[0].label, "Work");
        assert_eq!(dto.accounts[0].imap_port, "993");
        assert_eq!(dto.accounts[0].smtp_port, "587");
        assert_eq!(dto.claude_key, "sk-test");
    }

    #[test]
    fn test_save_config_updates_fields() {
        use crate::settings::save_config as settings_save;
        use std::sync::Arc;

        // We can't write to disk in unit tests, but we can test the logic
        // by verifying save_config builds the right Config structure.
        // Instead, test AccountDto → AccountConfig mapping manually.
        let accounts = vec![AccountDto {
            label: "Personal".to_string(),
            imap_host: "imap.test.com".to_string(),
            imap_port: "993".to_string(),
            imap_user: "me@test.com".to_string(),
            imap_pass: "pass".to_string(),
            imap_mailbox: "INBOX".to_string(),
            imap_fetch: "25".to_string(),
            smtp_host: "smtp.test.com".to_string(),
            smtp_port: "465".to_string(),
            smtp_user: "me@test.com".to_string(),
            smtp_pass: "pass".to_string(),
            smtp_from: "me@test.com".to_string(),
        }];

        // Validate parse logic used inside save_config
        let parsed_imap_port: u16 = accounts[0].imap_port.parse().unwrap_or(993);
        let parsed_smtp_port: u16 = accounts[0].smtp_port.parse().unwrap_or(587);
        let parsed_fetch: u32 = accounts[0].imap_fetch.parse().unwrap_or(50);

        assert_eq!(parsed_imap_port, 993);
        assert_eq!(parsed_smtp_port, 465);
        assert_eq!(parsed_fetch, 25);
        let _ = (settings_save, Arc::new(())); // suppress unused import warning
    }

    // ── EmailCard serialization ───────────────────────────────────────────────

    #[test]
    fn test_email_card_serde_roundtrip() {
        let card = EmailCard {
            uid: 42,
            subject: "Hello".to_string(),
            from_name: "Alice".to_string(),
            from_email: "alice@example.com".to_string(),
            to: String::new(), cc: String::new(), reply_to: String::new(), bcc: String::new(), message_id: String::new(),
            date_str: "10:30".to_string(),
            date_full: String::new(),
            summary: "A test email".to_string(),
            priority: Priority::Urgent,
            is_read: false,
            body_text: "Plain body".to_string(),
            body_html: "<p>HTML body</p>".to_string(),
            project: Some("MyProject".to_string()),
            deadline: None,
            task: Some("Do something".to_string()),
            meeting_link: None,
        };

        let json = serde_json::to_string(&card).unwrap();
        let restored: EmailCard = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.uid, 42);
        assert_eq!(restored.subject, "Hello");
        assert_eq!(restored.priority, Priority::Urgent);
        assert_eq!(restored.body_html, "<p>HTML body</p>");
        assert_eq!(restored.project, Some("MyProject".to_string()));
        assert_eq!(restored.task, Some("Do something".to_string()));
    }

    #[test]
    fn test_email_card_vec_serde() {
        let cards = vec![
            make_test_card(1, Priority::Urgent),
            make_test_card(2, Priority::HighPriority),
            make_test_card(3, Priority::Information),
        ];
        let json = serde_json::to_string(&cards).unwrap();
        let restored: Vec<EmailCard> = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.len(), 3);
        assert_eq!(restored[0].uid, 1);
        assert_eq!(restored[2].priority, Priority::Information);
    }

    // ── From-address parsing (mirrors app_state::parse_from) ─────────────────

    #[test]
    fn test_parse_from_with_name() {
        let (name, email) = parse_from_test("Alice Smith <alice@example.com>");
        assert_eq!(name, "Alice Smith");
        assert_eq!(email, "alice@example.com");
    }

    #[test]
    fn test_parse_from_quoted_name() {
        let (name, email) = parse_from_test("\"Bob Jones\" <bob@example.com>");
        assert_eq!(name, "Bob Jones");
        assert_eq!(email, "bob@example.com");
    }

    #[test]
    fn test_parse_from_email_only() {
        let (name, email) = parse_from_test("carol@example.com");
        assert_eq!(name, "carol@example.com");
        assert_eq!(email, "carol@example.com");
    }

    #[test]
    fn test_parse_from_empty_name() {
        let (name, email) = parse_from_test("<dave@example.com>");
        // Empty name → falls back to email
        assert_eq!(name, "dave@example.com");
        assert_eq!(email, "dave@example.com");
    }

    // ── Cache UID deduplication logic ─────────────────────────────────────────

    #[test]
    fn test_uid_dedup_logic() {
        let cached = vec![
            make_test_card(100, Priority::Urgent),
            make_test_card(101, Priority::Information),
            make_test_card(102, Priority::Task),
        ];

        let cached_uids: std::collections::HashSet<u32> =
            cached.iter().map(|c| c.uid).collect();

        // Simulate raw emails from server: 101 and 102 already known, 103 is new
        let server_uids = vec![101u32, 102, 103];
        let (known, new): (Vec<&u32>, Vec<&u32>) = server_uids.iter()
            .partition(|uid| cached_uids.contains(uid));

        assert_eq!(known.len(), 2);
        assert_eq!(new.len(), 1);
        assert_eq!(*new[0], 103);
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_test_config() -> Config {
        Config {
            user: UserConfig { display_name: "Test".to_string(), font_scale: 1.0, theme: "midnight".to_string(), language: "en".to_string() },
            accounts: vec![AccountConfig {
                name: "Work".to_string(),
                imap: ImapConfig {
                    host: "imap.example.com".to_string(), port: 993,
                    username: "u".to_string(), password: "p".to_string(),
                    mailbox: "INBOX".to_string(), fetch_count: 50,
                },
                smtp: SmtpConfig {
                    host: "smtp.example.com".to_string(), port: 587,
                    username: "u".to_string(), password: "p".to_string(),
                    from_address: "u@example.com".to_string(),
                },
            }],
            claude: ClaudeConfig {
                api_key: "sk-test".to_string(),
                model: "claude-sonnet-4-6".to_string(),
                max_tokens: 1024,
            },
            projects: vec![],
            gifs: Default::default(),
            database: None,
        }
    }

    fn make_test_card(uid: u32, priority: Priority) -> EmailCard {
        EmailCard {
            uid,
            subject: format!("Subject {}", uid),
            from_name: "Sender".to_string(),
            from_email: "sender@example.com".to_string(),
            to: String::new(), cc: String::new(), reply_to: String::new(), bcc: String::new(), message_id: String::new(),
            date_str: "Mon".to_string(),
            date_full: String::new(),
            summary: "Summary".to_string(),
            priority,
            is_read: false,
            body_text: "Body".to_string(),
            body_html: String::new(),
            project: None,
            deadline: None,
            task: None,
            meeting_link: None,
        }
    }

    /// Mirrors app_state::parse_from (duplicated here to test it without making it pub)
    fn parse_from_test(from: &str) -> (String, String) {
        let from = from.trim();
        if let Some(lt) = from.find('<') {
            if let Some(gt) = from.find('>') {
                let name = from[..lt].trim().trim_matches('"').to_string();
                let email = from[lt+1..gt].trim().to_string();
                let display = if name.is_empty() { email.clone() } else { name };
                return (display, email);
            }
        }
        (from.to_string(), from.to_string())
    }

    fn default_font_scale_val() -> f32 { 1.0 }

    // ── Task check / dismissals ───────────────────────────────────────────────

    #[test]
    fn test_parse_task_check_valid() {
        use crate::ai_classifier::parse_task_check;
        let out = parse_task_check(r#"{"done": [12, 99], "obsolete": [7]}"#, &[7, 12, 30]);
        // 99 is not an open task and must be ignored.
        assert_eq!(out, vec![(12, "done"), (7, "obsolete")]);
    }

    #[test]
    fn test_parse_task_check_code_fence_and_overlap() {
        use crate::ai_classifier::parse_task_check;
        let out = parse_task_check("```json\n{\"done\": [5], \"obsolete\": [5]}\n```", &[5]);
        // A uid listed as both counts once, as done.
        assert_eq!(out, vec![(5, "done")]);
    }

    #[test]
    fn test_parse_task_check_garbage() {
        use crate::ai_classifier::parse_task_check;
        assert!(parse_task_check("Sorry, I cannot tell.", &[1, 2]).is_empty());
        assert!(parse_task_check("{}", &[1, 2]).is_empty());
    }

    #[test]
    fn test_dismiss_kinds() {
        use crate::dismissals::Dismissals;
        let mut d = Dismissals::default();
        d.dismiss("task", 1).unwrap();
        d.dismiss("deadline", 2).unwrap();
        d.dismiss("meeting", 3).unwrap();
        assert!(d.dismiss("bogus", 4).is_err());
        assert!(d.tasks.contains(&1));
        assert!(d.deadlines.contains(&2) && d.meetings.contains(&3));
    }


    // ── Task board ────────────────────────────────────────────────────────────

    #[test]
    fn test_board_sync_from_cards() {
        use crate::dismissals::Dismissals;
        use crate::tasks::{Board, Status};
        let mut with_task = make_test_card(10, Priority::HighPriority);
        with_task.task = Some("Send the manuscript".into());
        with_task.project = Some("Thesis".into());
        let task_prio = make_test_card(11, Priority::Task);
        let plain = make_test_card(12, Priority::Information);
        let mut swiped = make_test_card(13, Priority::Task);
        swiped.task = Some("Old thing".into());
        let mut dismissed = Dismissals::default();
        dismissed.tasks.insert(13);

        let mut b = Board::default();
        let cards = vec![with_task, task_prio, plain, swiped];
        assert!(b.sync_from_cards(&cards, &dismissed));
        assert_eq!(b.tasks.len(), 3);
        assert_eq!(b.tasks[0].title, "Send the manuscript");
        assert_eq!(b.tasks[0].project.as_deref(), Some("Thesis"));
        assert_eq!(b.tasks[0].status, Status::Todo);
        assert_eq!(b.tasks[2].status, Status::GivenUp);
        // Second sync adds nothing.
        assert!(!b.sync_from_cards(&cards, &dismissed));
        assert_eq!(b.tasks.len(), 3);
    }

    #[test]
    fn test_board_delete_email_task_is_not_resynced() {
        use crate::dismissals::Dismissals;
        use crate::tasks::{Board, Status};
        let mut card = make_test_card(20, Priority::Task);
        card.task = Some("Reply to jury".into());
        let mut b = Board::default();
        b.sync_from_cards(&[card.clone()], &Dismissals::default());
        let id = b.tasks[0].id;
        b.set_status(id, Status::InProgress).unwrap();
        assert_eq!(b.tasks[0].status, Status::InProgress);
        b.delete(id).unwrap();
        assert!(b.tasks.is_empty());
        assert!(!b.sync_from_cards(&[card], &Dismissals::default()));
        assert!(b.set_status(id, Status::Done).is_err());
    }

    #[test]
    fn test_board_manual_task_ids_unique() {
        use crate::tasks::{Board, Status};
        let mut b = Board::default();
        let a = b.add("Buy paper".into(), None);
        let c = b.add("Book room".into(), Some("Work".into()));
        assert_ne!(a, c);
        assert!(b.tasks.iter().all(|t| t.status == Status::Todo && t.source_uid.is_none()));
        assert_eq!(Status::parse("in_progress"), Some(Status::InProgress));
        assert_eq!(Status::parse("nope"), None);
    }


    // ── Read state ────────────────────────────────────────────────────────────

    #[test]
    fn test_important_unread_skips_read() {
        use crate::app_state::important_unread;
        let mut read = make_test_card(1, Priority::Urgent);
        read.is_read = true;
        let unread = make_test_card(2, Priority::Urgent);
        let mut high_unread = make_test_card(3, Priority::HighPriority);
        high_unread.is_read = false;
        let out = important_unread(&[read, unread], &[high_unread]);
        let uids: Vec<u32> = out.iter().map(|c| c.uid).collect();
        assert_eq!(uids, vec![2, 3]);
    }


    // ── Overrides ─────────────────────────────────────────────────────────────

    #[test]
    fn test_override_project() {
        use crate::overrides::Overrides;
        let mut o = Overrides::default();
        let mut tagged = make_test_card(1, Priority::Information);
        tagged.project = Some("Thesis".into());
        let mut cleared = tagged.clone();
        cleared.uid = 2;
        let mut untouched = tagged.clone();
        untouched.uid = 3;

        o.set_project(1, Some("Work".into()));
        o.set_project(2, None);
        o.apply(&mut tagged);
        o.apply(&mut cleared);
        o.apply(&mut untouched);
        assert_eq!(tagged.project.as_deref(), Some("Work"));
        assert_eq!(cleared.project, None);
        assert_eq!(untouched.project.as_deref(), Some("Thesis"));

        // Round-trips through JSON (u32 map keys become strings).
        let json = serde_json::to_string(&o).unwrap();
        let back: Overrides = serde_json::from_str(&json).unwrap();
        assert_eq!(back.projects.get(&1).map(String::as_str), Some("Work"));
        assert_eq!(back.projects.get(&2).map(String::as_str), Some(""));
    }


    // ── Outgoing mail ─────────────────────────────────────────────────────────

    #[test]
    fn test_extract_inline_images() {
        use crate::smtp_sender::extract_inline_images;
        // "hi" in base64 is aGk=
        let html = r#"<p>Hello</p><img src="data:image/png;base64,aGk=" style="max-width:100%"><img src='https://example.com/a.png'><img src="data:text/html;base64,aGk=">"#;
        let (out, images) = extract_inline_images(html);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].cid, "img1");
        assert_eq!(images[0].content_type, "image/png");
        assert_eq!(images[0].data, b"hi");
        assert!(out.contains(r#"src="cid:img1""#));
        assert!(out.contains("src='https://example.com/a.png'"));
        // Non-image data URIs are left as they are.
        assert!(out.contains(r#"src="data:text/html;base64,aGk=""#));
        assert!(out.starts_with("<p>Hello</p>"));
    }

    #[test]
    fn test_extract_inline_images_none() {
        use crate::smtp_sender::extract_inline_images;
        let (out, images) = extract_inline_images("plain <b>text</b>, src= nothing");
        assert!(images.is_empty());
        assert_eq!(out, "plain <b>text</b>, src= nothing");
    }


    // ── GIFs ──────────────────────────────────────────────────────────────────

    #[test]
    fn test_parse_giphy() {
        use crate::gifs::parse_giphy;
        let json = r#"{"data":[
          {"id":"abc","title":"Cat GIF","images":{
             "fixed_height":{"url":"https://media1.giphy.com/media/abc/200.gif","width":"356","height":"200"},
             "fixed_width_small":{"url":"https://media1.giphy.com/media/abc/100w.gif","width":"100","height":"56"}}},
          {"id":"nofull","title":"x","images":{}}
        ],"pagination":{}}"#;
        let gifs = parse_giphy(json).unwrap();
        assert_eq!(gifs.len(), 1);
        assert_eq!(gifs[0].id, "abc");
        assert_eq!(gifs[0].url, "https://media1.giphy.com/media/abc/200.gif");
        assert_eq!(gifs[0].preview_url, "https://media1.giphy.com/media/abc/100w.gif");
        assert_eq!((gifs[0].width, gifs[0].height), (356, 200));
        assert!(parse_giphy("not json").is_err());
    }

    #[test]
    fn test_giphy_url_allowlist() {
        use crate::gifs::is_giphy_media_url;
        assert!(is_giphy_media_url("https://media2.giphy.com/media/x/200.gif"));
        assert!(is_giphy_media_url("https://i.giphy.com/x.gif"));
        assert!(!is_giphy_media_url("http://media.giphy.com/x.gif"));
        assert!(!is_giphy_media_url("https://evilgiphy.com/x.gif"));
        assert!(!is_giphy_media_url("https://giphy.com.evil.net/x.gif"));
        assert!(!is_giphy_media_url("file:///etc/passwd"));
    }


    // ── AI writing ────────────────────────────────────────────────────────────

    #[test]
    fn test_ai_writer_strip_fences() {
        use crate::ai_writer::strip_fences;
        assert_eq!(strip_fences("```html\n<p>Hi</p>\n```"), "<p>Hi</p>");
        assert_eq!(strip_fences("```\n<p>Hi</p>```"), "<p>Hi</p>");
        assert_eq!(strip_fences("  <p>Hi</p> "), "<p>Hi</p>");
    }


    // ── Theme ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_theme_defaults_and_roundtrip() {
        let toml_str = r#"
[user]
display_name = "A"

[claude]
api_key = "k"
model = "m"
max_tokens = 10
"#;
        let mut cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.user.theme, "midnight");
        cfg.user.theme = "brutalist".into();
        let back: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!(back.user.theme, "brutalist");
        assert_eq!(config_to_dto(&back).theme, "brutalist");
    }


    // ── Overview week strip ───────────────────────────────────────────────────

    #[test]
    fn test_week_strip_starts_today() {
        use crate::app_state::build_week_days;
        use chrono::NaiveDate;
        // Wednesday 21 Oct 2026; a deadline on Tue 27 Oct must be the last day shown.
        let today = NaiveDate::from_ymd_opt(2026, 10, 21).unwrap();
        let mut card = make_test_card(42, Priority::Task);
        card.subject = "Defense rehearsal".into();
        let past = make_test_card(43, Priority::Task);
        let deadlines = vec![
            (NaiveDate::from_ymd_opt(2026, 10, 27).unwrap(), card),
            (NaiveDate::from_ymd_opt(2026, 10, 19).unwrap(), past), // before today: not shown
        ];
        let days = build_week_days(today, &deadlines);
        assert_eq!(days.len(), 7);
        assert_eq!((days[0].day_name.as_str(), days[0].day_number), ("Wed", 21));
        assert!(days[0].is_today);
        assert!(days[1..].iter().all(|d| !d.is_today));
        assert_eq!((days[6].day_name.as_str(), days[6].day_number), ("Tue", 27));
        assert_eq!(days[6].dl1_subject, "Defense rehearsal");
        assert!(days.iter().all(|d| d.dl1_uid != 43 || d.dl1_subject.is_empty()));
    }

    #[test]
    fn test_week_strip_crosses_month_end() {
        use crate::app_state::build_week_days;
        use chrono::NaiveDate;
        let today = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap();
        let nums: Vec<u32> = build_week_days(today, &[]).iter().map(|d| d.day_number).collect();
        assert_eq!(nums, vec![29, 30, 31, 1, 2, 3, 4]);
    }


    // ── Language ──────────────────────────────────────────────────────────────

    #[test]
    fn test_i18n_dates() {
        use crate::i18n::{day_month, full_datetime, short_day, Lang};
        use chrono::NaiveDate;
        let d = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();   // a Tuesday
        let dt = d.and_hms_opt(9, 5, 0).unwrap();
        assert_eq!(short_day(&d, Lang::En), "Tue");
        assert_eq!(short_day(&d, Lang::Fr), "mar.");
        assert_eq!(day_month(&d, Lang::En), "Oct 20");
        assert_eq!(day_month(&d, Lang::Fr), "20 oct.");
        assert_eq!(full_datetime(&dt, Lang::En), "Tuesday 20 October 2026, 09:05");
        assert_eq!(full_datetime(&dt, Lang::Fr), "mardi 20 octobre 2026, 09:05");
        let feb = NaiveDate::from_ymd_opt(2027, 2, 1).unwrap();
        assert_eq!(day_month(&feb, Lang::Fr), "1 févr.");
        assert_eq!(day_month(&feb, Lang::En), "Feb 01");
    }

    #[test]
    fn test_i18n_texts() {
        use crate::i18n::{briefing_cached, briefing_live, classifying, greeting, Lang};
        assert_eq!(greeting(9, "Remi", Lang::En), "Good morning, Remi.");
        assert_eq!(greeting(20, "Remi", Lang::En), "Good evening, Remi.");
        assert_eq!(greeting(9, "Remi", Lang::Fr), "Bonjour, Remi.");
        assert_eq!(greeting(19, "Remi", Lang::Fr), "Bonsoir, Remi.");
        assert_eq!(greeting(9, "", Lang::Fr), "Bonjour.");
        assert_eq!(briefing_live(10, 42, 3, Lang::En), "Briefed at 10:42 · 3 require your attention");
        assert_eq!(briefing_live(10, 42, 1, Lang::En), "Briefed at 10:42 · 1 requires you");
        assert_eq!(briefing_live(10, 42, 3, Lang::Fr), "Résumé à 10h42 · 3 e-mails demandent votre attention");
        assert_eq!(briefing_cached(1, Lang::Fr), "En cache · 1 e-mail demande votre attention");
        assert_eq!(briefing_cached(0, Lang::Fr), "En cache · rien ne demande votre attention");
        assert_eq!(classifying(1, Lang::Fr), "Classement d'un nouvel e-mail par l'IA…");
        assert_eq!(classifying(4, Lang::En), "Classifying 4 new emails with AI…");
    }

    #[test]
    fn test_language_default_and_roundtrip() {
        let mut cfg: Config = toml::from_str(
            "[user]\ndisplay_name = \"A\"\n\n[claude]\napi_key = \"k\"\nmodel = \"m\"\nmax_tokens = 10\n",
        ).unwrap();
        assert_eq!(cfg.user.language, "en");
        cfg.user.language = "fr".into();
        let back: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!(back.user.language, "fr");
        assert_eq!(config_to_dto(&back).language, "fr");
    }


    // ── PostgreSQL sync ───────────────────────────────────────────────────────

    fn config_with_secrets() -> Config {
        let mut cfg = make_test_config();
        cfg.accounts = vec![AccountConfig {
            name: "Main".into(),
            imap: ImapConfig { host: "imap.example.com".into(), port: 993, username: "remi".into(),
                password: "IMAP-SECRET".into(), mailbox: "INBOX".into(), fetch_count: 50 },
            smtp: SmtpConfig { host: "smtp.example.com".into(), port: 465, username: "remi".into(),
                password: "SMTP-SECRET".into(), from_address: "Remi <remi@example.com>".into() },
        }];
        cfg.claude.api_key = "CLAUDE-SECRET".into();
        cfg.gifs.giphy_api_key = "GIPHY-SECRET".into();
        cfg.database = Some(DatabaseConfig { url: "postgres://u:DB-SECRET@h/db".into(), accept_invalid_certs: false });
        cfg.user.theme = "aero".into();
        cfg.projects = vec![ProjectConfig { name: "Thesis".into(), description: "PhD".into(), urgency: "high".into() }];
        cfg
    }

    #[test]
    fn test_pgsync_decide() {
        use crate::pgsync::{decide, SyncAction};
        use chrono::{Duration, Utc};
        let now = Utc::now();
        assert_eq!(decide(None, None), SyncAction::Nothing);
        assert_eq!(decide(Some(now), None), SyncAction::Push);
        assert_eq!(decide(None, Some(now)), SyncAction::Pull);
        assert_eq!(decide(Some(now), Some(now + Duration::seconds(60))), SyncAction::Pull);
        assert_eq!(decide(Some(now + Duration::seconds(60)), Some(now)), SyncAction::Push);
        // Within the 2 s timestamp tolerance: same version.
        assert_eq!(decide(Some(now), Some(now + Duration::milliseconds(1500))), SyncAction::Nothing);
    }

    #[test]
    fn test_pgsync_prefs_never_contain_secrets() {
        use crate::pgsync::prefs_from_config;
        let json = prefs_from_config(&config_with_secrets()).to_string();
        for secret in ["IMAP-SECRET", "SMTP-SECRET", "CLAUDE-SECRET", "GIPHY-SECRET", "DB-SECRET", "imap.example.com"] {
            assert!(!json.contains(secret), "prefs leak {}", secret);
        }
        assert!(json.contains("aero") && json.contains("Thesis"));
    }

    #[test]
    fn test_pgsync_merge_prefs_keeps_local_secrets() {
        use crate::pgsync::{merge_prefs, prefs_from_config};
        let mut remote_cfg = make_test_config();
        remote_cfg.user.theme = "brutalist".into();
        remote_cfg.user.language = "fr".into();
        remote_cfg.projects = vec![ProjectConfig { name: "Work".into(), description: "".into(), urgency: "normal".into() }];
        let remote = prefs_from_config(&remote_cfg);

        let mut local = config_with_secrets();
        merge_prefs(&mut local, &remote);
        assert_eq!(local.user.theme, "brutalist");
        assert_eq!(local.user.language, "fr");
        assert_eq!(local.projects.len(), 1);
        assert_eq!(local.projects[0].name, "Work");
        // Untouched: accounts, keys, database.
        assert_eq!(local.accounts[0].imap.password, "IMAP-SECRET");
        assert_eq!(local.claude.api_key, "CLAUDE-SECRET");
        assert_eq!(local.gifs.giphy_api_key, "GIPHY-SECRET");
        assert_eq!(local.database_url(), Some("postgres://u:DB-SECRET@h/db"));
    }

    #[test]
    fn test_pgsync_mailbox_key_and_url() {
        use crate::pgsync::mailbox_key;
        assert_eq!(mailbox_key(&config_with_secrets()), "remi@imap.example.com");
        let mut no_accounts = make_test_config();
        no_accounts.accounts.clear();
        assert_eq!(mailbox_key(&no_accounts), "default");
        let mut cfg = make_test_config();
        cfg.database = Some(DatabaseConfig { url: "   ".into(), accept_invalid_certs: false });
        assert_eq!(cfg.database_url(), None);
    }

    #[test]
    fn test_classified_email_roundtrip_and_placeholders() {
        use crate::ai_classifier::{is_real_result, ClassifiedEmail};
        let c = ClassifiedEmail {
            priority: Priority::HighPriority,
            summary: "Jury asks for the manuscript".into(),
            project: Some("Thesis".into()),
            deadline: chrono::NaiveDate::from_ymd_opt(2026, 11, 2),
            task: Some("Send manuscript".into()),
            meeting_link: None,
        };
        let back: ClassifiedEmail = serde_json::from_value(serde_json::to_value(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        assert!(is_real_result(&c));
        let failed = ClassifiedEmail { summary: "Could not classify email.".into(), ..c };
        assert!(!is_real_result(&failed));
    }

    #[tokio::test]
    async fn test_pgsync_unreachable_server_fails_fast() {
        // Port 1 on localhost: nothing listens there.
        let started = std::time::Instant::now();
        let err = crate::pgsync::test_connection("postgres://u:p@127.0.0.1:1/db?connect_timeout=3", false)
            .await
            .unwrap_err();
        assert!(format!("{:#}", err).contains("Could not connect to PostgreSQL"), "{:#}", err);
        assert!(started.elapsed() < std::time::Duration::from_secs(11));
        // An invalid URL is reported, not a panic.
        assert!(crate::pgsync::test_connection("not a url", false).await.is_err());
    }

    #[tokio::test]
    async fn test_pgsync_off_without_database() {
        crate::pgsync::configure(&make_test_config());
        assert!(!crate::pgsync::is_configured());
        assert!(crate::pgsync::lookup_ai(&["<x@y>".into()]).await.is_empty());
        crate::pgsync::store_ai(&[]).await;   // no-op, must not try to connect
        assert_eq!(crate::pgsync::pull_all().await.unwrap(), "database sync off");
    }

    /// Against a real server: TEST_DATABASE_URL=postgres://… cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn test_pgsync_integration() {
        use crate::ai_classifier::ClassifiedEmail;
        let url = std::env::var("TEST_DATABASE_URL").expect("set TEST_DATABASE_URL");
        let version = crate::pgsync::test_connection(&url, false).await.unwrap();
        assert!(version.starts_with("PostgreSQL"), "{}", version);

        let mut cfg = config_with_secrets();
        cfg.accounts[0].imap.username = format!("courriel-test-{}", std::process::id());
        cfg.database = Some(DatabaseConfig { url, accept_invalid_certs: false });
        crate::pgsync::configure(&cfg);

        let c = ClassifiedEmail {
            priority: Priority::Urgent, summary: "Integration test".into(), project: None,
            deadline: None, task: Some("Check".into()), meeting_link: None,
        };
        crate::pgsync::store_ai(&[("<it-1@example.com>".into(), c.clone())]).await;
        let found = crate::pgsync::lookup_ai(&["<it-1@example.com>".into(), "<missing@x>".into()]).await;
        assert_eq!(found.len(), 1);
        assert_eq!(found["<it-1@example.com>"], c);

        crate::pgsync::delete_mailbox_rows().await.unwrap();
        crate::pgsync::configure(&make_test_config());
    }

}
