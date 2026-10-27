<p align="center"><img src="assets/logo-ceres.png" alt="Le courriel logo" width="160"></p>

<p align="center"><a href="https://github.com/sponsors/Rlhoste"><img alt="Sponsor on GitHub" src="https://img.shields.io/badge/Sponsor-%E2%9D%A4-db61a2?logo=githubsponsors&amp;logoColor=white"></a> <a href="https://ko-fi.com/rlhoste"><img alt="Support on Ko-fi" src="https://img.shields.io/badge/Ko--fi-Support-ff5e5b?logo=kofi&amp;logoColor=white"></a></p>

# Le courriel

**An AI-boosted alternative to Thunderbird.** Free, open source, and it works with the mailbox you
already have.

We have never received so much information, and never had so little time to find what matters.
Hundreds of emails a week: newsletters, notifications, threads that go on forever. Somewhere in there
are the few messages that actually create value: the request from your client, the deadline your boss
mentioned in passing, the meeting that moved to Thursday. Finding them is a job in itself.

**Why not let an assistant do that job, and spend your time on what matters?**

Le courriel puts an AI assistant between you and your inbox. **It reads every new email for you:**
- it summarises it in one line and judges how urgent it is;
- it files it into the right project;
- it pulls out the tasks, deadlines and meetings hidden inside.

It does the tedious work and hands you only what's worth your time.

<p align="center"><img src="docs/screenshots/overview.png" alt="The Overview prepared by the AI: urgent emails, deadlines, tasks, meetings and the week ahead" width="900"></p>

## Your day, prepared by AI

Open the app and the triage is already done. The **Overview** answers four questions before you read
a single email:

- **Which emails really need me?** The AI reads each new email, ranks it (urgent, important, task,
  information, or even a phishing threat) and sums it up in one line.
- **What do I have to do?** The AI spots the requests buried in your messages and turns them into tasks.
- **Which deadlines are coming?** The AI finds the dates and due dates in your mail and counts them down.
- **Where is my next meeting?** The AI detects invitations and video-call links and gathers them in one place.

The week ahead sits underneath, so nothing slips.

## Tasks that manage themselves

A to-do list only helps if someone keeps it up to date. Here, the AI does:

- **It creates the tasks.** "Can you send me the figures by Thursday?" becomes a task, linked to the email it came from.
- **It sorts them.** Every email and task is filed into the right project, based on the descriptions
  you give your projects.
- **It closes them.** The AI reads what you send. When your reply shows a task is handled, it moves
  it to *Done*. When a later email makes it pointless, it marks it *obsolete*.

No one has to maintain the board, not even you.

<p align="center"><img src="docs/screenshots/tasks.png" alt="The task board, filled and updated by the AI" width="900"></p>

## Your AI writing partner

Write a rough draft and let the AI polish it. Or don't write at all:

- **Rewrite** your text in a more formal tone.
- **Correct** spelling and grammar in one click.
- **Translate** your message.
- **Draft the whole reply** from the email you're answering.

The AI only touches what you wrote, never your signature or the quoted message. Choose which address
sends it, and the matching signature follows.

<p align="center"><img src="docs/screenshots/compose.png" alt="Replying to an email with the AI writing tools" width="900"></p>

## Ask your inbox anything, and get an answer

Most search boxes give you a list of results and leave the reading to you. Here you can ask a real
question, the way you'd ask a colleague:

- *"What did we decide about the budget?"*
- *"Summarise this week's messages from Sara"*
- *"Does Julien have any insights about AI?"*

Press **Ctrl+K**. The AI reads the matching emails and **finds** the right ones, **summarises** them,
**answers** your question, or **draws a conclusion** across a dozen different threads. It always shows
which emails it used.

<p align="center"><img src="docs/screenshots/search.png" alt="The AI answering a question across several emails" width="900"></p>

## And everything you expect from a mail client

