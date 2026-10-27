//! One client for every AI provider the app supports.
//!
//! Only two wire formats exist: Anthropic's Messages API (Claude), and the OpenAI
//! Chat Completions API, which OpenAI, Mistral, Ollama and most others (Groq,
//! OpenRouter, LM Studio, DeepSeek, Gemini's OpenAI endpoint…) all speak.
//! Encoding and decoding are pure functions so they can be tested without a network.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::config::AiConfig;

/// One request: a system prompt, the user's content, an output limit, and whether
/// the answer must be a JSON object.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    pub system: &'a str,
    pub user: &'a str,
    pub max_tokens: u32,
    pub json: bool,
}

/// What goes over the wire: URL path (after the base URL), headers and JSON body.
#[derive(Debug, Clone, PartialEq)]
pub struct Encoded {
    pub path: &'static str,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

pub fn is_anthropic(ai: &AiConfig) -> bool {
    ai.provider() == "anthropic"
}

pub fn encode(ai: &AiConfig, req: &Request) -> Encoded {
    let key = ai.key();
    if is_anthropic(ai) {
        let mut headers = vec![("anthropic-version".to_string(), "2023-06-01".to_string())];
        if !key.is_empty() {
            headers.push(("x-api-key".to_string(), key.to_string()));
        }
        return Encoded {
            path: "/messages",
            headers,
            body: json!({
                "model": ai.model,
                "max_tokens": req.max_tokens,
                "system": req.system,
                "messages": [{ "role": "user", "content": req.user }],
            }),
        };
    }
    let mut headers = Vec::new();
    if !key.is_empty() {
        headers.push(("Authorization".to_string(), format!("Bearer {}", key)));
    }
    let mut body = json!({
        "model": ai.model,
        "messages": [
            { "role": "system", "content": req.system },
            { "role": "user", "content": req.user },
        ],
    });
    // OpenAI's newer models only accept max_completion_tokens; the others use max_tokens.
    let limit_field = if ai.provider() == "openai" { "max_completion_tokens" } else { "max_tokens" };
    body[limit_field] = json!(req.max_tokens);
    if req.json {
        body["response_format"] = json!({ "type": "json_object" });
    }
    Encoded { path: "/chat/completions", headers, body }
}

/// The answer text from a successful response.
pub fn decode(ai: &AiConfig, body: &Value) -> Result<String> {
    let text = if is_anthropic(ai) {
        body.get("content")
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("")
            })
    } else {
        body.pointer("/choices/0/message/content").and_then(Value::as_str).map(str::to_string)
    };
    text.filter(|t| !t.trim().is_empty())
        .ok_or_else(|| anyhow!("The AI returned an empty answer"))
}

/// A readable message from an error response body (each provider shapes it differently).
pub fn error_message(status: u16, body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        for ptr in ["/error/message", "/message", "/detail", "/error"] {
            if let Some(m) = v.pointer(ptr).and_then(Value::as_str) {
                if !m.is_empty() {
                    return format!("{} ({})", m, status);
                }
            }
        }
    }
    let body: String = body.chars().take(200).collect();
    format!("AI error {}: {}", status, body.trim())
}

/// Strip code fences and surrounding prose, keeping the first {...} object.
/// Small and local models often wrap JSON in text.
pub fn extract_json(text: &str) -> &str {
    let t = text.trim();
    let t = t.strip_prefix("```json").or_else(|| t.strip_prefix("```")).unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t).trim();
    match (t.find('{'), t.rfind('}')) {
        (Some(a), Some(b)) if a < b => &t[a..=b],
        _ => t,
    }
}

/// How many requests to send at once: a local model answers one at a time, cloud APIs
/// take several in parallel, Claude Code starts a process per request.
pub fn parallel_requests(ai: &AiConfig) -> usize {
    match ai.provider() {
        "ollama" => 1,
        crate::claude_code::PROVIDER => 3,
        _ => 6,
    }
}

fn client(ai: &AiConfig) -> Result<reqwest::Client> {
    // Local models are much slower than cloud APIs.
    let secs = if ai.provider() == "ollama" { 180 } else { 90 };
    Ok(reqwest::Client::builder().timeout(std::time::Duration::from_secs(secs)).build()?)
}

fn check_ready(ai: &AiConfig) -> Result<String> {
    let base = ai.base_url();
    if base.is_empty() {
        bail!("Enter the server URL of your AI provider in Settings → AI");
    }
    if ai.model.trim().is_empty() {
        bail!("Choose an AI model in Settings → AI");
    }
    if ai.key().is_empty() && !matches!(ai.provider(), "ollama" | "custom") {
        bail!("Add your {} API key in Settings → AI", provider_label(ai.provider()));
    }
    Ok(base)
}

pub fn provider_label(provider: &str) -> &'static str {
    match provider {
        "openai" => "OpenAI",
        "mistral" => "Mistral",
        "ollama" => "Ollama",
        "custom" => "AI",
        crate::claude_code::PROVIDER => "Claude (subscription)",
        _ => "Claude (Anthropic)",
    }
}

fn connect_error(ai: &AiConfig, e: reqwest::Error) -> anyhow::Error {
    if e.is_connect() && ai.provider() == "ollama" {
        anyhow!("Could not reach Ollama at {}. Is it running? (ollama serve)", ai.base_url())
    } else if e.is_timeout() {
        anyhow!("The AI took too long to answer")
    } else {
        anyhow!(e).context("Could not reach the AI provider")
    }
}

/// Send one request to the configured provider and return the answer text.
pub async fn complete(ai: &AiConfig, req: Request<'_>) -> Result<String> {
    if ai.uses_claude_code() {
        return crate::claude_code::complete(&ai.model, &req).await;
    }
    let base = check_ready(ai)?;
    let enc = encode(ai, &req);
    let mut builder = client(ai)?.post(format!("{}{}", base, enc.path)).json(&enc.body);
    for (k, v) in &enc.headers {
        builder = builder.header(k, v);
    }
    let resp = builder.send().await.map_err(|e| connect_error(ai, e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!(error_message(status.as_u16(), &text));
    }
    let body: Value = serde_json::from_str(&text).context("Unexpected answer from the AI provider")?;
    decode(ai, &body)
}

/// Model ids offered by the provider (GET {base}/models), sorted.
pub async fn list_models(ai: &AiConfig) -> Result<Vec<String>> {
    if ai.uses_claude_code() {
        return Ok(crate::claude_code::MODELS.iter().map(|m| m.to_string()).collect());
    }
    let base = ai.base_url();
    if base.is_empty() {
        bail!("Enter the server URL first");
    }
    let mut builder = client(ai)?.get(format!("{}/models", base));
    let key = ai.key();
    if is_anthropic(ai) {
        builder = builder.header("anthropic-version", "2023-06-01");
        if !key.is_empty() {
            builder = builder.header("x-api-key", key);
        }
    } else if !key.is_empty() {
        builder = builder.bearer_auth(key);
    }
    let resp = builder.send().await.map_err(|e| connect_error(ai, e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!(error_message(status.as_u16(), &text));
    }
    let body: Value = serde_json::from_str(&text).context("Unexpected answer from the AI provider")?;
    let mut ids: Vec<String> = body
        .get("data")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string)).collect())
        .unwrap_or_default();
    ids.sort();
    ids.dedup();
    Ok(ids)
}
