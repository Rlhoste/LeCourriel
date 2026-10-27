# Subscription sign-in goes through Claude Code

Anthropic does not let third-party apps offer sign-in with a Claude plan unless it approves them first, and the plans (Pro, Max, Team, Enterprise) include no API credit. So for Claude, subscription sign-in runs the user's own Claude Code: `claude auth login --claudeai` for the browser approval, `claude auth status` to check for a paid plan, and `claude -p` to triage each email. The app never holds a Claude token. This is for internal O2GAME use only. Using Claude Code as another app's engine is a grey area in the terms.

## Considered Options

- **Reuse Claude Code's saved token, or the claude.ai / desktop app session cookies, to call the API directly**: rejected. It breaks the terms, it stops working whenever Anthropic changes it, and a mail app reading other apps' sessions is a security red flag.
- **Our own browser sign-in**: needs Anthropic's approval. Revisit if that approval arrives, and in any case before a public release.
