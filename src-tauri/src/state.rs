use crate::models::EmailCard;
use crate::config::Config;
use std::sync::Arc;

pub struct AppState {
    pub cfg: Arc<Config>,
    pub emails: Vec<EmailCard>,
    /// Trash folder name of the first account, found during refresh.
    pub trash_folder: String,
    /// Spam/Junk folder name of the first account.
    pub spam_folder: String,
    /// Sent folder name of the first account.
    pub sent_folder: String,
}
