//! Kanban task board, persisted in `tasks.json`.
//! Tasks come from two places: the AI extracting an action item from an inbox
//! email (`source_uid` set), and tasks the user creates by hand.

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::dismissals::Dismissals;
use crate::models::{EmailCard, Priority};

const TASKS_PATH: &str = "tasks.json";

/// Serialises load-modify-save cycles (UI edits and refresh can overlap).
static WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Todo,
    InProgress,
    Done,
    GivenUp,
}

impl Status {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "todo" => Some(Status::Todo),
            "in_progress" => Some(Status::InProgress),
            "done" => Some(Status::Done),
            "given_up" => Some(Status::GivenUp),
            _ => None,
        }
    }

    pub fn is_open(self) -> bool {
        matches!(self, Status::Todo | Status::InProgress)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub status: Status,
    /// Project name, or None for tasks outside any project.
    #[serde(default)]
    pub project: Option<String>,
    /// Optional calendar date (YYYY-MM-DD) chosen by the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    /// Inbox UID of the email this task was extracted from.
    #[serde(default)]
    pub source_uid: Option<u64>,
    /// Sender and subject of the source email, for display.
    #[serde(default)]
    pub source_from: String,
    #[serde(default)]
    pub source_subject: String,
    /// Set when the AI moved the task ("done" / "obsolete").
    #[serde(default)]
    pub ai_verdict: Option<String>,
    /// When the task appeared (Unix seconds): the date of its email, or when it was
    /// added by hand. 0 = unknown (hand-made tasks from before dates were kept).
    #[serde(default)]
    pub created_ts: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Board {
    #[serde(default)]
    pub next_id: u64,
    #[serde(default)]
    pub tasks: Vec<Task>,
    /// Source UIDs of email tasks the user deleted, so sync doesn't re-add them.
    #[serde(default)]
    pub deleted_sources: std::collections::BTreeSet<u64>,
}

impl Board {
    pub fn load() -> Self {
        match std::fs::read_to_string(crate::paths::file(TASKS_PATH)) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                warn!("tasks.json parse error: {}", e);
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    fn save(&self) {
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(crate::paths::file(TASKS_PATH), json) {
                    warn!("tasks.json write error: {}", e);
                }
            }
            Err(e) => warn!("tasks.json serialize error: {}", e),
        }
    }

    /// Load the latest file, apply `f`, and save, all under one lock.
    pub fn update<T>(f: impl FnOnce(&mut Self) -> T) -> T {
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut b = Self::load();
        let out = f(&mut b);
        b.save();
        crate::cloud::schedule_push(crate::cloud::Doc::Tasks);
        out
    }

    #[cfg(test)]
    pub fn add(&mut self, title: String, project: Option<String>) -> u64 {
        self.add_dated(title, project, None)
    }

    pub fn add_dated(&mut self, title: String, project: Option<String>, due_date: Option<String>) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.tasks.push(Task {
            id,
            title,
            status: Status::Todo,
            project,
            due_date,
            source_uid: None,
            source_from: String::new(),
            source_subject: String::new(),
            ai_verdict: None,
            created_ts: chrono::Utc::now().timestamp(),
        });
        id
    }

    pub fn set_status(&mut self, id: u64, status: Status) -> Result<(), String> {
        let t = self.tasks.iter_mut().find(|t| t.id == id).ok_or("Task not found")?;
        t.status = status;
        Ok(())
    }

    pub fn delete(&mut self, id: u64) -> Result<(), String> {
        let pos = self.tasks.iter().position(|t| t.id == id).ok_or("Task not found")?;
        let t = self.tasks.remove(pos);
        if let Some(uid) = t.source_uid {
            self.deleted_sources.insert(uid);
        }
        Ok(())
    }

    /// Whether `sync_from_cards` has something to do: an email action item not on the
    /// board yet, or an email task still without its date.
    pub fn needs_sync(&self, cards: &[EmailCard]) -> bool {
        cards.iter().any(|c| {
            ((c.task.is_some() || c.priority == Priority::Task)
                && !self.deleted_sources.contains(&c.uid)
                && !self.tasks.iter().any(|t| t.source_uid == Some(c.uid)))
                || self.tasks.iter().any(|t| t.created_ts == 0 && c.date_ts != 0 && t.source_uid == Some(c.uid))
        })
    }

    /// Add a Todo task for every email action item not on the board yet.
    /// Earlier swipes / AI verdicts from `dismissed.json` decide the starting column.
    /// Email tasks from before dates were kept get the date of their email.
    /// Returns true when something was added.
    pub fn sync_from_cards(&mut self, cards: &[EmailCard], dismissed: &Dismissals) -> bool {
        for t in self.tasks.iter_mut().filter(|t| t.created_ts == 0) {
            if let Some(c) = t.source_uid.and_then(|uid| cards.iter().find(|c| c.uid == uid)) {
                t.created_ts = c.date_ts;
            }
        }
        let mut added = false;
        for c in cards {
            if c.task.is_none() && c.priority != Priority::Task {
                continue;
            }
            if self.deleted_sources.contains(&c.uid) || self.tasks.iter().any(|t| t.source_uid == Some(c.uid)) {
                continue;
            }
            let verdict = dismissed.task_check.get(&c.uid).cloned();
            let status = match verdict.as_deref() {
                Some("done") => Status::Done,
                Some(_) => Status::GivenUp,
                None if dismissed.tasks.contains(&c.uid) => Status::GivenUp,
                None => Status::Todo,
            };
            self.next_id += 1;
            self.tasks.push(Task {
                id: self.next_id,
                title: c.task.clone().unwrap_or_else(|| c.subject.clone()),
                status,
                project: c.project.clone(),
                due_date: None,
                source_uid: Some(c.uid),
                source_from: c.from_name.clone(),
                source_subject: c.subject.clone(),
                ai_verdict: verdict,
                // The task appeared with its email (now, if the email has no date).
                created_ts: if c.date_ts != 0 { c.date_ts } else { chrono::Utc::now().timestamp() },
            });
            added = true;
        }
        added
    }
}
