//! Report definitions: shapes, periods, and the validation shared by the API and the headless CLI. Pure.

use super::schedule::Schedule;
use crate::db::reports::ReportRow;
use crate::search::{self, ParseError};
use chrono::{Datelike, Days, NaiveDate};

pub const MAX_NAME: usize = 60;
pub const MAX_RECIPIENTS: usize = 10;
pub const MAX_EMAIL: usize = 254;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Table,
    ByCategory,
    ByPayee,
    ByMonth,
    Summary,
}

impl Shape {
    pub const ALL: [Shape; 5] = [Shape::Table, Shape::ByCategory, Shape::ByPayee, Shape::ByMonth, Shape::Summary];

    pub fn name(self) -> &'static str {
        match self {
            Shape::Table => "table",
            Shape::ByCategory => "by_category",
            Shape::ByPayee => "by_payee",
            Shape::ByMonth => "by_month",
            Shape::Summary => "summary",
        }
    }

    pub fn from_name(s: &str) -> Option<Shape> {
        Shape::ALL.into_iter().find(|x| x.name() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Last7d,
    LastFullWeek,
    LastFullMonth,
    MonthToDate,
    LastFullQuarter,
    Ytd,
    All,
}

impl Period {
    pub const ALL: [Period; 7] = [
        Period::Last7d,
        Period::LastFullWeek,
        Period::LastFullMonth,
        Period::MonthToDate,
        Period::LastFullQuarter,
        Period::Ytd,
        Period::All,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Period::Last7d => "last_7d",
            Period::LastFullWeek => "last_full_week",
            Period::LastFullMonth => "last_full_month",
            Period::MonthToDate => "month_to_date",
            Period::LastFullQuarter => "last_full_quarter",
            Period::Ytd => "ytd",
            Period::All => "all",
        }
    }

    pub fn from_name(s: &str) -> Option<Period> {
        Period::ALL.into_iter().find(|x| x.name() == s)
    }

    /// Resolve against the local `today`. Last 7 days includes today; full weeks run Mon..Sun.
    pub fn resolve(self, today: NaiveDate) -> Span {
        let (from, to, title) = match self {
            Period::Last7d => (Some(today - Days::new(6)), Some(today), "last 7 days"),
            Period::LastFullWeek => {
                let this_monday = today - Days::new(today.weekday().num_days_from_monday() as u64);
                let from = this_monday - Days::new(7);
                (Some(from), Some(from + Days::new(6)), "last full week")
            }
            Period::LastFullMonth => {
                let to = first_of_month(today) - Days::new(1);
                (Some(first_of_month(to)), Some(to), "last full month")
            }
            Period::MonthToDate => (Some(first_of_month(today)), Some(today), "month to date"),
            Period::LastFullQuarter => {
                let to = quarter_start(today) - Days::new(1);
                (Some(quarter_start(to)), Some(to), "last full quarter")
            }
            Period::Ytd => (NaiveDate::from_ymd_opt(today.year(), 1, 1), Some(today), "year to date"),
            Period::All => return Span { from: None, to: None, label: "all time".to_string() },
        };
        let (f, t) = (from.unwrap_or(today), to.unwrap_or(today));
        Span { from, to, label: format!("{title} ({} to {})", f.format("%Y-%m-%d"), t.format("%Y-%m-%d")) }
    }
}

/// A resolved period. `None` bounds mean unbounded (`all`).
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub label: String,
}

impl Span {
    pub fn contains(&self, d: NaiveDate) -> bool {
        self.from.is_none_or(|f| d >= f) && self.to.is_none_or(|t| d <= t)
    }

    /// The period of equal length immediately before this one (none for unbounded spans).
    pub fn previous(&self) -> Option<Span> {
        let (f, t) = (self.from?, self.to?);
        let n = (t - f).num_days() + 1;
        let to = f - Days::new(1);
        let from = f - Days::new(n as u64);
        Some(Span {
            from: Some(from),
            to: Some(to),
            label: format!("{} to {}", from.format("%Y-%m-%d"), to.format("%Y-%m-%d")),
        })
    }
}

pub fn first_of_month(d: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap_or(d)
}