<table>
  <tr>
    <td width="50%"><img src="docs/screenshots/calendar.png" alt="Calendar with deadlines"><br><sub><b>Calendar</b>: every deadline the AI found in your mail, on its day.</sub></td>
    <td width="50%"><img src="docs/screenshots/inbox.png" alt="Inbox with the reading pane"><br><sub><b>Inbox</b>: a coloured bar shows the importance the AI gave each email; a box shows read/unread.</sub></td>
  </tr>
  <tr>
    <td><img src="docs/screenshots/settings.png" alt="Account settings with the connection test"><br><sub><b>Your accounts</b>: any IMAP/SMTP mailbox, a one-click connection test, and a signature per account.</sub></td>
    <td></td>
  </tr>
</table>

## Make it yours

*Don't forget to do some ricing.*

The AI gave you your time back. Spend a little of it making your mail client look exactly the way
you like.

<p align="center"><img src="docs/screenshots/themes.gif" alt="The Overview in the six themes: Premium, Midnight, Modern, Brutalist, Frutiger Aero and Terminal" width="900"></p>

- **Premium**: graphite grey and champagne gold, with a serif greeting.
- **Midnight**: the classic dark theme with blue accents.
- **Modern**: light, airy and rounded.
- **Brutalist**: thick black borders, loud yellow, monospace, hard shadows.
- **Frutiger Aero**: glossy bubbles and sky gradients, 2008 nostalgia included.
- **Terminal**: green on black, for those who never left the shell.

Switch in **Settings → General** with a live preview, adjust the text size, and pick one of
13 languages, including right-to-left Arabic and Urdu.

