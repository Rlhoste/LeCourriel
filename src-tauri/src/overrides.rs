//! Manual changes to AI results, persisted in `overrides.json`.
//! Kept apart from the email cache: a refresh rewrites the cache from what it
//! loaded when it started, so an edit made meanwhile would be lost.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tracing::warn;

use crate::models::EmailCard;

const OVERRIDES_PATH: &str = "overrides.json";

/// Serialises load-modify-save cycles.
static WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Overrides {
    /// Project chosen by the user per inbox UID; "" means "no project".
    #[serde(default)]
    pub projects: BTreeMap<u32, String>,
}

impl Overrides {
    pub fn load() -> Self {
        match std::fs::read_to_string(OVERRIDES_PATH) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                warn!("overrides.json parse error: {}", e);
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    fn save(&self) {
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(OVERRIDES_PATH, json) {
                    warn!("overrides.json write error: {}", e);
                }
            }
            Err(e) => warn!("overrides.json serialize error: {}", e),
        }
    }

    /// Load the latest file, apply `f`, and save, all under one lock.
    pub fn update<T>(f: impl FnOnce(&mut Self) -> T) -> T {
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut o = Self::load();
        let out = f(&mut o);
        o.save();
        out
    }

    pub fn set_project(&mut self, uid: u32, project: Option<String>) {
        self.projects.insert(uid, project.unwrap_or_default());
    }

    /// Apply the user's overrides to a card.
    pub fn apply(&self, card: &mut EmailCard) {
        if let Some(p) = self.projects.get(&card.uid) {
            card.project = if p.is_empty() { None } else { Some(p.clone()) };
        }
    }
}