pub fn quarter_start(d: NaiveDate) -> NaiveDate {
    let m = ((d.month() - 1) / 3) * 3 + 1;
    NaiveDate::from_ymd_opt(d.year(), m, 1).unwrap_or(d)
}

/// A stored report, parsed. Built only from rows this code wrote, so a parse failure means a damaged row.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub id: i64,
    pub name: String,
    pub query: String,
    pub shape: Shape,
    pub period: Period,
    pub schedule: Schedule,
    pub recipients: Vec<String>,
    pub redact: bool,
    pub enabled: bool,
}

impl Report {
    pub fn from_row(r: &ReportRow) -> Result<Report, String> {
        let shape = Shape::from_name(&r.shape).ok_or("stored shape is not recognised")?;
        let period = Period::from_name(&r.period).ok_or("stored period is not recognised")?;
        let sv: serde_json::Value = serde_json::from_str(&r.schedule_json).map_err(|_| "stored schedule is not JSON")?;
        let schedule = Schedule::from_json(&sv)?;
        let recipients: Vec<String> = serde_json::from_str(&r.recipients_json).map_err(|_| "stored recipients are not a list")?;
        Ok(Report {
            id: r.id,
            name: r.name.clone(),
            query: r.query.clone(),
            shape,
            period,
            schedule,
            recipients,
            redact: r.redact,
            enabled: r.enabled,
        })
    }
}

/// Name: trimmed, 1..=60 characters, no control characters (it ends up in a mail subject and a file name).
pub fn check_name(s: &str) -> Result<String, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err("name is required".to_string());
    }
    if t.chars().count() > MAX_NAME {
        return Err(format!("name must be at most {MAX_NAME} characters"));
    }
    if t.chars().any(|c| c.is_control()) {
        return Err("name must not contain control characters".to_string());
    }
    Ok(t.to_string())
}

/// The query must parse with the same grammar as the UI. The error carries the parser's position and text.
pub fn check_query(q: &str, today: NaiveDate) -> Result<(), ParseError> {
    search::parse(q, today).map(|_| ())
}

/// Lowercased, trimmed, plausibly an address, and free of header-breaking characters.
pub fn check_email(s: &str) -> Result<String, String> {
    let t = s.trim().to_lowercase();
    let bad_char = t.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | ',' | ';' | '"' | '\\'));
    let parts: Vec<&str> = t.split('@').collect();
    let ok = !t.is_empty()
        && t.len() <= MAX_EMAIL
        && !bad_char
        && parts.len() == 2
        && !parts[0].is_empty()
        && parts[1].contains('.')
        && !parts[1].starts_with('.')
        && !parts[1].ends_with('.');
    if ok {
        Ok(t)
    } else {
        Err(format!("not an email address: {}", if t.is_empty() { "(empty)" } else { t.as_str() }))
    }
}

/// The one recipient rule: the authenticated sender, or an address on the settings allowlist.
pub fn recipient_allowed(allowlist: &[String], sender: Option<&str>, addr: &str) -> bool {
    let a = addr.trim().to_lowercase();
    if a.is_empty() {
        return false;
    }
    sender.is_some_and(|s| s.trim().to_lowercase() == a) || allowlist.iter().any(|x| x.trim().to_lowercase() == a)
}

/// Validate a recipient list. `None` means "default": the sender when known, else nobody.
pub fn check_recipients(list: Option<&[String]>, allowlist: &[String], sender: Option<&str>) -> Result<Vec<String>, String> {
    let Some(list) = list else {
        return Ok(sender.map(|s| vec![s.trim().to_lowercase()]).unwrap_or_default());
    };
    if list.len() > MAX_RECIPIENTS {
        return Err(format!("at most {MAX_RECIPIENTS} recipients"));
    }
    let mut out: Vec<String> = Vec::new();
    for raw in list {
        let a = check_email(raw)?;
        if !recipient_allowed(allowlist, sender, &a) {
            return Err(format!("recipient not allowed: {a} (add it to [reports] allowed_recipients)"));
        }
        if !out.contains(&a) {
            out.push(a);
        }
    }
    Ok(out)
}
