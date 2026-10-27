# Le courriel

An AI-assisted desktop mail client: it reads your mailboxes, triages each email and turns what it finds into tasks, deadlines and briefs.

## Accounts

**Account**:
One mailbox the user reads and sends from, with its own server settings, sender identity and signature.
_Avoid_: Mailbox (that is the IMAP folder), profile, connection

**Server settings**:
The IMAP and SMTP host, port, security and username an account connects with. Passwords are not part of them.
_Avoid_: Connection, mail infos, config

**Auto-setup**:
Finding an account's server settings from its email address alone, so the user only has to type a password.
_Avoid_: Autoconfig (that is one source it uses), autodetect, auto-connection

**Auto-scan**:
Looking through the other mail apps on this computer for accounts the user can add with their server settings already filled in.
_Avoid_: Import, detect, migration

## AI

**AI provider**:
The service the app sends emails to so they get triaged: Claude, OpenAI, Mistral, Ollama…
_Avoid_: Model (that is one choice inside a provider), AI connection, backend

**Subscription sign-in**:
Using an AI provider through the user's existing chat plan (Claude Pro, Max, Team or Enterprise), approved once in the browser, instead of an API key.
_Avoid_: Anthropic connection, OAuth, login
