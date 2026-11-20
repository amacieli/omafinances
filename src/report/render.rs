//! Rendering a computed report to subject, plain text, HTML and CSV. Pure: no I/O, no clock.
//!
//! HTML rules: inline styles only, no `<script>`, no `<img>`/`<link>`, no URL of any kind (remote content in mail is
//! both a tracking channel and a phishing vector). CSV rules: text cells that a spreadsheet would evaluate as a
//! formula (leading `= + - @`, tab or CR) get a leading apostrophe; numbers are written unprefixed so they stay numeric.

use super::model::{Report, Shape};
use super::run::{Computed, GroupLine, Summary, Totals, TxnLine, TABLE_HTML_CAP};
use crate::mail::{Attachment, OutMail};

#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    pub subject: String,
    pub text: String,
    pub html: String,
    pub csv: String,
    pub csv_name: String,
    /// Data lines in the CSV (header excluded).
    pub csv_rows: usize,
}

pub fn render(rep: &Report, c: &Computed) -> Rendered {
    let (csv, csv_rows) = csv_of(c);
    Rendered {
        subject: format!("[OmaFinances] {}: {}", rep.name, c.span.label),
        text: text_of(rep, c),
        html: html_of(rep, c),
        csv,
        csv_name: csv_name(&rep.name, c),
        csv_rows,
    }
}

/// The mail the report sends: subject, text and HTML parts, and the CSV as an attachment.
pub fn mail(rep: &Report, c: &Computed, to: &[String]) -> OutMail {
    let r = render(rep, c);
    OutMail {
        to: to.to_vec(),
        subject: r.subject,
        text: r.text,
        html: r.html,
        attachments: vec![Attachment { filename: r.csv_name, mime: "text/csv".to_string(), bytes: r.csv.into_bytes() }],
    }
}

/// `1,234.56`, `-1,234.56`; never `-0.00`.
pub fn money(x: f64) -> String {
    let body = format!("{:.2}", x.abs());
    let (int, frac) = body.split_once('.').unwrap_or((&body, "00"));
    let mut grouped = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let sign = if x < 0.0 && body != "0.00" { "-" } else { "" };
    format!("{sign}{grouped}.{frac}")
}

/// Plain number for CSV: two decimals, no thousands separators.
fn num(x: f64) -> String {
    let s = format!("{:.2}", x);
    if s == "-0.00" {
        "0.00".to_string()
    } else {
        s
    }
}

/// Spreadsheet formula guard for a text cell.
pub fn guard_formula(s: &str) -> String {
    match s.chars().next() {
        Some('=') | Some('+') | Some('-') | Some('@') | Some('\t') | Some('\r') => format!("'{s}"),
        _ => s.to_string(),
    }
}

/// One CSV field: formula-guarded, then RFC 4180 quoting when needed.
pub fn csv_cell(s: &str) -> String {
    let g = guard_formula(s);
    if g.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", g.replace('"', "\"\""))
    } else {
        g
    }
}

fn csv_line(cells: &[String]) -> String {
    cells.join(",") + "\r\n"
}

/// File name: `<sanitized name>-<from>-<to>.csv`, or `all` for an unbounded period.
pub fn csv_name(name: &str, c: &Computed) -> String {
    let safe: String = name
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' })
        .collect();
    let safe = if safe.trim_matches('_').is_empty() { "report".to_string() } else { safe };
    let span = match (c.span.from, c.span.to) {
        (Some(f), Some(t)) => format!("{}-{}", f.format("%Y%m%d"), t.format("%Y%m%d")),
        _ => "all".to_string(),
    };
    format!("{safe}-{span}.csv")
}

fn csv_of(c: &Computed) -> (String, usize) {
    match c.shape {
        Shape::ByCategory | Shape::ByPayee | Shape::ByMonth => {
            let mut s = csv_line(&["name", "count", "out", "in", "net", "share_pct"].map(String::from));
            for g in &c.groups {
                s += &csv_line(&[
                    csv_cell(&g.name),
                    g.count.to_string(),
                    num(g.out),
                    num(g.inn),
                    num(g.net),
                    format!("{:.1}", g.share),
                ]);
            }
            (s, c.groups.len())
        }
        Shape::Table | Shape::Summary => {
            let mut s = csv_line(&["date", "payee", "account", "category", "amount"].map(String::from));
            for t in &c.txns {
                s += &csv_line(&[csv_cell(&t.date), csv_cell(&t.payee), csv_cell(&t.account), csv_cell(&t.category), num(t.amount)]);
            }
            (s, c.txns.len())
        }
    }
}

