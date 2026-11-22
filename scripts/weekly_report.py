#!/usr/bin/env python3
"""OmaFinances weekly finances report: offline builder and Gmail sender.

Spec: docs/EMAIL_REPORT.md. Everything in the report is computed from the local
SQLCipher database by plain Python; no AI or network call is involved in building it.
The network is used only by --send (Gmail API), and only to the allowlisted recipient.

Usage (run with the project venv, which has sqlcipher3):
  .venv/bin/python scripts/weekly_report.py                    # print plain text
  .venv/bin/python scripts/weekly_report.py --html out.html    # write the HTML body
  .venv/bin/python scripts/weekly_report.py --draft docs/WEEKLY_REPORT_DRAFT.md
  .venv/bin/python scripts/weekly_report.py --send            # send the email
  --as-of YYYY-MM-DD  report as of a date (default: today, local time)

Secrets: DB passphrase from the Secret Service keyring (service omafinances, user
db-passphrase) with fallback to DB_PASSPHRASE in .env. Gmail refresh token from the
keyring (service omafinances, user gmail-refresh-token) with a test-only override
OMAFINANCES_GMAIL_REFRESH_TOKEN. Nothing secret is printed or logged.
"""
from __future__ import annotations

import argparse
import base64
import datetime as dt
import html
import json
import os
import re
import subprocess
import sys
import tomllib
import unicodedata
from collections import Counter, defaultdict
from decimal import ROUND_HALF_UP, Decimal
from email.message import EmailMessage
from pathlib import Path

PROJECT = Path(__file__).resolve().parent.parent
DEFAULT_DB = PROJECT / "finances.db"
SETTINGS = Path.home() / ".config/omafinances/settings.toml"

# ---------------------------------------------------------------- spec constants
SENDER = "adam.macielinski@gmail.com"
RECIPIENT = "adam@macielinski.net"
SUBJECT = "OmaFinances - Weekly Finances Report"
FOCUS_DAYS = 90
FONT_STACK = ("'JetBrainsMono Nerd Font','JetBrainsMono NF','Nerd Font Mono',"
              "'CaskaydiaMono Nerd Font',Menlo,Consolas,'DejaVu Sans Mono',monospace")

# Display configuration (edit here). Keyed by the account's last four digits (mask).
BANK_NAMES = {"Citibank Online": "Citibank", "TD Bank": "TD Bank", "Capital One": "Capital One"}
BANK_ORDER = ["TD Bank", "Citibank", "Capital One"]
CREDIT_TYPE_ORDER = ["Credit card", "Line of credit"]
CASH_TYPE_ORDER = ["Checking", "Savings"]

# Food: LM categories by name, plus a vendor-name fallback used only for uncategorized rows.
FOOD_CATEGORIES = {"Groceries", "Restaurants", "Food and Drinks"}
FOOD_VENDOR_RE = re.compile(
    r"restaurant|kitchen|grill|\bbbq\b|bistro|\bcaf[eé]\b|coffee|dunkin|starbucks|mcdonald|"
    r"chick-?fil-?a|chipotle|panera|mooyah|melting pot|biryani|thai taste|curry|brewing|pizza|"
    r"sushi|taco|burger|wendy|subway|domino|popeyes|greco|cheddar up|publix|harris teeter|"
    r"whole foods|trader joe|aldi|food lion|kroger|bakery|\bdeli\b|diner|\btea\b|wings|"
    r"ice cream|donut|doughnut|smoothie|boba|super g mart",
    re.IGNORECASE,
)
FOOD_VENDOR_EXCLUDE_RE = re.compile(r"bbq guru", re.IGNORECASE)
AMAZON_RE = re.compile(r"amazon|amzn", re.IGNORECASE)