**Your AI, your choice.** Use Claude (with an API key, or with your Claude subscription through
Claude Code), OpenAI or Mistral. Or run a model on your own computer with Ollama, so your emails
never leave it. See [Choose your AI](#choose-your-ai).

## Features

- **13 languages**: English, Français, 中文, हिन्दी, Español, العربية (right-to-left), বাংলা, Русский,
  Português, اردو (right-to-left), Bahasa Indonesia, Deutsch, 日本語. Translations live in
  `ui/locales/`; corrections from native speakers are welcome (check them with `node ui/locales/check.js`).
- **Overview**: a morning brief with unread urgent and high-priority emails, deadlines, tasks and meetings.
  Swipe a row right to move an email to the trash or dismiss an item.
- **AI triage**: each new email is classified (urgent, high, task, information, threat…), summarised,
  assigned to one of your projects, and scanned for deadlines, action items and meeting links.
- **Tasks kanban**: Todo, In progress, Done and Given up, with one board per project. Tasks found in
  emails are added automatically, and the AI moves them to Done when your sent mail shows they're handled.
- **Calendar**: deadlines shown in their day's cell.
- **Reading pane**: Reply, Forward, Spam, Delete, Move to project, Mark as unread, Save as `.eml`, Print.
- **Gmail-style composer**: fonts, sizes, bold/italic/underline, colours, alignment, lists, links, images,
  GIFs (GIPHY), and AI tools (*make formal*, *translate to English*, *correct*, *auto answer*).
  Sent mail is saved to your IMAP Sent folder.

## Install

Ready-made packages are built by GitHub Actions (**Actions → Build installers**, or the
**Releases** page for tagged versions):

- **Windows**: `Le courriel_x.y.z_x64-setup.exe`. Run it; no administrator rights needed.
  Windows 10/11 already include the WebView2 runtime it uses (the installer fetches it otherwise).
- **Linux**: `Le courriel_x.y.z_amd64.AppImage`. Make it executable and run it:

  ```sh
  chmod +x "Le courriel_0.1.0_amd64.AppImage"
  ./"Le courriel_0.1.0_amd64.AppImage"
  ```

  (On Ubuntu 22.04+ install `libfuse2` if it complains about FUSE.)

On first start the app opens **Settings → Accounts** so you can add your mailbox; no
`config.toml` editing needed.

## Build from source

### Requirements



- **Rust** (stable): <https://rustup.rs/>
- **Tauri 2 system libraries**. On Debian/Ubuntu:

  ```sh
  sudo apt update
  sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
    libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
  ```

  For other systems, see <https://tauri.app/start/prerequisites/>.
- An **IMAP/SMTP** email account. Gmail needs an [App Password](https://myaccount.google.com/apppasswords).
- For the AI features, either an API key (Claude, OpenAI, Mistral…) or [Ollama](https://ollama.com)
  running locally — see [Choose your AI](#choose-your-ai).
- *Optional:* your own **GIPHY API key** for the GIF picker (a key is built in): <https://developers.giphy.com/>. GIFs are inserted as links to GIPHY, as its terms require, so they are not copied into the email.

### Setup

```sh
git clone https://github.com/Rlhoste/LeCourriel.git
cd LeCourriel
cp config.toml.example config.toml
```

Edit `config.toml` with your mailbox, SMTP server, Claude key and, optionally, your projects.
You can change most of these later in the app under **⚙ Settings**.

> `config.toml` holds your passwords and is git-ignored. Never commit it.

### Build and run

From the repository root:

```sh
cargo build --release --manifest-path src-tauri/Cargo.toml
./run.sh
```

`run.sh` starts the app from the repository root, which is where it looks for `config.toml`.
The first build takes a few minutes.

For a debug build with logs:

```sh
RUST_LOG=courriel=debug cargo run --manifest-path src-tauri/Cargo.toml
```

Run this from the repository root too.

### Tests

Run the complete deterministic suite (Rust backend, frontend behavior and contracts,
all locales, and updater manifest generation):

```sh
./scripts/test-all.sh
```

It ends with the end-to-end tests (`scripts/e2e.sh`): they start the real app window as on
a computer where it was just installed, fill in Settings like a user, and check what the
Overview, Inbox and Tasks show, against local fake IMAP and AI servers. They need
`sudo apt install webkit2gtk-driver xvfb` and `cargo install tauri-driver --locked`
(Linux only); `SKIP_E2E=1 ./scripts/test-all.sh` skips them. With `E2E_LIVE=1` and a real
mailbox and AI in environment variables (see `e2e/fresh-install.e2e.js`), they also run
against real servers.

The same suites run automatically on every push and pull request. Tests use isolated
temporary data and local mock servers; they do not read your mailbox credentials or
modify your Courriel data. The three tests that require real external services remain
ignored by default and can be run explicitly with `cargo test -- --ignored`.

### Packages

```sh
cd src-tauri
cargo tauri build --bundles appimage   # Linux → target/release/bundle/appimage/
cargo tauri build --bundles nsis       # on Windows → target/release/bundle/nsis/
```

## Choose your AI

**⚙ Settings → AI** lets you pick the provider, enter its API key, load its list of models and
**Test** the connection.

| Provider | What you need | Where your emails go |
|---|---|---|
| **Claude** (API) | API key from <https://console.anthropic.com/> (**Get a key** opens the page) | Anthropic |
| **Claude** (subscription) | [Claude Code](https://claude.com/claude-code) installed, then **Sign in with Claude** with a paid plan (Pro, Max, Team or Enterprise) | Anthropic |
| **OpenAI** | API key from <https://platform.openai.com/> | OpenAI |
| **Mistral** | API key from <https://console.mistral.ai/> | Mistral (EU) |
| **Ollama** (local, free) | [Ollama](https://ollama.com) installed, then e.g. `ollama pull qwen2.5` | **Nowhere: everything stays on your computer** |
| **Other** | Any OpenAI-compatible API (Groq, OpenRouter, LM Studio, DeepSeek…): its URL, key and model | That provider |

Small local models are slower and classify less reliably than large cloud models; a 7–8B text
model (`qwen2.5`, `llama3.1`) is a good start. Keys are stored per provider in `config.toml`,
so you can switch back and forth.

With **Claude (subscription)**, emails are sorted by your own Claude Code (`claude -p`, with no
tools and none of your Claude Code settings); the app stores no token. The choice is greyed out
when Claude Code isn't installed. If the plan's usage limit is reached, sorting stops and the
remaining emails are sorted at the next refresh. See
[ADR 0001](docs/adr/0001-claude-subscription-via-claude-code.md).

## Cloud sync with Supabase (optional)

Le courriel can sync its data with a [Supabase](https://supabase.com) project (PostgreSQL with
user accounts). Each person signs in with their own account and **only ever sees their own
data**: row-level security makes PostgreSQL itself refuse access to other users' rows, whatever
client is used. A second computer gets your tasks, preferences and AI results without paying to
re-classify every email. Local files remain the working copy: the app keeps working offline and
syncs at the next start.

### Set up the project (once)

1. Create a project at [supabase.com](https://supabase.com).
2. Open **SQL Editor**, paste the contents of [`supabase/schema.sql`](supabase/schema.sql) and press **Run**.
   It creates the tables, the row-level security policies and the sync functions (safe to run again).
3. In **Project Settings → API**, copy the **Project URL** and the **anon public** key.
4. *(Optional)* In **Authentication → Providers → Email**, keep "Confirm email" on if you want new
   accounts to confirm their address first.

### In the app

**⚙ Settings → Cloud**: paste the project URL and anon key, enter your email and password, then
**Sign in** (or **Create account**). The first sync runs at once; after that, every change is
uploaded in the background and the app syncs at each start.

| Synced (your rows only) | Never leaves your computer |
|---|---|
| Task board, swipe dismissals, project tags | Mail passwords (IMAP/SMTP) |
| Projects, display name, font size, theme, language, AI provider and model | AI and GIPHY API keys, the AI server URL |
| AI results per email (priority, summary, project, deadline, task) — no email bodies | The Supabase URL, key and your session (`session.json`) |

When the same document changed on two computers, the most recent version wins.

The data lives in plain relational tables (`tasks`, `projects`, `email_insights`, `user_prefs`…),
so a future company reporting system can query them — give it its own role and policies rather
than weakening the per-user ones.

### Tests

`supabase/schema.sql` and the sync code are tested against PostgreSQL 16 and PostgREST 12 (the
REST server Supabase uses). To run the end-to-end test against a project with the users
`alice@example.com` / `alice-pw` and `bob@example.com` / `bob-pw`:

```sh
CLOUD_TEST_URL=https://xxxx.supabase.co CLOUD_TEST_ANON=<anon key> \
  cargo test --manifest-path src-tauri/Cargo.toml test_cloud_e2e -- --ignored
```

## Where your data lives

| Platform | Folder |
|---|---|
| Linux | `~/.local/share/le-courriel` |
| Windows | `%APPDATA%\LeCourriel` |
| macOS | `~/Library/Application Support/LeCourriel` |

If the folder you start the app from contains a `config.toml` (development, `./run.sh`), that
folder is used instead. Set `COURRIEL_DATA_DIR` to choose another one.

The app writes these files there:

| File | Contents |
|------|----------|
| `cache-v2.json` | Classified emails, so the AI doesn't re-classify them on every start |
| `tasks.json` | The task board |
| `dismissed.json` | Overview items you swiped away |
| `overrides.json` | Projects you assigned by hand |
| `session.json` | Your Supabase sign-in (only if cloud sync is used; private to your user) |

Delete `cache-v2.json` to force a full re-classification.

## Privacy

Email content is sent to the AI provider you choose (see [Choose your AI](#choose-your-ai)) — or
stays on your computer with Ollama:
- each **new** email: subject, sender and the first 800 characters, to classify it;
- the task check, the search assistant and the composer's AI tools, when used.

Nothing else leaves your machine except normal IMAP/SMTP traffic, GIPHY searches and, if you
enable it, the Supabase sync (never your passwords or keys).

## Project layout

```
src-tauri/   Rust backend (Tauri app): IMAP, SMTP, AI, storage
ui/          Front end: index.html, app.js, styles.css
src/, ui/*.slint, Cargo.toml (root)   Earlier Slint prototype, not used by the Tauri app
```

## Support the project

Le courriel is free and open source. If it saves you time, you can support its development on
**[GitHub Sponsors](https://github.com/sponsors/Rlhoste)**, monthly or one-time, or on
**[Ko-fi](https://ko-fi.com/rlhoste)**. The **❤ Sponsor** button in the app's sidebar opens
the Ko-fi page. Thank you!

## License

[GNU General Public License v3.0 or later](LICENSE). Copyright (C) 2026 Remi Lhoste.
