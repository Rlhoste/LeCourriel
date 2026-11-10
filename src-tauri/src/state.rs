use crate::config::Config;
use crate::models::EmailCard;
use crate::search::Folder;
use std::collections::HashSet;
use std::sync::Arc;

pub struct AppState {
    pub cfg: Arc<Config>,
    pub emails: Vec<EmailCard>,
    /// Every cached email tagged with its folder (UIDs repeat across folders), for search;
    /// also holds the results of the last server search (Folder::Server).
    pub search_index: Vec<(Folder, EmailCard)>,
    /// Whether `cache-v2.json` existed and decoded successfully in this process,
    /// or a refresh has since persisted a replacement.
    pub cache_loaded: bool,
    /// Secondary folders fetched during this session. Inbox is maintained by the
    /// incremental refresh; the others are loaded only when the user opens them.
    pub loaded_folders: HashSet<Folder>,
    /// The accounts and their special folders as of the refresh that fetched `emails`:
    /// an email id's account part is a position in this list (`models::mail_id`).
    pub mailboxes: Vec<(crate::config::AccountConfig, crate::app_state::SpecialFolders)>,
    /// Sent folder name of each account (same order as the config), found during refresh.
    pub sent_folders: Vec<String>,
    /// The briefing line last shown, kept when the screen is redrawn without a refresh.
    pub briefing: Option<crate::dto::Briefing>,
    /// Why config.toml could not be loaded at startup, shown once in the UI.
    pub startup_error: Option<String>,
}