# ---------------------------------------------------------------- text helpers
def sanitize(text) -> str:
    """Make text safe for a monospace email: ASCII-ish only, so every glyph is one cell wide."""
    if text is None:
        return ""
    out = []
    for ch in unicodedata.normalize("NFC", str(text)):
        if ch in "\u2018\u2019":
            out.append("'")
        elif ch in "\u201c\u201d":
            out.append('"')
        elif ch in "\u2013\u2014":
            out.append("-")
        elif ch in "\u00ae\u2122\u00a9":
            continue  # registered, trademark, copyright marks (card product names)
        elif ord(ch) < 0x250 and unicodedata.category(ch)[0] != "C":
            out.append(ch)
        elif ch.isspace():
            out.append(" ")
        elif unicodedata.category(ch)[0] in ("S", "M", "C"):
            continue  # emoji, symbols, variation selectors, combining marks: dropped
        else:
            out.append("?")  # other letters we cannot draw in a fixed grid
    return re.sub(r"\s+", " ", "".join(out)).strip()


def money_int(v) -> int:
    """Round to the nearest whole dollar (half away from zero)."""
    return int(Decimal(str(v)).quantize(Decimal("1"), rounding=ROUND_HALF_UP))


def money_cells(values):
    """Format one column of money values per the spec.

    Numbers are right-justified; the widest number gets "$ " directly to its left and every
    other number is padded so its "$" lands in the same column. None shows "-" (no value in
    a balance table); "" stays blank (a row with no amount). Returns (cells, width).
    """
    nums = [v if v in ("", None) else money_int(v) for v in values]
    texts = [f"{n:,}" for n in nums if n not in ("", None)]
    m = max((len(t) for t in texts), default=1)
    width = m + 2
    cells = []
    for n in nums:
        if n == "":
            cells.append(" " * width)
        elif n is None:
            cells.append("-".rjust(width))
        else:
            cells.append(("$ " + f"{n:,}".rjust(m)).rjust(width))
    return cells, width


def fit(text: str, width: int) -> str:
    return text if len(text) <= width else text[: max(width - 3, 0)] + "..."


# ---------------------------------------------------------------- table renderer
class Column:
    def __init__(self, title, align="l", kind="text", sub=None, max_w=None):
        self.title, self.align, self.kind, self.sub, self.max_w = title, align, kind, sub, max_w


class Table:
    """Monospace table. Each rendered line is (text, style).

    Styles: header, dim, rule, group, total, data, blank. A row is either a list of cells
    (one per column) or a plain label (group/dim line, not tied to a column).
    """

    def __init__(self, columns, gap=2):
        self.cols, self.gap, self.rows = columns, gap, []

    def add(self, cells, style="data", indent=0):
        self.rows.append((cells, style, indent))

    def group(self, label, style="group", indent=0):
        self.rows.append((label, style, indent))

    def blank(self):
        self.rows.append((None, "blank", 0))

    def render(self):
        table_rows = [r for r in self.rows if isinstance(r[0], list)]
        widths, money_fmt = [], {}
        for i, col in enumerate(self.cols):
            if col.kind == "money":
                cells, w = money_cells([r[0][i] for r in table_rows])
                money_fmt[i] = {id(r[0]): c for r, c in zip(table_rows, cells)}
                w = max(w, len(col.title), len(col.sub or ""))
            else:
                texts = [((" " * r[2] + (r[0][i] or "")) if i == 0 else (r[0][i] or "")) for r in table_rows]
                w = max([len(col.title)] + [len(t) for t in texts])
                if col.max_w:
                    w = min(w, col.max_w)
            widths.append(w)

        def join(parts):
            return (" " * self.gap).join(parts).rstrip()

        def align(text, i):
            col = self.cols[i]
            return text.rjust(widths[i]) if col.align == "r" else text.ljust(widths[i])

        lines = [(join([align(c.title, i) for i, c in enumerate(self.cols)]), "header")]
        if any(c.sub for c in self.cols):
            lines.append((join([align(c.sub or "", i) for i, c in enumerate(self.cols)]), "dim"))
        lines.append((join(["-" * w for w in widths]), "rule"))

        for cells, style, indent in self.rows:
            if style == "blank":
                lines.append(("", "blank"))
            elif not isinstance(cells, list):
                lines.append((" " * indent + cells, style))
            else:
                parts = []
                for i, col in enumerate(self.cols):
                    if col.kind == "money":
                        cell = money_fmt[i][id(cells)]
                    else:
                        text = cells[i] or ""
                        if i == 0 and indent:
                            text = " " * indent + text
                        cell = fit(text, widths[i])
                    parts.append(align(cell, i))
                lines.append((join(parts), style))
        return lines


