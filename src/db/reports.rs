//! Scheduled reports: CRUD on `report` and the `report_run` log (migration v3). Plain rows only; validation and
//! meaning live in `crate::report`. Nothing here logs row text.

use super::Db;
use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReportRow {
    pub id: i64,
    pub name: String,
    pub query: String,
    pub shape: String,
    pub period: String,
    pub schedule_json: String,
    pub recipients_json: String,
    pub redact: bool,
    pub enabled: bool,
    pub created_at: String,
}

/// The writable columns of a report (`enabled` is changed only through `report_set_enabled`).
#[derive(Debug, Clone, PartialEq)]
pub struct ReportFields {
    pub name: String,
    pub query: String,
    pub shape: String,
    pub period: String,
    pub schedule_json: String,
    pub recipients_json: String,
    pub redact: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RunRow {
    pub id: i64,
    pub report_id: i64,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: String,
    pub detail: Option<String>,
    pub rows: Option<i64>,
    pub message_id: Option<String>,
    pub trigger: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewRun {
    pub report_id: i64,
    pub started_at: String,
    pub finished_at: Option<String>,
    /// `ok` | `failed` | `skipped`
    pub status: String,
    pub detail: Option<String>,
    pub rows: Option<i64>,
    pub message_id: Option<String>,
    /// `timer` | `manual` | `test` | `catchup`
    pub trigger: String,
}

const REPORT_COLS: &str = "id, name, query, shape, period, schedule_json, recipients_json, redact, enabled, created_at";

fn report_from_row(r: &Row) -> rusqlite::Result<ReportRow> {
    Ok(ReportRow {
        id: r.get(0)?,
        name: r.get(1)?,
        query: r.get(2)?,
        shape: r.get(3)?,
        period: r.get(4)?,
        schedule_json: r.get(5)?,
        recipients_json: r.get(6)?,
        redact: r.get::<_, i64>(7)? != 0,
        enabled: r.get::<_, i64>(8)? != 0,
        created_at: r.get(9)?,
    })
}

fn run_from_row(r: &Row) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: r.get(0)?,
        report_id: r.get(1)?,
        started_at: r.get(2)?,
        finished_at: r.get(3)?,
        status: r.get(4)?,
        detail: r.get(5)?,
        rows: r.get(6)?,
        message_id: r.get(7)?,
        trigger: r.get(8)?,
    })
}

const RUN_COLS: &str = "id, report_id, started_at, finished_at, status, detail, rows, message_id, \"trigger\"";

impl Db {
    /// Every report, by name.
    pub fn reports_list(&self) -> Result<Vec<ReportRow>> {
        let conn = self.lock()?;
        let mut s = conn.prepare(&format!("SELECT {REPORT_COLS} FROM report ORDER BY name COLLATE NOCASE, id"))?;
        let rows = s.query_map([], report_from_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn report_get(&self, id: i64) -> Result<Option<ReportRow>> {
        let conn = self.lock()?;
        Ok(conn
            .query_row(&format!("SELECT {REPORT_COLS} FROM report WHERE id=?1"), [id], report_from_row)
            .optional()?)
    }

    /// True when another report (not `except`) already has this name (case-sensitive, as the UNIQUE index).
    pub fn report_name_taken(&self, name: &str, except: Option<i64>) -> Result<bool> {
        let conn = self.lock()?;
        let n: i64 = conn.query_row(
            "SELECT count(*) FROM report WHERE name=?1 AND id IS NOT ?2",
            params![name, except],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    pub fn report_insert(&self, f: &ReportFields) -> Result<i64> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO report(name, query, shape, period, schedule_json, recipients_json, redact, enabled)
             VALUES (?1,?2,?3,?4,?5,?6,?7,0)",
            params![f.name, f.query, f.shape, f.period, f.schedule_json, f.recipients_json, f.redact as i64],
        )
        .context("insert report")?;
        Ok(conn.last_insert_rowid())
    }

    /// Returns false when no such report exists.
    pub fn report_update(&self, id: i64, f: &ReportFields) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn
            .execute(
                "UPDATE report SET name=?2, query=?3, shape=?4, period=?5, schedule_json=?6, recipients_json=?7, redact=?8
                 WHERE id=?1",
                params![id, f.name, f.query, f.shape, f.period, f.schedule_json, f.recipients_json, f.redact as i64],
            )
            .context("update report")?;
        Ok(n > 0)
    }

    pub fn report_set_enabled(&self, id: i64, enabled: bool) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn.execute("UPDATE report SET enabled=?2 WHERE id=?1", params![id, enabled as i64])?;
        Ok(n > 0)
    }

    /// Deletes the report and, through `ON DELETE CASCADE`, its run log.
    pub fn report_delete(&self, id: i64) -> Result<bool> {
        let conn = self.lock()?;
        let n = conn.execute("DELETE FROM report WHERE id=?1", [id])?;
        Ok(n > 0)
    }

    pub fn run_insert(&self, r: &NewRun) -> Result<i64> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO report_run(report_id, started_at, finished_at, status, detail, rows, message_id, \"trigger\")
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![r.report_id, r.started_at, r.finished_at, r.status, r.detail, r.rows, r.message_id, r.trigger],
        )
        .context("insert report_run")?;
        Ok(conn.last_insert_rowid())
    }

    /// Newest first.
    pub fn runs_list(&self, report_id: i64, limit: i64) -> Result<Vec<RunRow>> {
        let conn = self.lock()?;
        let mut s = conn.prepare(&format!("SELECT {RUN_COLS} FROM report_run WHERE report_id=?1 ORDER BY id DESC LIMIT ?2"))?;
        let rows = s.query_map(params![report_id, limit.clamp(1, 500)], run_from_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn last_run(&self, report_id: i64) -> Result<Option<RunRow>> {
        let conn = self.lock()?;
        Ok(conn
            .query_row(
                &format!("SELECT {RUN_COLS} FROM report_run WHERE report_id=?1 ORDER BY id DESC LIMIT 1"),
                [report_id],
                run_from_row,
            )
            .optional()?)
    }
}
