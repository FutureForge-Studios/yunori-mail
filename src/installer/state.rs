//! Persisted installer progress: one row per step plus a capped log. The
//! console renders exactly this, so a page refresh (or a lost SSH session)
//! never loses track of where the install is.

use crate::store::Db;
use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    Pending,
    Running,
    Done,
    Failed,
    Waiting,
    Skipped,
}

impl StepStatus {
    fn as_str(self) -> &'static str {
        match self {
            StepStatus::Pending => "pending",
            StepStatus::Running => "running",
            StepStatus::Done => "done",
            StepStatus::Failed => "failed",
            StepStatus::Waiting => "waiting",
            StepStatus::Skipped => "skipped",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "running" => StepStatus::Running,
            "done" => StepStatus::Done,
            "failed" => StepStatus::Failed,
            "waiting" => StepStatus::Waiting,
            "skipped" => StepStatus::Skipped,
            _ => StepStatus::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallStep {
    pub id: String,
    pub title: String,
    pub status: StepStatus,
    pub detail: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallLogLine {
    pub at: String,
    pub level: String,
    pub step: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallState {
    pub steps: Vec<InstallStep>,
    pub logs: Vec<InstallLogLine>,
}

impl InstallState {
    pub fn waiting_detail(&self) -> Option<String> {
        self.steps
            .iter()
            .find(|s| matches!(s.status, StepStatus::Waiting | StepStatus::Failed))
            .map(|s| s.detail.clone().unwrap_or_else(|| s.title.clone()))
    }
}

pub const STEPS: &[(&str, &str)] = &[
    ("preflight", "Check this server"),
    ("inputs", "Your domain"),
    ("host_record", "Point the mail hostname here"),
    ("firewall", "Open mail ports"),
    ("install_engine", "Install the mail engine"),
    ("bootstrap_engine", "Configure the mail engine"),
    ("edge_tls", "Start the console and HTTPS edge"),
    ("certificate", "Get a trusted TLS certificate"),
    ("dns", "Publish DNS records"),
    ("ptr", "Set reverse DNS"),
    ("first_mailbox", "Create your first mailbox"),
    ("done", "Finish"),
];

const MAX_LOG_LINES: i64 = 800;

impl Db {
    pub fn init_steps(&self) -> Result<()> {
        let conn = self.conn();
        // Steps an older release had and this one does not: drop them, so the
        // checklist only ever shows the steps that will actually run.
        let known: Vec<String> = STEPS.iter().map(|(id, _)| format!("'{id}'")).collect();
        conn.execute(
            &format!(
                "DELETE FROM install_steps WHERE id NOT IN ({})",
                known.join(",")
            ),
            [],
        )?;
        for (i, (id, title)) in STEPS.iter().enumerate() {
            conn.execute(
                "INSERT INTO install_steps (id, ord, title, status) VALUES (?1, ?2, ?3, 'pending')
                 ON CONFLICT(id) DO UPDATE SET ord = excluded.ord, title = excluded.title",
                params![id, i as i64, title],
            )?;
        }
        Ok(())
    }

    pub fn set_step(&self, id: &str, status: StepStatus, detail: Option<&str>) -> Result<()> {
        let now = crate::util::now_rfc3339();
        let conn = self.conn();
        match status {
            StepStatus::Running => conn.execute(
                "UPDATE install_steps SET status = ?2, detail = ?3, started_at = ?4, finished_at = NULL WHERE id = ?1",
                params![id, status.as_str(), detail, now],
            )?,
            StepStatus::Pending => conn.execute(
                "UPDATE install_steps SET status = ?2, detail = ?3, started_at = NULL, finished_at = NULL WHERE id = ?1",
                params![id, status.as_str(), detail],
            )?,
            _ => conn.execute(
                "UPDATE install_steps SET status = ?2, detail = ?3, finished_at = ?4 WHERE id = ?1",
                params![id, status.as_str(), detail, now],
            )?,
        };
        Ok(())
    }

    pub fn step_status(&self, id: &str) -> Result<StepStatus> {
        let s: String = self
            .conn()
            .query_row(
                "SELECT status FROM install_steps WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "pending".into());
        Ok(StepStatus::parse(&s))
    }

    pub fn install_log(&self, level: &str, step: Option<&str>, message: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO install_log (at, level, step, message) VALUES (?1, ?2, ?3, ?4)",
            params![crate::util::now_rfc3339(), level, step, message],
        )?;
        conn.execute(
            "DELETE FROM install_log WHERE id <= (SELECT MAX(id) FROM install_log) - ?1",
            params![MAX_LOG_LINES],
        )?;
        Ok(())
    }
}

pub fn load(db: &Db) -> Result<InstallState> {
    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT id, title, status, detail, started_at, finished_at FROM install_steps ORDER BY ord",
    )?;
    let steps = stmt
        .query_map([], |r| {
            Ok(InstallStep {
                id: r.get(0)?,
                title: r.get(1)?,
                status: StepStatus::parse(&r.get::<_, String>(2)?),
                detail: r.get(3)?,
                started_at: r.get(4)?,
                finished_at: r.get(5)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut stmt = conn.prepare(
        "SELECT at, level, step, message FROM (SELECT * FROM install_log ORDER BY id DESC LIMIT 200) ORDER BY id ASC",
    )?;
    let logs = stmt
        .query_map([], |r| {
            Ok(InstallLogLine {
                at: r.get(0)?,
                level: r.get(1)?,
                step: r.get(2)?,
                message: r.get(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(InstallState { steps, logs })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_and_logs_persist() {
        let db = Db::open_memory().unwrap();
        db.init_steps().unwrap();
        db.init_steps().unwrap(); // idempotent
        db.set_step("preflight", StepStatus::Running, None).unwrap();
        db.set_step("preflight", StepStatus::Done, Some("All checks passed"))
            .unwrap();
        db.set_step("dns", StepStatus::Waiting, Some("Publish the MX record"))
            .unwrap();
        db.install_log("info", Some("preflight"), "ok").unwrap();
        // A row from an older release disappears on the next init.
        db.conn()
            .execute(
                "INSERT INTO install_steps (id, ord, title, status) VALUES ('retired', 99, 'Old', 'done')",
                [],
            )
            .unwrap();
        db.init_steps().unwrap();
        let st = load(&db).unwrap();
        assert_eq!(st.steps.len(), STEPS.len());
        assert_eq!(st.steps[0].status, StepStatus::Done);
        assert!(st.steps[0].finished_at.is_some());
        assert_eq!(
            st.waiting_detail().as_deref(),
            Some("Publish the MX record")
        );
        assert_eq!(st.logs.len(), 1);
        assert_eq!(db.step_status("dns").unwrap(), StepStatus::Waiting);
    }
}
