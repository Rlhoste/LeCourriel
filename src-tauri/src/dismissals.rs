//! Overview items the user swiped away, persisted in `dismissed.json`.
//! (`tasks` / `task_check` are legacy: tasks now live on the board in `tasks.json`,
//! and these only seed the starting column of tasks migrated from them.)
//! Kept apart from the email cache so a refresh rewriting the cache can't lose them.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use tracing::warn;

const DISMISSED_PATH: &str = "dismissed.json";

/// Serialises load-modify-save cycles (swipes and the AI task check can overlap).
static WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Dismissals {
    #[serde(default)]
    pub tasks: BTreeSet<u32>,
    #[serde(default)]
    pub deadlines: BTreeSet<u32>,
    #[serde(default)]
    pub meetings: BTreeSet<u32>,
    /// AI verdict per task uid: "done" or "obsolete".
    #[serde(default)]
    pub task_check: BTreeMap<u32, String>,
}

impl Dismissals {
    pub fn load() -> Self {
        match std::fs::read_to_string(crate::paths::file(DISMISSED_PATH)) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                warn!("dismissed.json parse error: {}", e);
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Load the latest file, apply `f`, and save, all under one lock.
    pub fn update<T>(f: impl FnOnce(&mut Self) -> T) -> T {
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut d = Self::load();
        let out = f(&mut d);
        d.save();
        crate::cloud::schedule_push(crate::cloud::Doc::Dismissed);
        out
    }

    fn save(&self) {
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(crate::paths::file(DISMISSED_PATH), json) {
                    warn!("dismissed.json write error: {}", e);
                }
            }
            Err(e) => warn!("dismissed.json serialize error: {}", e),
        }
    }

    /// Record a user dismissal. `kind` is "task", "deadline" or "meeting".
    pub fn dismiss(&mut self, kind: &str, uid: u32) -> Result<(), String> {
        match kind {
            "task" => self.tasks.insert(uid),
            "deadline" => self.deadlines.insert(uid),
            "meeting" => self.meetings.insert(uid),
            other => return Err(format!("Unknown item kind: {}", other)),
        };
        Ok(())
    }
}
