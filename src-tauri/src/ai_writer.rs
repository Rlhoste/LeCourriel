//! AI writing helpers for the composer: make formal, translate to English,
//! correct, and draft a whole reply. Input and output are HTML fragments from
//! the editor; the UI swaps images for placeholders before sending them here.

use anyhow::Result;

use crate::config::Config;

/// Context of the email being answered, for the auto-reply.
pub struct Original {
    pub subject: String,
    pub from: String,
    pub date: String,
    pub text: String,
}

const HTML_RULES: &str = "The input is an HTML fragment from an email editor. \
Respond ONLY with the resulting HTML fragment: no explanation, no markdown, no code fences, \
no <html>, <head> or <body> tags. Keep the existing formatting (bold, italic, underline, colours, \
fonts, alignment, lists, links, line breaks and paragraphs). Keep every <img> tag exactly as it is, \
at the same place. Do not add a subject line.";

fn system_prompt(action: &str, user_name: &str) -> Option<String> {
    let task = match action {
        "formal" => "Rewrite the text in a formal, polite and professional register, suitable for \
            a business or academic email. Keep the same language as the input. Keep the meaning and \
            every piece of information; do not add facts. Use formal greetings and sign-offs where \
            the text has them."
            .to_string(),
        "translate_en" => "Translate the text into natural, fluent English. Keep the tone and \
            register of the original. If it is already in English, return it unchanged."
            .to_string(),
        "translate_fr" => "Translate the text into natural, fluent French. Keep the tone and \
            register of the original (tutoiement or vouvoiement as fits). If it is already in \
            French, return it unchanged."
            .to_string(),
        "translate_ui" => format!(
            "Translate the text into natural, fluent {lang}. Keep the tone and register of the \
            original. If it is already in {lang}, return it unchanged.",
            lang = crate::i18n::ai_language()
        ),
        "correct" => "Correct spelling, grammar, punctuation, accents and typos only. Keep the same \
            language, tone, wording and structure; do not rephrase sentences that are already \
            correct."
            .to_string(),
        "auto_reply" => format!(
            "You write email replies on behalf of {name}. Write a complete reply to the original \
            email given below. Reply in the same language as the original email. If draft notes \
            from {name} are given, follow them: they say what the reply must contain. Be clear, \
            concise and polite; match the formality of the original. Do not invent facts, dates, \
            prices or commitments that the notes or the original do not support; write a short \
            placeholder in [square brackets] where information is missing. End with a sign-off \
            and the name {name}. Do not quote the original email.",
            name = if user_name.trim().is_empty() { "the user" } else { user_name }
        ),
        _ => return None,
    };
    Some(format!("{}\n\n{}", task, HTML_RULES))
}

/// Remove a ```html … ``` fence if the model added one anyway.
pub fn strip_fences(text: &str) -> String {
    let t = text.trim();
    let t = t.strip_prefix("```html").or_else(|| t.strip_prefix("```")).unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t);
    t.trim().to_string()
}

pub async fn run(cfg: &Config, action: &str, html: &str, original: Option<Original>) -> Result<String> {
    let system =
        system_prompt(action, &cfg.user_name()).ok_or_else(|| anyhow::anyhow!("Unknown AI action: {}", action))?;

    let user_content = if action == "auto_reply" {
        let o = original.ok_or_else(|| anyhow::anyhow!("Auto answer works when replying to an email"))?;
        let body: String = o.text.chars().take(6000).collect();
        let notes = if html.trim().is_empty() { "(none)".to_string() } else { html.to_string() };
        format!(
            "ORIGINAL EMAIL\nFrom: {}\nDate: {}\nSubject: {}\n\n{}\n\nDRAFT NOTES FROM {} (HTML):\n{}",
            o.from,
            o.date,
            o.subject,
            body,
            cfg.user_name(),
            notes
        )
    } else {
        if html.trim().is_empty() {
            anyhow::bail!("Write something first");
        }
        html.to_string()
    };

    let text = crate::ai::complete(
        &cfg.ai,
        crate::ai::Request { system: &system, user: &user_content, max_tokens: 4000, json: false },
    )
    .await?;
    let out = strip_fences(&text);
    if out.is_empty() {
        anyhow::bail!("The AI returned an empty answer");
    }
    Ok(out)
}
