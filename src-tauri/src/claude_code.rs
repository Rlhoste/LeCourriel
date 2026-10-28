//! "Claude (subscription)": AI requests go through the user's own Claude Code CLI
//! (`claude -p`), which is signed in with their Claude subscription. The app never
//! sees or stores a token (see docs/adr/0001-claude-subscription-via-claude-code.md).

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;

use crate::ai::Request;

/// The AI provider name saved in the config for this route.
pub const PROVIDER: &str = "claude_code";

/// Model aliases Claude Code understands; each points at the latest model of that tier.
pub const MODELS: [&str; 3] = ["haiku", "sonnet", "opus"];

/// Command-line arguments for one question. The system prompt goes in a file and the
/// email on stdin: an npm install is a .cmd script, which can't take long or multi-line arguments.
pub fn args(model: &str, system_file: &str) -> Vec<String> {
    let mut a: Vec<String> =
        ["-p", "--output-format", "json", "--system-prompt-file", system_file].iter().map(|s| s.to_string()).collect();
    if !model.trim().is_empty() {
        a.extend(["--model".to_string(), model.trim().to_string()]);
    }
    // A plain question and answer: no tools, and none of the user's own settings,
    // hooks, MCP servers or commands (--bare would also do it, but it ignores the subscription login).
    a.extend(
        [
            "--tools",
            "",
            "--setting-sources",
            "",
            "--strict-mcp-config",
            "--disable-slash-commands",
            "--no-session-persistence",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    a
}

/// The answer text from `claude -p --output-format json`.
pub fn answer(stdout: &str) -> Result<String> {
    // The JSON result is the last line; anything printed before it is noise.
    let line = stdout.lines().rev().find(|l| l.trim_start().starts_with('{')).unwrap_or("");
    let v: Value = serde_json::from_str(line).context("Unexpected answer from Claude Code")?;
    let text = v.get("result").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if v.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
        bail!("Claude: {}", if text.is_empty() { "unknown error" } else { &text });
    }
    if text.is_empty() {
        bail!("The AI returned an empty answer");
    }
    Ok(text)
}

/// What Settings shows next to "Sign in with Claude".
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Status {
    /// The `claude` command was found.
    pub installed: bool,
    pub signed_in: bool,
    pub email: String,
    /// Signed in with a paid Claude plan (Pro, Max, Team or Enterprise).
    pub paid: bool,
}

/// Parse `claude auth status --json`.
pub fn parse_status(json: &str) -> Status {
    let v: Value = serde_json::from_str(json).unwrap_or(Value::Null);
    let str_of = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let signed_in = v.get("loggedIn").and_then(Value::as_bool).unwrap_or(false);
    let paid = signed_in
        && str_of("authMethod") == "claude.ai"
        && matches!(str_of("subscriptionType").as_str(), "pro" | "max" | "team" | "enterprise");
    Status { installed: true, signed_in, email: str_of("email"), paid }
}

/// `claude`, or on Windows the `claude.cmd` script an npm install creates.
fn command(program: &str) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(program);
    #[cfg(windows)]
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flashing up
    c
}

const PROGRAMS: &[&str] = if cfg!(windows) { &["claude", "claude.cmd"] } else { &["claude"] };

/// Spawn `claude` with these arguments, trying each program name until one exists.
/// `wait`: we read its output (all pipes, killed if dropped); otherwise it runs on its own.
fn spawn<S: AsRef<std::ffi::OsStr>>(args: &[S], wait: bool) -> Result<tokio::process::Child> {
    let mut last = std::io::Error::from(std::io::ErrorKind::NotFound);
    for p in PROGRAMS {
        let mut c = command(p);
        // A neutral folder, so no project's CLAUDE.md or settings get picked up.
        c.args(args).current_dir(std::env::temp_dir()).kill_on_drop(wait);
        let io = || if wait { Stdio::piped() } else { Stdio::null() };
        c.stdin(io()).stdout(io()).stderr(io());
        match c.spawn() {
            Ok(child) => return Ok(child),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => last = e,
            Err(e) => return Err(e.into()),
        }
    }
    Err(anyhow!(last).context("Claude Code is not installed on this computer"))
}

/// On Windows an npm install runs through claude.cmd, and killing it would leave the
/// node process running: end the whole process tree.
fn kill_tree(pid: Option<u32>) {
    #[cfg(windows)]
    if let Some(pid) = pid {
        let _ = command("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    #[cfg(not(windows))]
    let _ = pid; // kill_on_drop is enough: there is no wrapper script
}

/// Send one question through Claude Code and return the answer text.
pub async fn complete(model: &str, req: &Request<'_>) -> Result<String> {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // In the app's own data folder, not the shared temp folder.
    let file = crate::paths::file(&format!("claude-prompt-{}.txt", n));
    std::fs::write(&file, req.system).context("Could not write the AI prompt")?;
    let file = std::path::absolute(&file).unwrap_or(file); // claude runs in another folder
    let result = run(model, &file, req.user).await;
    std::fs::remove_file(&file).ok();
    result
}

async fn run(model: &str, system_file: &std::path::Path, user: &str) -> Result<String> {
    let mut child = spawn(&args(model, &system_file.to_string_lossy()), true)?;
    let mut stdin = child.stdin.take().context("No stdin")?;
    let pid = child.id();
    let talk = async {
        // If claude exits early (e.g. not signed in), its own message below says why.
        stdin.write_all(user.as_bytes()).await.ok();
        drop(stdin);
        child.wait_with_output().await
    };
    let out = match tokio::time::timeout(std::time::Duration::from_secs(120), talk).await {
        Ok(out) => out?,
        Err(_) => {
            kill_tree(pid);
            bail!("The AI took too long to answer");
        }
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    answer(&stdout).map_err(|e| {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stdout.trim().is_empty() && !stderr.trim().is_empty() {
            anyhow!("Claude Code: {}", stderr.trim().chars().take(200).collect::<String>())
        } else {
            e
        }
    })
}

/// Is Claude Code installed, and signed in with a paid plan?
pub async fn status() -> Status {
    let child = match spawn(&["auth", "status", "--json"], true) {
        Ok(c) => c,
        Err(_) => return Status::default(),
    };
    match tokio::time::timeout(std::time::Duration::from_secs(20), child.wait_with_output()).await {
        Ok(Ok(out)) => parse_status(&String::from_utf8_lossy(&out.stdout)),
        _ => Status { installed: true, ..Default::default() },
    }
}

/// Start `claude auth login --claudeai`: it opens the browser and finishes on its own.
/// Settings polls `status()` to see when it is done.
pub fn sign_in() -> Result<()> {
    spawn(&["auth", "login", "--claudeai"], false).map(drop)
}

/// `claude auth logout`: signs Claude Code out on this computer.
pub async fn sign_out() -> Result<()> {
    let child = spawn(&["auth", "logout"], true)?;
    let out = tokio::time::timeout(std::time::Duration::from_secs(20), child.wait_with_output())
        .await
        .map_err(|_| anyhow!("Claude Code took too long to sign out"))??;
    if !out.status.success() {
        bail!("Claude Code: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}
