use anyhow::Result;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::config::{AiConfig, ProjectConfig};
use crate::models::{Priority, RawEmail};





#[derive(Deserialize)]
struct ClassificationResult {
    priority: String,
    summary: String,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    deadline: Option<String>,
    #[serde(default)]
    task: Option<String>,
    #[serde(default)]
    meeting_link: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClassifiedEmail {
    pub priority: Priority,
    pub summary: String,
    pub project: Option<String>,
    pub deadline: Option<NaiveDate>,
    pub task: Option<String>,
    pub meeting_link: Option<String>,
}

const CLASSIFY_FAILED: &str = "Could not classify email.";
const PARSE_FAILED: &str = "Unable to summarize email.";

/// False for the placeholder results returned when the API call or parsing failed;
/// those must not be stored and reused on other computers.
pub fn is_real_result(c: &ClassifiedEmail) -> bool {
    is_real_summary(&c.summary)
}

/// Whether an email's summary comes from the AI. Empty (never sorted, e.g. the AI was not
/// set up yet) and the failure placeholders (also found in caches of older versions) mean
/// the email still has to be sorted.
pub fn is_real_summary(summary: &str) -> bool {
    !summary.trim().is_empty() && summary != CLASSIFY_FAILED && summary != PARSE_FAILED
}

/// The result for an email the AI couldn't sort, or that wasn't sent to it: left unsorted
/// (empty summary), so it is sorted again at the next refresh.
pub fn unclassified() -> ClassifiedEmail {
    ClassifiedEmail {
        priority: Priority::Information,
        summary: String::new(),
        project: None,
        deadline: None,
        task: None,
        meeting_link: None,
    }
}

fn build_system_prompt(projects: &[ProjectConfig]) -> String {
    let mut prompt = String::from(
        r#"You are an email priority classifier. Given an email's subject and body, respond ONLY with a JSON object (no explanation, no markdown, no code fences) in this exact format:
{"priority": "<value>", "summary": "<one sentence>", "project": "<project name or null>", "deadline": "<ISO date YYYY-MM-DD or null>", "task": "<action item or null>", "meeting_link": "<URL or null>"}

Priority must be exactly one of: urgent, high_priority, other_priority, information, threat, task

Guidelines:
- urgent: requires immediate action or response today (deadlines, emergencies, critical issues)
- high_priority: important but not time-critical (project updates, client requests, meetings)
- threat: phishing, scams, suspicious links, security warnings
- task: action items, to-dos, requests that need scheduling
- information: newsletters, receipts, notifications, FYI emails
- other_priority: everything else

The summary must be a single concise sentence (max 15 words) describing what the email is about.

For the deadline field: extract a specific date if the email mentions one (e.g. "due Friday Nov 21" → "2026-11-21"), otherwise null.
For the task field: if the email requires a specific action, describe it concisely (max 10 words), otherwise null.
For the meeting_link field: if the email body contains a video meeting URL (Zoom https://zoom.us/j/..., Teams https://teams.microsoft.com/l/meetup-join/..., Google Meet https://meet.google.com/..., Webex https://webex.com/...), extract the full URL. Otherwise null.
"#,
    );

    if !projects.is_empty() {
        prompt.push_str("\nFor the project field, assign the email to one of these user-defined projects if it matches, otherwise null:\n");
        for p in projects {
            prompt.push_str(&format!(
                "- \"{}\": {} (urgency: {})\n",
                p.name, p.description, p.urgency
            ));
        }
        prompt.push_str("\nIf none match, use null for project.\n");
    } else {
        prompt.push_str("\nFor the project field: always use null (no projects configured).\n");
    }


    prompt.push_str(&format!(
        "\nWrite the summary and the task in {}, whatever the language of the email.\n",
        crate::i18n::ai_language()
    ));
    prompt
}

/// Fails only when the AI can't be reached (the error says why); an answer that can't be
/// read gives a placeholder result.
pub async fn classify(
    ai: &AiConfig,
    email: &RawEmail,
    projects: &[ProjectConfig],
) -> Result<ClassifiedEmail> {
    let body_snippet: String = email.body_text.chars().take(800).collect();
    let user_content = format!(
        "Subject: {}\nFrom: {}\nBody:\n{}",
        email.subject, email.from, body_snippet
    );
    let system_prompt = build_system_prompt(projects);
    let placeholder = |summary: &str| ClassifiedEmail {
        priority: Priority::Information,
        summary: summary.to_string(),
        project: None,
        deadline: None,
        task: None,
        meeting_link: None,
    };

    let text = match crate::ai::complete(ai, crate::ai::Request {
        system: &system_prompt,
        user: &user_content,
        max_tokens: ai.max_tokens,
        json: true,
    })
    .await
    {
        Ok(t) => t,
        Err(e) => {
            // Not a placeholder: the email stays unsorted and is tried again at the next refresh.
            warn!("AI classification failed: {:#}", e);
            return Err(e);
        }
    };
    debug!("AI raw response: {}", text);

    match serde_json::from_str::<ClassificationResult>(crate::ai::extract_json(&text)) {
        Ok(result) => {
            let priority = Priority::from_str(&result.priority);
            let deadline = result
                .deadline
                .as_deref()
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
            Ok(ClassifiedEmail {
                priority,
                summary: result.summary,
                project: result.project.filter(|p| p != "null" && !p.is_empty()),
                deadline,
                task: result.task.filter(|t| t != "null" && !t.is_empty()),
                meeting_link: result.meeting_link.filter(|m| m != "null" && !m.is_empty()),
            })
        }
        Err(e) => {
            warn!("Failed to parse the AI's JSON answer: {} — raw: {}", e, text);
            Ok(placeholder(PARSE_FAILED))
        }
    }
}

// ── Task completion check ────────────────────────────────────────────────────

/// An open task extracted from an inbox email.
pub struct OpenTask {
    pub uid: u32,
    pub task: String,
    pub subject: String,
    pub from: String,
    pub date: String,
}

/// A recent email (inbox or sent) used as evidence that a task is done or moot.
pub struct RecentMail {
    pub folder: &'static str,
    pub from: String,
    pub to: String,
    pub subject: String,
    pub date: String,
    pub snippet: String,
}

#[derive(Deserialize, Default)]
struct TaskCheckResult {
    #[serde(default)]
    done: Vec<u32>,
    #[serde(default)]
    obsolete: Vec<u32>,
}

const TASK_CHECK_PROMPT: &str = r#"You track a user's to-do list built from their emails. You get the open tasks (each with an id) and the user's recent emails, both received (inbox) and sent by the user (sent).

Decide for each task whether it is:
- done: a sent email or a later received email shows the action was carried out (e.g. the user replied with the requested document, confirmed attendance, or someone acknowledges receipt);
- obsolete: a later email cancels it, supersedes it, or makes it pointless (e.g. event cancelled, request withdrawn, deadline long passed with no follow-up needed);
- still open: otherwise. When unsure, leave it open.

Respond ONLY with a JSON object (no explanation, no markdown, no code fences):
{"done": [<task ids>], "obsolete": [<task ids>]}"#;

/// Parse the model's answer into (uid, verdict) pairs, ignoring ids that are not open tasks.
pub fn parse_task_check(text: &str, open_uids: &[u32]) -> Vec<(u32, &'static str)> {
    let trimmed = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let result: TaskCheckResult = match serde_json::from_str(trimmed) {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to parse task check response: {} — raw: {}", e, text);
            return vec![];
        }
    };
    let mut out = Vec::new();
    for uid in result.done {
        if open_uids.contains(&uid) {
            out.push((uid, "done"));
        }
    }
    for uid in result.obsolete {
        if open_uids.contains(&uid) && !out.iter().any(|(u, _)| *u == uid) {
            out.push((uid, "obsolete"));
        }
    }
    out
}

/// Ask Claude which open tasks are already done or no longer relevant.
pub async fn check_tasks(
    ai: &AiConfig,
    tasks: &[OpenTask],
    recent: &[RecentMail],
) -> Result<Vec<(u32, &'static str)>> {
    if tasks.is_empty() || recent.is_empty() {
        return Ok(vec![]);
    }

    let mut user_content = String::from("OPEN TASKS:\n");
    for t in tasks {
        user_content.push_str(&format!(
            "- id {}: \"{}\" (from email \"{}\" by {}, received {})\n",
            t.uid, t.task, t.subject, t.from, t.date
        ));
    }
    user_content.push_str("\nRECENT EMAILS:\n");
    for m in recent {
        user_content.push_str(&format!(
            "[{}] {} | From: {} | To: {} | Subject: {}\n{}\n---\n",
            m.folder, m.date, m.from, m.to, m.subject, m.snippet
        ));
    }

    let text = crate::ai::complete(ai, crate::ai::Request {
        system: TASK_CHECK_PROMPT,
        user: &user_content,
        max_tokens: 500,
        json: true,
    })
    .await?;
    debug!("Task check raw response: {}", text);

    let open_uids: Vec<u32> = tasks.iter().map(|t| t.uid).collect();
    Ok(parse_task_check(&text, &open_uids))
}
