use anyhow::Result;
use chrono::NaiveDate;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::config::{ClaudeConfig, ProjectConfig};
use crate::models::{Priority, RawEmail};

#[derive(Serialize)]
struct ClaudeRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<Message>,
}

#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ClaudeResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: Option<String>,
}

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

pub struct ClassifiedEmail {
    pub priority: Priority,
    pub summary: String,
    pub project: Option<String>,
    pub deadline: Option<NaiveDate>,
    pub task: Option<String>,
    pub meeting_link: Option<String>,
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

    prompt
}

pub async fn classify(
    client: &Client,
    cfg: &ClaudeConfig,
    email: &RawEmail,
    projects: &[ProjectConfig],
) -> Result<ClassifiedEmail> {
    let body_snippet: String = email.body_text.chars().take(800).collect();

    let user_content = format!(
        "Subject: {}\nFrom: {}\nBody:\n{}",
        email.subject, email.from, body_snippet
    );

    let system_prompt = build_system_prompt(projects);

    let request = ClaudeRequest {
        model: cfg.model.clone(),
        max_tokens: cfg.max_tokens,
        system: system_prompt,
        messages: vec![Message {
            role: "user".to_string(),
            content: user_content,
        }],
    };

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &cfg.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&request)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        warn!("Claude API error {}: {}", status, body);
        return Ok(ClassifiedEmail {
            priority: Priority::Information,
            summary: "Could not classify email.".to_string(),
            project: None,
            deadline: None,
            task: None,
            meeting_link: None,
        });
    }

    let claude_resp: ClaudeResponse = response.json().await?;

    let text = claude_resp
        .content
        .into_iter()
        .find(|b| b.block_type == "text")
        .and_then(|b| b.text)
        .unwrap_or_default();

    debug!("Claude raw response: {}", text);

    match serde_json::from_str::<ClassificationResult>(&text) {
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
            warn!("Failed to parse Claude JSON response: {} — raw: {}", e, text);
            Ok(ClassifiedEmail {
                priority: Priority::Information,
                summary: "Unable to summarize email.".to_string(),
                project: None,
                deadline: None,
                task: None,
                meeting_link: None,
            })
        }
    }
}