def section(title: str, subtitle: str | None, table_lines):
    """A titled block: the title underlined with hyphens on the next line."""
    out = [(title, "title"), ("-" * len(title), "rule")]
    if subtitle:
        out.append((subtitle, "dim"))
    out.append(("", "blank"))
    out.extend(table_lines)
    return out


# ---------------------------------------------------------------- data access
def open_db(path: Path):
    from sqlcipher3 import dbapi2 as sqlite

    passphrase = keyring_lookup("db-passphrase") or dotenv_value("DB_PASSPHRASE")
    if not passphrase:
        raise SystemExit("no database passphrase found (keyring db-passphrase or .env DB_PASSPHRASE)")
    conn = sqlite.connect(str(path))
    # PRAGMA does not take bound parameters; quote the passphrase as a SQL string literal.
    conn.execute("PRAGMA key = '" + passphrase.replace("'", "''") + "'")
    conn.execute("PRAGMA busy_timeout = 5000")
    conn.execute("SELECT count(*) FROM sqlite_master").fetchone()  # fails fast on a wrong key
    return conn


def keyring_lookup(username: str) -> str | None:
    try:
        out = subprocess.run(
            ["secret-tool", "lookup", "service", "omafinances", "username", username],
            capture_output=True, text=True, timeout=15,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    value = out.stdout.strip("\n")
    return value or None


def dotenv_value(key: str) -> str | None:
    env = PROJECT / ".env"
    if not env.exists():
        return None
    for line in env.read_text().splitlines():
        if line.startswith(key + "="):
            return line.split("=", 1)[1].strip().strip('"').strip("'")
    return None


def load_settings() -> dict:
    if SETTINGS.exists():
        with SETTINGS.open("rb") as f:
            return tomllib.load(f)
    return {}


def table_columns(conn, table) -> set:
    return {r[1] for r in conn.execute(f"PRAGMA table_info({table})")}


# ---------------------------------------------------------------- report model
def month_start(d: dt.date) -> dt.date:
    return d.replace(day=1)


def prev_month(d: dt.date) -> dt.date:
    return (d.replace(day=1) - dt.timedelta(days=1)).replace(day=1)


def account_sections(conn, as_of: dt.date):
    # accounts.nickname arrived with schema v5; an older DB simply has none.
    nick_sql = "nickname" if "nickname" in table_columns(conn, "accounts") else "NULL"
    accounts = conn.execute(
        f"SELECT id, type, subtype, institution_name, mask, name, display_name, is_closed, {nick_sql} FROM accounts"
    ).fetchall()
    credit, cash, skipped = [], [], 0
    months = [month_start(as_of), prev_month(month_start(as_of))]
    months.append(prev_month(months[-1]))

    def latest(acct_id, month_first):
        month_last = (month_first.replace(day=28) + dt.timedelta(days=4)).replace(day=1) - dt.timedelta(days=1)
        upper = min(month_last, as_of)
        row = conn.execute(
            "SELECT balance, balance_date FROM account_balances WHERE account_id=? AND balance_date>=? "
            "AND balance_date<=? ORDER BY balance_date DESC LIMIT 1",
            (acct_id, month_first.isoformat(), upper.isoformat()),
        ).fetchone()
        return (row[0], row[1]) if row else (None, None)

    for acct_id, typ, sub, inst, mask, name, disp, closed, nickname in accounts:
        mask = (mask or "").strip()
        subtext = (sub or "").lower()
        if typ == "credit":
            kind = "Credit card"
        elif typ == "loan" and ("line of credit" in subtext or "home equity" in subtext):
            kind = "Line of credit"
        elif typ == "cash" and subtext in ("checking", "savings"):
            kind = "Checking" if subtext == "checking" else "Savings"
        else:
            skipped += 1
            continue
        if closed:
            skipped += 1
            continue
        bank = BANK_NAMES.get((inst or "").strip(), (inst or "Other").strip())
        # Nickname if the user set one, else the card's formal name as the bank reports it.
        nick = (nickname or "").strip() or (name or "").strip() or (disp or "").strip() or "Account"
        balances = [latest(acct_id, m) for m in months]
        row = {"bank": bank, "kind": kind, "nick": sanitize(nick), "bal": balances}
        (credit if kind in CREDIT_TYPE_ORDER else cash).append(row)

    def key(order):
        def k(r):
            b = BANK_ORDER.index(r["bank"]) if r["bank"] in BANK_ORDER else len(BANK_ORDER)
            t = order.index(r["kind"]) if r["kind"] in order else len(order)
            return (b, t, r["nick"].lower())
        return k

    credit.sort(key=key(CREDIT_TYPE_ORDER))
    cash.sort(key=key(CASH_TYPE_ORDER))
    return credit, cash, months, skipped


def balance_table(rows, months, as_of: dt.date):
    """Bank | Account | M+0 | M-1 | M-2. Rows are ordered bank, then account type, then name; the bank
    is printed once per bank and a blank line separates banks and account types."""
    cols = [Column("Bank", "l", max_w=16), Column("Account", "l", max_w=44)]
    for i, m in enumerate(months):
        label = ["M+0", "M-1", "M-2"][i]
        cols.append(Column(label, "r", "money"))
    table = Table(cols, gap=3)
    # as-of dates: the latest snapshot date seen in each month across all accounts
    asof = []
    for i in range(len(months)):
        dates = [r["bal"][i][1] for r in rows if r["bal"][i][1]]
        asof.append(max(dates) if dates else None)
    for i, col in enumerate(cols[2:]):
        d = asof[i]
        col.sub = ("as of " + dt.date.fromisoformat(d).strftime("%m/%d")) if d else "no data"
    last_bank = last_kind = None
    for r in rows:
        new_bank = r["bank"] != last_bank
        if last_bank is not None and (new_bank or r["kind"] != last_kind):
            table.blank()
        table.add([r["bank"] if new_bank else "", r["nick"]] + [b[0] for b in r["bal"]])
        last_bank, last_kind = r["bank"], r["kind"]
    if not rows:
        table.group("No accounts in this group.", "dim")
    return table


def classify_txn_rows(conn, start: dt.date, end: dt.date):
    cols = table_columns(conn, "transactions")
    notes_sql = "t.user_notes" if "user_notes" in cols else "NULL"
    rows = conn.execute(
        f"""SELECT t.id, t.date, t.payee, t.original_name, t.amount, c.name, {notes_sql}, t.category_id
            FROM transactions t LEFT JOIN categories c ON c.id = t.category_id
            WHERE t.date >= ? AND t.date <= ? AND COALESCE(t.is_split_parent,0)=0
              AND COALESCE(t.is_group_parent,0)=0""",
        (start.isoformat(), end.isoformat()),
    ).fetchall()
    out = []
    for tid, d, payee, orig, amount, cat, notes, cat_id in rows:
        vendor = sanitize(payee or orig or "(unknown)")
        out.append({
            "id": tid, "date": d, "vendor": vendor,
            "amount": float(amount), "cat": sanitize(cat or ""), "cat_id": cat_id,
            "notes": sanitize(notes or ""),
            "orig": (payee or "") + " " + (orig or ""),
        })
    return out


def amazon_rows(txns):
    return [t for t in txns if AMAZON_RE.search(t["orig"]) or AMAZON_RE.search(t["vendor"])]


def food_rows(txns):
    out = []
    for t in txns:
        if t["cat"] in FOOD_CATEGORIES:
            t["food_basis"] = t["cat"]
            out.append(t)
        elif not t["cat"] and FOOD_VENDOR_RE.search(t["vendor"]) and not FOOD_VENDOR_EXCLUDE_RE.search(t["vendor"]):
            t["food_basis"] = "food (by vendor)"
            out.append(t)
    return out


def amazon_section(rows, start, end):
    rows = sorted(rows, key=lambda t: (-t["amount"], t["date"]), reverse=False)
    for t in rows:
        t["cat_label"] = t["cat"] or "(none)"
    total = sum(t["amount"] for t in rows)
    table = Table([
        Column("Date", "l"), Column("Amount", "r", "money"), Column("Vendor", "l", max_w=36),
        Column("Category", "l", max_w=24), Column("Notes", "l", max_w=60),
    ])
    for t in rows:
        table.add([t["date"], t["amount"], t["vendor"], t["cat_label"], t["notes"]])
    if rows:
        table.blank()
        table.add(["", total, f"Total ({len(rows)})", "", ""], style="total")
    else:
        table.group("No Amazon transactions in this period.", "dim")
    return section("Amazon", f"{start:%b %d, %Y} to {end:%b %d, %Y}, largest first", table.render())


def food_section(rows, start, end):
    for t in rows:
        t["cat_label"] = t["food_basis"]
    by_vendor = defaultdict(list)
    names = defaultdict(Counter)
    for t in rows:
        key = t["vendor"].lower()
        by_vendor[key].append(t)
        names[key][t["vendor"]] += 1
    ordered = sorted(by_vendor.items(), key=lambda kv: -sum(t["amount"] for t in kv[1]))
    table = Table([
        Column("Date", "l"), Column("Amount", "r", "money"), Column("Vendor", "l", max_w=36),
        Column("Category", "l", max_w=24), Column("Notes", "l", max_w=60),
    ])
    grand = 0.0
    for key, items in ordered:
        name = names[key].most_common(1)[0][0]
        total = sum(t["amount"] for t in items)
        grand += total
        items.sort(key=lambda t: t["date"])
        table.add(["", total, f"{name} ({len(items)})", "", ""], style="group")
        for t in items:
            table.add([t["date"], t["amount"], "", t["cat_label"], t["notes"]], indent=0)
    if ordered:
        table.blank()
        table.add(["", grand, f"Total ({len(rows)})", "", ""], style="total")
    else:
        table.group("No food or dining transactions in this period.", "dim")
    return section("Food, eating out and casual dining", f"{start:%b %d, %Y} to {end:%b %d, %Y}, by vendor then date",
                   table.render())


def build_report(conn, as_of: dt.date):
    # "Prior 90 days": (reporting date - 90, reporting date], i.e. 90 calendar days ending on the report date.
    start = as_of - dt.timedelta(days=FOCUS_DAYS) + dt.timedelta(days=1)
    credit, cash, months, skipped = account_sections(conn, as_of)
    txns = classify_txn_rows(conn, start, as_of)
    amazon = amazon_rows(txns)
    food = food_rows(txns)
    latest_sync = None
    try:
        row = conn.execute("SELECT max(balance_date) FROM account_balances").fetchone()
        latest_sync = row[0]
    except Exception:
        pass

    header = [
        ("OMAFINANCES", "title"),
        ("Weekly Finances Report", "title2"),
        (f"Reporting date: {as_of:%A, %B %d, %Y}", "dim"),
        ("", "blank"),
    ]
    balances_intro = "Balances: latest snapshot in each month, as synced from the bank."
    credit_block = section("Credit accounts", balances_intro, balance_table(credit, months, as_of).render())
    cash_block = section("Checking and savings", None, balance_table(cash, months, as_of).render())
    focus_block = (
        [("Transactions of focus", "title"), ("-" * len("Transactions of focus"), "rule"),
         (f"Prior {FOCUS_DAYS} days: {start:%b %d, %Y} to {as_of:%b %d, %Y}", "dim"), ("", "blank")]
    )
    focus_block.extend(amazon_section(amazon, start, as_of))
    focus_block.append(("", "blank"))
    focus_block.extend(food_section(food, start, as_of))
    footer = [
        ("", "blank"),
        ("Amounts: positive = money out; negative = money in. Values rounded to the nearest dollar.", "dim"),
        (f"Balance snapshots through {latest_sync or 'n/a'}. Generated locally from the encrypted ledger.", "dim"),
    ]
    if skipped:
        footer.append((f"{skipped} account(s) not shown (closed or outside this report).", "dim"))
    return {
        "as_of": as_of, "start": start,
        "lines": header + credit_block + [("", "blank")] + cash_block + [("", "blank")] + focus_block + footer,
        "counts": {"credit": len(credit), "cash": len(cash), "amazon": len(amazon), "food": len(food),
                   "txns_window": len(txns)},
    }


# ---------------------------------------------------------------- renderers
def render_text(report) -> str:
    return "\n".join(t for t, _ in report["lines"]) + "\n"


def render_html(report) -> str:
    cls = {
        "title": "t-title", "title2": "t-title2", "header": "t-head", "rule": "t-rule", "group": "t-group",
        "total": "t-total", "dim": "t-dim", "data": "", "blank": "",
    }
    lines = report["lines"]
    banner, body = lines[:4], lines[4:]

    def pre(block):
        spans = []
        for text, style in block:
            c = cls.get(style, "")
            spans.append(f'<span class="{c}">{html.escape(text)}</span>' if c else html.escape(text))
        return "<pre>" + "\n".join(spans) + "</pre>"

    # One panel per section: a new panel starts at every section title line.
    panels, current = [], []
    for item in body:
        if item[1] == "title" and current:
            panels.append(current)
            current = []
        current.append(item)
    if current:
        panels.append(current)

    banner_html = (
        '<div class="banner">'
        '<pre><span class="b-kicker">' + html.escape(banner[0][0]) + '</span>\n'
        '<span class="b-title">' + html.escape(banner[1][0]) + '</span>\n'
        '<span class="b-date">' + html.escape(banner[2][0]) + '</span></pre></div>'
    )
    panel_html = "".join(f'<div class="card">{pre(p)}</div>' for p in panels)
    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{html.escape(SUBJECT)}</title>
<style>
  body {{ margin:0; padding:0; background:#eef1f5; font-family:{FONT_STACK}; color:#1d2330; }}
  .wrap {{ max-width:900px; margin:0 auto; padding:24px 16px; }}
  .banner {{ background:#0b3d75; color:#ffffff; border-radius:10px; padding:18px 22px; margin-bottom:16px; }}
  .banner pre {{ color:#ffffff; }}
  .b-kicker {{ color:#9fc3ec; font-weight:700; letter-spacing:2px; }}
  .b-title {{ color:#ffffff; font-weight:700; font-size:20px; }}
  .b-date {{ color:#cfe0f5; }}
  .card {{ background:#ffffff; border:1px solid #d7dde6; border-radius:10px; padding:18px 22px; margin-bottom:16px; }}
  pre {{ margin:0; font-family:{FONT_STACK}; font-size:13px; line-height:1.55; white-space:pre; overflow-x:auto; color:#1d2330; }}
  .t-title {{ color:#0b3d75; font-weight:700; font-size:15px; }}
  .t-head {{ color:#1d2330; font-weight:700; }}
  .t-rule {{ color:#8a94a6; }}
  .t-group {{ color:#0b5cad; font-weight:700; }}
  .t-total {{ color:#1d2330; font-weight:700; }}
  .t-dim {{ color:#6b7487; }}
  .foot {{ color:#6b7487; font-size:12px; text-align:center; margin-top:8px; }}
</style>
</head>
<body>
<div class="wrap">
{banner_html}
{panel_html}
<div class="foot">OmaFinances &middot; generated locally</div>
</div>
</body>
</html>
"""


def render_markdown(report) -> str:
    text = render_text(report)
    stamp = dt.datetime.now().strftime("%Y-%m-%d %H:%M")
    return (
        "# OmaFinances - Weekly Finances Report (draft)\n\n"
        f"Generated by `scripts/weekly_report.py --draft` on {stamp} from the local database, as of "
        f"{report['as_of']:%Y-%m-%d}. Contains real balances and transactions; keep this file out of "
        "public repositories.\n\n"
        "Spec: [EMAIL_REPORT.md](EMAIL_REPORT.md). Rendered email is monospace; this draft shows the same text layout.\n\n"
        "```text\n" + text + "```\n"
    )


# ---------------------------------------------------------------- Gmail
def gmail_bases():
    base = os.environ.get("OMAFINANCES_GMAIL_API_BASE", "https://gmail.googleapis.com/gmail/v1/users/me")
    token_url = os.environ.get("OMAFINANCES_OAUTH_TOKEN_URL", "https://oauth2.googleapis.com/token")
    userinfo_url = os.environ.get("OMAFINANCES_USERINFO_URL", "https://openidconnect.googleapis.com/v1/userinfo")
    return base.rstrip("/"), token_url, userinfo_url


def load_client(settings) -> tuple[str, str]:
    raw = (settings.get("mail", {}).get("client_secret_file") or "").strip()
    if not raw:
        raise SystemExit("[mail] client_secret_file is not set in settings.toml")
    data = json.loads(Path(os.path.expanduser(raw)).read_text())
    client = data.get("installed") or data.get("web") or {}
    if not client.get("client_id") or not client.get("client_secret"):
        raise SystemExit("client secret file has no installed/web client_id and client_secret")
    return client["client_id"], client["client_secret"]


def refresh_token() -> str:
    token = os.environ.get("OMAFINANCES_GMAIL_REFRESH_TOKEN") or keyring_lookup("gmail-refresh-token")
    if not token:
        raise SystemExit("no Gmail token: run `omafinances gmail-auth` first")
    return token


def access_token(settings) -> str:
    import requests

    client_id, client_secret = load_client(settings)
    _, token_url, _ = gmail_bases()
    r = requests.post(token_url, data={
        "client_id": client_id, "client_secret": client_secret,
        "refresh_token": refresh_token(), "grant_type": "refresh_token",
    }, timeout=30)
    if r.status_code != 200:
        raise SystemExit(f"token refresh failed (HTTP {r.status_code}); re-run `omafinances gmail-auth`")
    return r.json()["access_token"]


def build_message(subject: str, to: str, text: str, html_body: str) -> EmailMessage:
    for field in (subject, to):
        if "\n" in field or "\r" in field:
            raise SystemExit("header injection refused")
    msg = EmailMessage()
    msg["From"] = SENDER
    msg["To"] = to
    msg["Subject"] = subject
    msg.set_content(text)
    msg.add_alternative(html_body, subtype="html")
    return msg


def send_report(report, settings, to: str = RECIPIENT) -> str:
    import requests

    if to != RECIPIENT:
        raise SystemExit(f"refusing to send to {to}: the only allowed recipient is {RECIPIENT}")
    base, _, userinfo_url = gmail_bases()
    token = access_token(settings)
    headers = {"Authorization": f"Bearer {token}"}
    who = requests.get(userinfo_url, headers=headers, timeout=30)
    email = (who.json().get("email", "") if who.status_code == 200 else "").lower()
    if email != SENDER.lower():
        raise SystemExit(f"authenticated Gmail account is not the sender {SENDER}; refusing to send")
    msg = build_message(SUBJECT, to, render_text(report), render_html(report))
    raw = base64.urlsafe_b64encode(msg.as_bytes()).decode()
    r = requests.post(f"{base}/messages/send", headers=headers, json={"raw": raw}, timeout=60)
    if r.status_code != 200:
        raise SystemExit(f"Gmail send failed (HTTP {r.status_code})")
    return r.json().get("id", "")


# ---------------------------------------------------------------- main
def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--as-of", type=dt.date.fromisoformat, default=dt.date.today())
    ap.add_argument("--db", type=Path, default=None)
    ap.add_argument("--html", type=Path, help="write the HTML body to this path")
    ap.add_argument("--draft", type=Path, help="write the Markdown draft to this path")
    ap.add_argument("--send", action="store_true", help="send the email to the allowed recipient")
    args = ap.parse_args(argv)

    settings = load_settings()
    db_path = args.db or Path(settings.get("database", {}).get("path") or DEFAULT_DB)
    conn = open_db(db_path)
    try:
        report = build_report(conn, args.as_of)
    finally:
        conn.close()

    wrote = []
    if args.html:
        args.html.write_text(render_html(report))
        wrote.append(str(args.html))
    if args.draft:
        args.draft.write_text(render_markdown(report))
        wrote.append(str(args.draft))
    if args.send:
        msg_id = send_report(report, settings)
        wrote.append(f"sent to {RECIPIENT} (Gmail id {msg_id})")
    if not wrote:
        sys.stdout.write(render_text(report))
    else:
        print("\n".join(wrote))
    print(f"counts: {report['counts']}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
