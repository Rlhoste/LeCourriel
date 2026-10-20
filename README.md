<p align="center"><img src="assets/logo-ceres.png" alt="Le courriel logo" width="160"></p>

<p align="center"><a href="https://github.com/sponsors/Rlhoste"><img alt="Sponsor on GitHub" src="https://img.shields.io/badge/Sponsor-%E2%9D%A4-db61a2?logo=githubsponsors&amp;logoColor=white"></a></p>

# Le courriel

An AI-assisted desktop email client built with [Tauri 2](https://tauri.app/) (Rust backend, HTML/JS UI).
It connects to any IMAP/SMTP mailbox and uses AI to triage your inbox — **Claude, OpenAI, Mistral,
a local Ollama model, or any OpenAI-compatible API**, your choice.

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
- *Optional:* a **GIPHY API key** for the GIF picker: <https://developers.giphy.com/>

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

```sh
cargo test --manifest-path src-tauri/Cargo.toml
```

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
| **Claude** (Anthropic) | API key from <https://console.anthropic.com/> | Anthropic |
| **OpenAI** | API key from <https://platform.openai.com/> | OpenAI |
| **Mistral** | API key from <https://console.mistral.ai/> | Mistral (EU) |
| **Ollama** (local, free) | [Ollama](https://ollama.com) installed, then e.g. `ollama pull qwen2.5` | **Nowhere: everything stays on your computer** |
| **Other** | Any OpenAI-compatible API (Groq, OpenRouter, LM Studio, DeepSeek…): its URL, key and model | That provider |

Small local models are slower and classify less reliably than large cloud models; a 7–8B text
model (`qwen2.5`, `llama3.1`) is a good start. Keys are stored per provider in `config.toml`,
so you can switch back and forth.

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
**[GitHub Sponsors](https://github.com/sponsors/Rlhoste)**, monthly or one-time. You can also
use **❤ Sponsor** in the app's sidebar. Thank you!

## License

[GNU General Public License v3.0 or later](LICENSE). Copyright (C) 2026 Remi Lhoste.
