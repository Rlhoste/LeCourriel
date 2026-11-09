//! Local extraction of short-lived login and email-verification codes.

use crate::dto::VerificationDto;
use crate::models::EmailCard;

pub const LIFETIME_SECONDS: i64 = 10 * 60;

const CONTEXTS: &[&str] = &[
    "verification code",
    "security code",
    "login code",
    "sign-in code",
    "sign in code",
    "authentication code",
    "one-time password",
    "one time password",
    "otp code",
    "code de vérification",
    "code de verification",
    "code de connexion",
    "code d’authentification",
    "code d'authentification",
    "mot de passe à usage unique",
    "código de verificación",
    "codigo de verificacion",
    "código de acceso",
    "bestätigungscode",
    "anmeldecode",
    "sicherheitscode",
    "验证码",
    "認証コード",
    "код подтверждения",
];

fn candidate(token: &str) -> Option<String> {
    let token = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
    if token.is_empty() || token.starts_with('-') || token.ends_with('-') {
        return None;
    }
    let compact: String = token.chars().filter(|c| *c != '-').collect();
    let digits = compact.chars().filter(char::is_ascii_digit).count();
    let letters = compact.chars().filter(char::is_ascii_alphabetic).count();
    if digits == 0 || !compact.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let valid = if letters == 0 {
        (4..=8).contains(&digits) && compact.parse::<u32>().is_ok_and(|number| !(1900..=2100).contains(&number))
    } else {
        (5..=10).contains(&compact.len()) && letters >= 1
    };
    valid.then(|| token.to_ascii_uppercase())
}

/// Extract only near an explicit verification/login phrase. This avoids treating dates, prices,
/// order numbers and arbitrary short numbers as authentication codes.
pub fn extract(card: &EmailCard) -> Option<VerificationDto> {
    let text = format!("{}\n{}", card.subject, card.body_text);
    let lower = text.to_lowercase();
    let context =
        CONTEXTS.iter().filter_map(|phrase| lower.find(phrase).map(|at| (at, phrase.len()))).min_by_key(|x| x.0)?;
    let start = context.0 + context.1;
    let nearby: String = lower.get(start..)?.chars().take(140).collect();
    let code = nearby.split_whitespace().find_map(candidate)?;
    Some(VerificationDto {
        uid: card.uid,
        code,
        service: if card.from_name.is_empty() { card.from_email.clone() } else { card.from_name.clone() },
        from_email: card.from_email.clone(),
        subject: card.subject.clone(),
        expires_ts: card.date_ts.saturating_add(LIFETIME_SECONDS),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EmailCard, Priority};

    fn card(subject: &str, body: &str) -> EmailCard {
        EmailCard {
            uid: 1,
            subject: subject.into(),
            from_name: "Example".into(),
            from_email: "security@example.test".into(),
            to: String::new(),
            cc: String::new(),
            reply_to: String::new(),
            bcc: String::new(),
            message_id: String::new(),
            references: String::new(),
            date_str: String::new(),
            date_full: String::new(),
            date_ts: 1_800_000_000,
            summary: String::new(),
            priority: Priority::Information,
            is_read: false,
            body_text: body.into(),
            body_html: String::new(),
            attachments: vec![],
            project: None,
            deadline: None,
            task: None,
            meeting_link: None,
        }
    }

    #[test]
    fn extracts_contextual_codes_without_mistaking_common_numbers() {
        let item = extract(&card("Login", "Your verification code is 847-219. It expires in 10 minutes.")).unwrap();
        assert_eq!(item.code, "847-219");
        assert_eq!(item.expires_ts, 1_800_000_000 + 10 * 60);
        assert_eq!(extract(&card("Connexion", "Votre code de connexion : AB3-K9P")).unwrap().code, "AB3-K9P");
        assert!(extract(&card("Invoice 847219", "Your order costs 2026 EUR and arrives in 10 minutes.")).is_none());
        assert!(extract(&card("Security code", "This policy was updated in 2026.")).is_none());
    }
}
