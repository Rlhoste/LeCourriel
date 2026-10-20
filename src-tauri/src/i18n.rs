//! Language for text the backend builds itself: greeting, briefing lines and dates.
//! The current language is global (set from config at startup and on save) so the
//! date helpers can be called from anywhere without passing the config around.

use chrono::{Datelike, Timelike};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
}

static LANG: AtomicU8 = AtomicU8::new(0);

/// "fr" selects French; anything else is English.
pub fn set_lang(code: &str) {
    LANG.store(if code == "fr" { 1 } else { 0 }, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    if LANG.load(Ordering::Relaxed) == 1 { Lang::Fr } else { Lang::En }
}

const FR_DAYS: [&str; 7] = ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"];
const FR_DAYS_SHORT: [&str; 7] = ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."];
const FR_MONTHS: [&str; 12] = [
    "janvier", "février", "mars", "avril", "mai", "juin",
    "juillet", "août", "septembre", "octobre", "novembre", "décembre",
];
const FR_MONTHS_SHORT: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin",
    "juil.", "août", "sept.", "oct.", "nov.", "déc.",
];

/// Short weekday: "Tue" / "mar.".
pub fn short_day(d: &impl Datelike, l: Lang) -> String {
    let i = d.weekday().num_days_from_monday() as usize;
    match l {
        Lang::En => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][i].to_string(),
        Lang::Fr => FR_DAYS_SHORT[i].to_string(),
    }
}

/// Day and short month: "Oct 20" / "20 oct.".
pub fn day_month(d: &impl Datelike, l: Lang) -> String {
    match l {
        Lang::En => {
            let m = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
            format!("{} {:02}", m[d.month0() as usize], d.day())
        }
        Lang::Fr => format!("{} {}", d.day(), FR_MONTHS_SHORT[d.month0() as usize]),
    }
}

/// Full date and time: "Tuesday 20 October 2026, 10:41" / "mardi 20 octobre 2026, 10:41".
pub fn full_datetime<T: Datelike + Timelike>(dt: &T, l: Lang) -> String {
    let wd = dt.weekday().num_days_from_monday() as usize;
    let m = dt.month0() as usize;
    match l {
        Lang::En => {
            let days = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
            let months = ["January", "February", "March", "April", "May", "June", "July",
                "August", "September", "October", "November", "December"];
            format!("{} {:02} {} {}, {:02}:{:02}", days[wd], dt.day(), months[m], dt.year(), dt.hour(), dt.minute())
        }
        Lang::Fr => format!(
            "{} {} {} {}, {:02}:{:02}",
            FR_DAYS[wd], dt.day(), FR_MONTHS[m], dt.year(), dt.hour(), dt.minute()
        ),
    }
}

pub fn greeting(hour: u32, name: &str, l: Lang) -> String {
    let name = name.trim();
    let base = match l {
        Lang::En => {
            let w = if hour < 12 { "morning" } else if hour < 17 { "afternoon" } else { "evening" };
            format!("Good {}", w)
        }
        Lang::Fr => (if hour < 18 { "Bonjour" } else { "Bonsoir" }).to_string(),
    };
    if name.is_empty() { format!("{}.", base) } else { format!("{}, {}.", base, name) }
}

/// "3 require your attention" / "3 e-mails demandent votre attention".
fn attention(n: usize, l: Lang) -> String {
    match l {
        Lang::En => format!("{} require{}", n, if n == 1 { "s you" } else { " your attention" }),
        Lang::Fr => match n {
            0 => "rien ne demande votre attention".to_string(),
            1 => "1 e-mail demande votre attention".to_string(),
            _ => format!("{} e-mails demandent votre attention", n),
        },
    }
}

pub fn briefing_live(hour: u32, minute: u32, n: usize, l: Lang) -> String {
    match l {
        Lang::En => format!("Briefed at {:02}:{:02} · {}", hour, minute, attention(n, l)),
        Lang::Fr => format!("Résumé à {:02}h{:02} · {}", hour, minute, attention(n, l)),
    }
}

pub fn briefing_cached(n: usize, l: Lang) -> String {
    match l {
        Lang::En => format!("Cached · {}", attention(n, l)),
        Lang::Fr => format!("En cache · {}", attention(n, l)),
    }
}

pub fn classifying(n: usize, l: Lang) -> String {
    match l {
        Lang::En => format!("Classifying {} new email{} with AI…", n, if n == 1 { "" } else { "s" }),
        Lang::Fr => if n == 1 {
            "Classement d'un nouvel e-mail par l'IA…".to_string()
        } else {
            format!("Classement de {} nouveaux e-mails par l'IA…", n)
        },
    }
}

pub fn up_to_date(l: Lang) -> &'static str {
    match l { Lang::En => "Up to date.", Lang::Fr => "À jour." }
}

pub fn unknown_date(l: Lang) -> &'static str {
    match l { Lang::En => "Unknown", Lang::Fr => "Inconnue" }
}

/// Language name for AI prompts.
pub fn ai_language_name(l: Lang) -> &'static str {
    match l { Lang::En => "English", Lang::Fr => "French" }
}
