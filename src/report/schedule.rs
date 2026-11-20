//! Report schedules: JSON <-> `Schedule`, next runs (local naive time, DST-free), and the systemd `OnCalendar` form.
//!
//! Monthly `"last"` has no direct OnCalendar spelling on every systemd, so it is written as days 28..31 and the
//! run itself skips when the date is not the last day (`needs_day_check`).

use chrono::{Datelike, Days, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Time {
    pub hour: u32,
    pub minute: u32,
}

impl Time {
    /// Strict `HH:MM`, 00:00..23:59.
    pub fn parse(s: &str) -> Result<Time, String> {
        let b = s.as_bytes();
        let ok = b.len() == 5 && b[2] == b':' && b[0].is_ascii_digit() && b[1].is_ascii_digit() && b[3].is_ascii_digit() && b[4].is_ascii_digit();
        if !ok {
            return Err("time must be HH:MM".to_string());
        }
        let hour: u32 = s[0..2].parse().unwrap_or(99);
        let minute: u32 = s[3..5].parse().unwrap_or(99);
        if hour > 23 || minute > 59 {
            return Err("time must be HH:MM between 00:00 and 23:59".to_string());
        }
        Ok(Time { hour, minute })
    }

    pub fn text(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    fn naive(&self) -> NaiveTime {
        NaiveTime::from_hms_opt(self.hour, self.minute, 0).unwrap_or(NaiveTime::MIN)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Day {
    /// 1..=28 (every month has it).
    Num(u32),
    Last,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Daily,
    Weekly(Weekday),
    Monthly(Day),
    /// Jan/Apr/Jul/Oct on this day (1..=28).
    Quarterly(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schedule {
    pub kind: Kind,
    pub time: Time,
}

pub fn weekday_name(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "mon",
        Weekday::Tue => "tue",
        Weekday::Wed => "wed",
        Weekday::Thu => "thu",
        Weekday::Fri => "fri",
        Weekday::Sat => "sat",
        Weekday::Sun => "sun",
    }
}

pub fn weekday_from(s: &str) -> Option<Weekday> {
    [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat, Weekday::Sun]
        .into_iter()
        .find(|w| weekday_name(*w) == s)
}

fn day_from_value(v: Option<&Value>, max_num: u32) -> Result<Day, String> {
    match v {
        Some(Value::String(s)) if s == "last" && max_num == 28 => Ok(Day::Last),
        Some(Value::Number(n)) => match n.as_u64() {
            Some(d) if d >= 1 && d <= max_num as u64 => Ok(Day::Num(d as u32)),
            _ => Err(format!("day must be 1..{max_num}{}", if max_num == 28 { " or \"last\"" } else { "" })),
        },
        _ => Err(format!("day must be 1..{max_num}{}", if max_num == 28 { " or \"last\"" } else { "" })),
    }
}

impl Schedule {
    /// Parse and validate the stored/request JSON. Unknown fields are refused so typos are not silently ignored.
    pub fn from_json(v: &Value) -> Result<Schedule, String> {
        let obj: &Map<String, Value> = v.as_object().ok_or("schedule must be an object")?;
        let kind = obj.get("kind").and_then(Value::as_str).ok_or("schedule.kind is required")?;
        let allowed: &[&str] = match kind {
            "daily" => &["kind", "time"],
            "weekly" => &["kind", "weekday", "time"],
            "monthly" | "quarterly" => &["kind", "day", "time"],
            _ => return Err(format!("unknown schedule kind {kind:?} (daily, weekly, monthly, quarterly)")),
        };
        if let Some(k) = obj.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(format!("schedule has unknown field {k:?}"));
        }
        let time = Time::parse(obj.get("time").and_then(Value::as_str).ok_or("schedule.time must be HH:MM")?)?;
        let kind = match kind {
            "daily" => Kind::Daily,
            "weekly" => {
                let w = obj.get("weekday").and_then(Value::as_str).and_then(weekday_from);
                Kind::Weekly(w.ok_or("weekday must be mon, tue, wed, thu, fri, sat or sun")?)
            }
            "monthly" => Kind::Monthly(day_from_value(obj.get("day"), 28)?),
            _ => match day_from_value(obj.get("day"), 28)? {
                Day::Num(n) => Kind::Quarterly(n),
                Day::Last => return Err("quarterly day must be 1..28".to_string()),
            },
        };
        Ok(Schedule { kind, time })
    }

    /// Build from the query string of `GET /schedule/preview` (`day` is a number or `last`).
    pub fn from_query(kind: &str, weekday: Option<&str>, day: Option<&str>, time: &str) -> Result<Schedule, String> {
        let mut m = Map::new();
        m.insert("kind".into(), json!(kind));
        m.insert("time".into(), json!(time));
        if let Some(w) = weekday {
            m.insert("weekday".into(), json!(w));
        }
        if let Some(d) = day {
            let v = match d.parse::<u64>() {
                Ok(n) if d.bytes().all(|b| b.is_ascii_digit()) => json!(n),
                _ => json!(d),
            };
            m.insert("day".into(), v);
        }
        Schedule::from_json(&Value::Object(m))
    }

    pub fn to_json(&self) -> Value {
        let t = self.time.text();
        match self.kind {
            Kind::Daily => json!({ "kind": "daily", "time": t }),
            Kind::Weekly(w) => json!({ "kind": "weekly", "weekday": weekday_name(w), "time": t }),
            Kind::Monthly(d) => json!({ "kind": "monthly", "day": day_json(d), "time": t }),
            Kind::Quarterly(n) => json!({ "kind": "quarterly", "day": n, "time": t }),
        }
    }

    /// Does the schedule fire on this calendar date?
    pub fn fires_on(&self, d: NaiveDate) -> bool {
        match self.kind {
            Kind::Daily => true,
            Kind::Weekly(w) => d.weekday() == w,
            Kind::Monthly(Day::Num(n)) => d.day() == n,
            Kind::Monthly(Day::Last) => is_last_day(d),
            Kind::Quarterly(n) => matches!(d.month(), 1 | 4 | 7 | 10) && d.day() == n,
        }
    }

    /// True when OnCalendar is coarser than the schedule, so the timer run must check the date itself.
    pub fn needs_day_check(&self) -> bool {
        matches!(self.kind, Kind::Monthly(Day::Last))
    }

    /// The first `n` runs strictly after `after`.
    pub fn next_runs(&self, after: NaiveDateTime, n: usize) -> Vec<NaiveDateTime> {
        let mut out = Vec::with_capacity(n);
        let mut d = after.date();
        // A decade of days is far beyond any schedule's gap (quarterly is at most ~92 days).
        for _ in 0..(366 * 10) {
            if out.len() >= n {
                break;
            }
            if self.fires_on(d) {
                let at = d.and_time(self.time.naive());
                if at > after {
                    out.push(at);
                }
            }
            match d.checked_add_days(Days::new(1)) {
                Some(next) => d = next,
                None => break,
            }
        }
        out
    }

    /// systemd `OnCalendar=` value.
    pub fn to_oncalendar(&self) -> String {
        let t = self.time.text();
        match self.kind {
            Kind::Daily => format!("*-*-* {t}:00"),
            Kind::Weekly(w) => format!("{} *-*-* {t}:00", capitalize(weekday_name(w))),
            Kind::Monthly(Day::Num(n)) => format!("*-*-{n:02} {t}:00"),
            Kind::Monthly(Day::Last) => format!("*-*-28,29,30,31 {t}:00"),
            Kind::Quarterly(n) => format!("*-01,04,07,10-{n:02} {t}:00"),
        }
    }
}

fn day_json(d: Day) -> Value {
    match d {
        Day::Num(n) => json!(n),
        Day::Last => json!("last"),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn is_last_day(d: NaiveDate) -> bool {
    d.checked_add_days(Days::new(1)).is_some_and(|n| n.day() == 1)
}