fn currency_note(c: &Computed) -> Option<String> {
    match c.currencies.len() {
        0 => None,
        1 => Some(format!("Currency: {}", c.currencies[0])),
        _ => Some(format!("Currencies: {}. Totals are not converted between currencies.", c.currencies.join(", "))),
    }
}

const CONVENTION: &str = "Positive amounts are money out; negative amounts are money in. Net = out + in.";

fn totals_line(t: &Totals) -> String {
    format!(
        "Money out {}  |  Money in {}  |  Net {}  |  {} transaction{}",
        money(t.out),
        money(t.inn),
        money(t.net),
        t.count,
        if t.count == 1 { "" } else { "s" }
    )
}

fn text_of(rep: &Report, c: &Computed) -> String {
    let mut s = String::new();
    s += &format!("{}\n", rep.name);
    s += &format!("Period: {}\n", c.span.label);
    s += &format!("{CONVENTION}\n");
    s += &format!("{}\n", totals_line(&c.totals));
    if let Some(n) = currency_note(c) {
        s += &format!("{n}\n");
    }
    s.push('\n');
    match c.shape {
        Shape::Table => {
            s += &text_txns(&c.txns);
        }
        Shape::ByCategory | Shape::ByPayee | Shape::ByMonth => {
            s += &text_groups(&c.groups);
        }
        Shape::Summary => {
            if let Some(sm) = &c.summary {
                s += &text_summary(sm);
            }
        }
    }
    s += "\nThe full data is in the attached CSV.\n";
    s
}

fn text_txns(txns: &[TxnLine]) -> String {
    if txns.is_empty() {
        return "No transactions in this period.\n".to_string();
    }
    let mut s = String::new();
    for t in txns.iter().take(TABLE_HTML_CAP) {
        s += &format!("{}  {}  [{}]  {}  {}\n", t.date, t.payee, t.account, t.category, money(t.amount));
    }
    if txns.len() > TABLE_HTML_CAP {
        s += &format!("... {} more rows in the CSV\n", txns.len() - TABLE_HTML_CAP);
    }
    s
}

fn text_groups(groups: &[GroupLine]) -> String {
    if groups.is_empty() {
        return "No transactions in this period.\n".to_string();
    }
    let mut s = String::new();
    for g in groups {
        s += &format!(
            "{}  ({})  out {}  in {}  net {}  {:.1}%\n",
            g.name,
            g.count,
            money(g.out),
            money(g.inn),
            money(g.net),
            g.share
        );
    }
    s
}

fn text_summary(sm: &Summary) -> String {
    let mut s = String::new();
    match (&sm.prev, &sm.prev_span, sm.delta) {
        (Some(p), Some(ps), Some(d)) => {
            let pct = sm.delta_pct.map(|x| format!(" ({:+.1}%)", x)).unwrap_or_default();
            s += &format!("Previous period ({}): net {}\n", ps.label, money(p.net));
            s += &format!("Change in net: {}{}\n", money(d), pct);
        }
        _ => s += "No previous period to compare (all time).\n",
    }
    s += "\nTop categories by money out:\n";
    s += &lines_or_none(&sm.top_categories, |g| format!("  {}  out {}  {:.1}%\n", g.name, money(g.out), g.share));
    s += "\nTop payees by money out:\n";
    s += &lines_or_none(&sm.top_payees, |g| format!("  {}  out {}  {:.1}%\n", g.name, money(g.out), g.share));
    s += &format!("\nUnusual transactions ({}):\n", sm.anomalies.len());
    s += &lines_or_none(&sm.anomalies, |a| format!("  {}  {}  {}  {}\n", a.date, a.payee, money(a.amount), a.detail));
    s += &format!("\nRecurring charges due in the next 14 days ({}):\n", sm.upcoming.len());
    s += &lines_or_none(&sm.upcoming, |u| format!("  {}  {}  {}  next {}\n", u.payee, u.period, money(u.amount), u.next_expected));
    s
}

