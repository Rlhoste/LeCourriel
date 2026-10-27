use crate::models::EmailCard;
use crate::search::Folder;
use crate::config::Config;
use std::sync::Arc;

pub struct AppState {
    pub cfg: Arc<Config>,
    pub emails: Vec<EmailCard>,
    /// Every cached email tagged with its folder (UIDs repeat across folders), for search;
    /// also holds the results of the last server search (Folder::Server).
    pub search_index: Vec<(Folder, EmailCard)>,
    /// Trash folder name of the first account, found during refresh.
    pub trash_folder: String,
    /// Spam/Junk folder name of the first account.
    pub spam_folder: String,
    /// Sent folder name of each account (same order as the config), found during refresh.
    pub sent_folders: Vec<String>,
    /// The briefing line last shown, kept when the screen is redrawn without a refresh.
    pub briefing: Option<crate::dto::Briefing>,
    /// Why config.toml could not be loaded at startup, shown once in the UI.
    pub startup_error: Option<String>,
}
