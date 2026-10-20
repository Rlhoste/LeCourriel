<p align="center"><img src="assets/logo-ceres.png" alt="Le courriel logo" width="160"></p>

# Le courriel

An AI-assisted desktop email client built with [Tauri 2](https://tauri.app/) (Rust backend, HTML/JS UI).
It connects to any IMAP/SMTP mailbox and uses Claude to triage your inbox.

## Features

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

## Requirements

- **Rust** (stable): <https://rustup.rs/>
- **Tauri 2 system libraries**. On Debian/Ubuntu:

  ```sh
  sudo apt update
  sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
    libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
  ```

  For other systems, see <https://tauri.app/start/prerequisites/>.
- An **IMAP/SMTP** email account. Gmail needs an [App Password](https://myaccount.google.com/apppasswords).
- An **Anthropic API key** for the AI features: <https://console.anthropic.com/>
- *Optional:* a **GIPHY API key** for the GIF picker: <https://developers.giphy.com/>

## Setup

```sh
git clone https://github.com/Rlhoste/LeCourriel.git
cd LeCourriel
cp config.toml.example config.toml
```

Edit `config.toml` with your mailbox, SMTP server, Claude key and, optionally, your projects.
You can change most of these later in the app under **⚙ Settings**.

> `config.toml` holds your passwords and is git-ignored. Never commit it.

## Build and run

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

## Sync with your PostgreSQL (optional)

Le courriel can sync its data with **your own PostgreSQL server**, so a second computer gets
your tasks, preferences and AI results without paying to re-classify every email.
Local files remain the working copy: the app keeps working offline and syncs at the next start.

1. On the server, create a database and a user:

   ```sql
   CREATE DATABASE courriel;
   CREATE USER courriel WITH PASSWORD 'choose-a-password';
   GRANT ALL PRIVILEGES ON DATABASE courriel TO courriel;
   \c courriel
   GRANT ALL ON SCHEMA public TO courriel;
   ```

2. In the app, open **⚙ Settings → Database**, paste the URL and press **Test connection**:

   ```
   postgres://courriel:choose-a-password@db.example.com:5432/courriel?sslmode=require
   ```

   `sslmode` can be `disable`, `prefer` (default) or `require`. Tick **Accept self-signed
   certificates** if your server uses one. Then press **Save**; the first sync runs at once.

The app creates its two tables itself (`courriel_documents`, `courriel_ai_results`). Rows are keyed
by your mailbox (`user@imap-host`), so several mailboxes can share one database.

| Synced | Never synced |
|---|---|
| Task board, swipe dismissals, project tags | Mail passwords (IMAP/SMTP) |
| Projects, display name, font size, theme, language, Claude model | Claude and GIPHY API keys |
| AI results per email (priority, summary, project, deadline, task) — no email bodies | The database URL itself |

When both sides changed, the most recent version wins (per document).

To check the sync against your server:

```sh
TEST_DATABASE_URL='postgres://…' cargo test --manifest-path src-tauri/Cargo.toml -- --ignored
```

## Files the app creates

These files are written next to `config.toml` and are git-ignored:

| File | Contents |
|------|----------|
| `cache-v2.json` | Classified emails, so the AI doesn't re-classify them on every start |
| `tasks.json` | The task board |
| `dismissed.json` | Overview items you swiped away |
| `overrides.json` | Projects you assigned by hand |

Delete `cache-v2.json` to force a full re-classification.

## Privacy

Email content is sent to the Anthropic API to be classified:
- each **new** email: subject, sender and the first 800 characters;
- the task check and the composer's AI tools, when used.

Nothing else leaves your machine except normal IMAP/SMTP traffic and GIPHY searches.

## Project layout

```
src-tauri/   Rust backend (Tauri app): IMAP, SMTP, AI, storage
ui/          Front end: index.html, app.js, styles.css
src/, ui/*.slint, Cargo.toml (root)   Earlier Slint prototype, not used by the Tauri app
```

## License

[GNU General Public License v3.0 or later](LICENSE). Copyright (C) 2026 Remi Lhoste.