fn lines_or_none<T>(items: &[T], f: impl Fn(&T) -> String) -> String {
    if items.is_empty() {
        return "  none\n".to_string();
    }
    items.iter().map(f).collect()
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c if c.is_control() && c != '\n' => {}
            c => o.push(c),
        }
    }
    o
}

const TH: &str = "text-align:left;border-bottom:1px solid #cccccc;padding:6px;color:#333333;";
const TH_R: &str = "text-align:right;border-bottom:1px solid #cccccc;padding:6px;color:#333333;";
const TD: &str = "text-align:left;border-bottom:1px solid #eeeeee;padding:6px;";
const TD_R: &str = "text-align:right;border-bottom:1px solid #eeeeee;padding:6px;";
const H2: &str = "font-size:15px;margin:18px 0 6px 0;color:#111111;";
const TABLE: &str = "border-collapse:collapse;width:100%;font-size:13px;";

fn html_of(rep: &Report, c: &Computed) -> String {
    let mut b = String::new();
    b += "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>";
    b += &esc(&rep.name);
    b += "</title></head>";
    b += "<body style=\"margin:0;padding:16px;background:#ffffff;color:#1a1a1a;font-family:Arial,Helvetica,sans-serif;font-size:14px;\">";
    b += &format!("<h1 style=\"font-size:20px;margin:0 0 4px 0;color:#111111;\">{}</h1>", esc(&rep.name));
    b += &format!("<p style=\"margin:0 0 8px 0;color:#555555;\">{}</p>", esc(&c.span.label));
    b += &format!("<p style=\"margin:0 0 4px 0;\"><strong>{}</strong></p>", esc(&totals_line(&c.totals)));
    b += &format!("<p style=\"margin:0 0 4px 0;color:#555555;font-size:12px;\">{}</p>", CONVENTION);
    if let Some(n) = currency_note(c) {
        b += &format!("<p style=\"margin:0 0 4px 0;color:#555555;font-size:12px;\">{}</p>", esc(&n));
    }
    match c.shape {
        Shape::Table => b += &html_txns(&c.txns),
        Shape::ByCategory | Shape::ByPayee | Shape::ByMonth => b += &html_groups(&c.groups),
        Shape::Summary => {
            if let Some(sm) = &c.summary {
                b += &html_summary(sm);
            }
        }
    }
    b += "<p style=\"margin:18px 0 0 0;color:#777777;font-size:12px;\">The full data is in the attached CSV.</p>";
    b += "</body></html>";
    b
}

fn html_txns(txns: &[TxnLine]) -> String {
    if txns.is_empty() {
        return "<p>No transactions in this period.</p>".to_string();
    }
    let mut b = format!(
        "<table role=\"presentation\" style=\"{TABLE}\"><tr><th style=\"{TH}\">Date</th><th style=\"{TH}\">Payee</th>\
         <th style=\"{TH}\">Account</th><th style=\"{TH}\">Category</th><th style=\"{TH_R}\">Amount</th></tr>"
    );
    for t in txns.iter().take(TABLE_HTML_CAP) {
        b += &format!(
            "<tr><td style=\"{TD}\">{}</td><td style=\"{TD}\">{}</td><td style=\"{TD}\">{}</td><td style=\"{TD}\">{}</td><td style=\"{TD_R}\">{}</td></tr>",
            esc(&t.date),
            esc(&t.payee),
            esc(&t.account),
            esc(&t.category),
            money(t.amount)
        );
    }
    b += "</table>";
    if txns.len() > TABLE_HTML_CAP {
        b += &format!("<p style=\"color:#555555;\">Showing {TABLE_HTML_CAP} of {} rows; the CSV has all of them.</p>", txns.len());
    }
    b
}

fn html_groups(groups: &[GroupLine]) -> String {
    if groups.is_empty() {
        return "<p>No transactions in this period.</p>".to_string();
    }
    let mut b = format!(
        "<table role=\"presentation\" style=\"{TABLE}\"><tr><th style=\"{TH}\">Name</th><th style=\"{TH_R}\">Count</th>\
         <th style=\"{TH_R}\">Out</th><th style=\"{TH_R}\">In</th><th style=\"{TH_R}\">Net</th><th style=\"{TH_R}\">Share</th></tr>"
    );
    for g in groups {
        b += &format!(
            "<tr><td style=\"{TD}\">{}</td><td style=\"{TD_R}\">{}</td><td style=\"{TD_R}\">{}</td><td style=\"{TD_R}\">{}</td><td style=\"{TD_R}\">{}</td><td style=\"{TD_R}\">{:.1}%</td></tr>",
            esc(&g.name),
            g.count,
            money(g.out),
            money(g.inn),
            money(g.net),
            g.share
        );
    }
    b + "</table>"
}

fn html_summary(sm: &Summary) -> String {
    let mut b = String::new();
    match (&sm.prev, &sm.prev_span, sm.delta) {
        (Some(p), Some(ps), Some(d)) => {
            let pct = sm.delta_pct.map(|x| format!(" ({:+.1}%)", x)).unwrap_or_default();
            b += &format!("<h2 style=\"{H2}\">Compared with the previous period</h2>");
            b += &format!(
                "<p style=\"margin:0 0 4px 0;\">Previous period ({}): net {}</p>",
                esc(&ps.label),
                money(p.net)
            );
            b += &format!("<p style=\"margin:0 0 4px 0;\">Change in net: {}{}</p>", money(d), esc(&pct));
        }
        _ => b += "<p>No previous period to compare (all time).</p>",
    }
    b += &format!("<h2 style=\"{H2}\">Top categories by money out</h2>");
    b += &html_groups_small(&sm.top_categories);
    b += &format!("<h2 style=\"{H2}\">Top payees by money out</h2>");
    b += &html_groups_small(&sm.top_payees);
    b += &format!("<h2 style=\"{H2}\">Unusual transactions ({})</h2>", sm.anomalies.len());
    if sm.anomalies.is_empty() {
        b += "<p>None.</p>";
    } else {
        b += &format!("<table role=\"presentation\" style=\"{TABLE}\">");
        for a in &sm.anomalies {
            b += &format!(
                "<tr><td style=\"{TD}\">{}</td><td style=\"{TD}\">{}</td><td style=\"{TD_R}\">{}</td><td style=\"{TD}\">{}</td></tr>",
                esc(&a.date),
                esc(&a.payee),
                money(a.amount),
                esc(&a.detail)
            );
        }
        b += "</table>";
    }
    b += &format!("<h2 style=\"{H2}\">Recurring charges due in the next 14 days ({})</h2>", sm.upcoming.len());
    if sm.upcoming.is_empty() {
        b += "<p>None.</p>";
    } else {
        b += &format!("<table role=\"presentation\" style=\"{TABLE}\">");
        for u in &sm.upcoming {
            b += &format!(
                "<tr><td style=\"{TD}\">{}</td><td style=\"{TD}\">{}</td><td style=\"{TD_R}\">{}</td><td style=\"{TD}\">next {}</td></tr>",
                esc(&u.payee),
                u.period,
                money(u.amount),
                esc(&u.next_expected)
            );
        }
        b += "</table>";
    }
    b
}

fn html_groups_small(groups: &[GroupLine]) -> String {
    if groups.is_empty() {
        return "<p>None.</p>".to_string();
    }
    let mut b = format!("<table role=\"presentation\" style=\"{TABLE}\">");
    for g in groups {
        b += &format!(
            "<tr><td style=\"{TD}\">{}</td><td style=\"{TD_R}\">{}</td><td style=\"{TD_R}\">{:.1}%</td></tr>",
            esc(&g.name),
            money(g.out),
            g.share
        );
    }
    b + "</table>"
}

/// Used by the preview endpoint and tests.
pub fn preview_json(r: &Rendered) -> serde_json::Value {
    serde_json::json!({
        "subject": r.subject,
        "text": r.text,
        "html": r.html,
        "csv_name": r.csv_name,
        "csv_rows": r.csv_rows,
    })
}
