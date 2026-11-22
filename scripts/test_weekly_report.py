"""Offline tests for scripts/weekly_report.py. Run: .venv/bin/python -m unittest scripts/test_weekly_report.py

Uses a synthetic plaintext SQLite database and a local fake Gmail server. Never touches finances.db.
"""
import base64
import datetime as dt
import email
import json
import os
import re
import sqlite3
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import weekly_report as wr  # noqa: E402


TEST_TO = "adam@test.example"  # stands in for [reports] recipient; the real one lives in settings.toml

class MoneyFormatting(unittest.TestCase):
    def test_rounds_half_away_from_zero_to_whole_dollars(self):
        self.assertEqual(wr.money_int(2.5), 3)
        self.assertEqual(wr.money_int(-2.5), -3)
        self.assertEqual(wr.money_int(1234.4), 1234)
        self.assertEqual(wr.money_int(0.49), 0)

    def test_biggest_number_gets_dollar_and_space_and_others_align_the_dollar(self):
        cells, width = wr.money_cells([14500.0, 10250.0, 0.0, None])
        self.assertEqual(cells[0], "$ 14,500")
        self.assertEqual(cells[1].strip(), "$ 10,250")
        self.assertEqual(len({len(c) for c in cells}), 1, "all cells share one width")
        dollar_cols = {c.index("$") for c in cells if "$" in c}
        self.assertEqual(len(dollar_cols), 1, "every dollar sign sits in the same column")
        self.assertTrue(cells[3].strip() == "-")
        self.assertEqual(cells[2].strip(), "$      0")

    def test_smaller_numbers_are_right_justified(self):
        cells, _ = wr.money_cells([1000.0, 5.0])
        self.assertEqual(cells[0].rstrip(), "$ 1,000")
        self.assertEqual(cells[1].rstrip(), "$     5")

    def test_negative_biggest_number(self):
        cells, _ = wr.money_cells([-1500.0, 20.0])
        self.assertEqual(cells[0].strip(), "$ -1,500")
        self.assertEqual(cells[0].index("$"), cells[1].index("$"))


class Text(unittest.TestCase):
    def test_sanitize_drops_emoji_and_keeps_names_one_cell_wide(self):
        self.assertEqual(wr.sanitize("🛒 Groceries"), "Groceries")
        self.assertEqual(wr.sanitize("🍽️ Restaurants"), "Restaurants")
        self.assertEqual(wr.sanitize("Citi®/AAdvantage®"), "Citi/AAdvantage")  # (R)/TM marks dropped
        self.assertEqual(wr.sanitize("Dunkin\u2019"), "Dunkin'")
        self.assertEqual(wr.sanitize("\u4e2d\u6587"), "??")

    def test_fit_truncates_with_ascii_ellipsis(self):
        self.assertEqual(wr.fit("abcdefghij", 6), "abc...")
        self.assertEqual(wr.fit("abc", 6), "abc")


class Table(unittest.TestCase):
    def test_header_underline_matches_each_column_width(self):
        t = wr.Table([wr.Column("Account", "l"), wr.Column("M+0", "r", "money", sub="as of 11/15")], gap=3)
        t.add(["Citi Gold", 0.0])
        lines = [text for text, _ in t.render()]
        header, sub, rule = lines[0], lines[1], lines[2]
        # first column is as wide as its widest cell; the underline matches it
        self.assertEqual(len(rule.split()[0]), len("Citi Gold"))
        self.assertEqual(len(rule.split()[1]), len("as of 11/15"))
        self.assertTrue(set(rule.replace(" ", "")) <= {"-"})
        self.assertIn("M+0", header)
        self.assertIn("as of 11/15", sub)


def make_db():
    """Plaintext synthetic ledger with the columns build_report reads."""
    conn = sqlite3.connect(":memory:")
    conn.executescript("""
    CREATE TABLE accounts (id INTEGER PRIMARY KEY, type TEXT, subtype TEXT, institution_name TEXT,
        mask TEXT, name TEXT, display_name TEXT, is_closed INTEGER DEFAULT 0, nickname TEXT);
    CREATE TABLE account_balances (id INTEGER PRIMARY KEY, account_id INTEGER, balance_date TEXT, balance REAL,
        UNIQUE(account_id, balance_date));
    CREATE TABLE categories (id INTEGER PRIMARY KEY, name TEXT);
    CREATE TABLE transactions (id INTEGER PRIMARY KEY, date TEXT, payee TEXT, original_name TEXT, amount REAL,
        category_id INTEGER, is_split_parent INTEGER DEFAULT 0, is_group_parent INTEGER DEFAULT 0,
        user_notes TEXT);
    INSERT INTO categories VALUES (1,'🛒 Groceries'),(2,'🍽️ Restaurants'),(3,'💵 Income');
    INSERT INTO accounts VALUES
      (10,'credit','credit card','Citibank Online','1234','Citi\u00ae Platinum Select\u00ae Mastercard\u00ae','Citibank Online Citi Platinum',0,'Platinum'),
      (11,'loan','line of credit','TD Bank','2345','CREDITLINE','TD Bank CREDITLINE',0,NULL),
      (12,'cash','checking','TD Bank','3456','TD BEYOND','TD Bank TD BEYOND',0,'Everyday'),
      (13,'investment','brokerage','Fidelity','9999','Brokerage','Brokerage',0,NULL),
      (14,'credit','credit card','Citibank Online','1111','Old','Old card',1,NULL),
      (15,'credit','credit card','Citibank Online','2222','Citi\u00ae Gold\u2122 Mastercard\u00ae','Citibank Online Citi Gold',0,'  ');
    INSERT INTO account_balances (account_id, balance_date, balance) VALUES
      (10,'2026-09-20',900),(10,'2026-09-27',1000),(10,'2026-10-25',2000),(10,'2026-11-01',3000),(10,'2026-11-15',3500),
      (11,'2026-11-08',17000),
      (12,'2026-11-15',10250.50);
    """)
    rows = [
        (1, "2026-11-13", "AMAZON MKTPLACE", None, 433.4, None, 0, 0, "gift"),
        (2, "2026-11-10", "Amazon Prime Video", None, 15, None, 0, 0, None),
        (3, "2026-11-01", "Harris Teeter", None, 88.0, 1, 0, 0, None),
        (4, "2026-10-01", "Lam's Kitchen", None, 24.0, None, 0, 0, None),   # vendor-name food, uncategorized
        (5, "2026-10-05", "Acme Payroll", None, -2000.0, 3, 0, 0, None),
        (6, "2026-08-24", "Amazon", None, 50.0, None, 0, 0, None),          # exactly 90 days before as-of
        (7, "2026-08-23", "Amazon", None, 70.0, None, 0, 0, None),          # 91 days before: outside window
        (8, "2026-11-20", "Bbq Guru", None, 45.0, None, 0, 0, None),        # excluded food false positive
        (9, "2026-11-19", "Split parent Amazon", None, 999.0, None, 1, 0, None),  # split parent: not counted
        (10, "2026-11-18", "Restaurant X", None, 30.0, 2, 0, 0, "x" * 70),
    ]
    conn.executemany("INSERT INTO transactions (id,date,payee,original_name,amount,category_id,is_split_parent,"
                     "is_group_parent,user_notes) VALUES (?,?,?,?,?,?,?,?,?)", rows)
    return conn


class Report(unittest.TestCase):
    def setUp(self):
        self.conn = make_db()
        self.addCleanup(self.conn.close)
        self.as_of = dt.date(2026, 11, 21)
        self.report = wr.build_report(self.conn, self.as_of)
        self.text = wr.render_text(self.report)

    def test_balances_use_latest_snapshot_per_month_and_skip_closed_and_other_types(self):
        self.assertIn("Platinum", self.text)
        self.assertIn("3,500", self.text)            # M+0: latest in November
        self.assertIn("2,000", self.text)            # M-1: latest in October
        self.assertIn("1,000", self.text)            # M-2: latest in September
        self.assertNotIn("Old card", self.text)      # closed
        self.assertNotIn("Brokerage", self.text)     # not credit or checking/savings

    def test_bank_and_nickname_are_separate_columns_with_formal_name_fallback(self):
        lines = self.text.splitlines()
        header = next(ln for ln in lines if ln.startswith("Bank"))
        self.assertEqual(header.split()[:2], ["Bank", "Account"])
        credit = lines[lines.index(header):]
        credit = credit[: credit.index("")] if "" in credit else credit
        # a set nickname wins over the formal name
        self.assertTrue(any("Platinum" in ln and "Select" not in ln for ln in lines))
        self.assertIn("Everyday", self.text)
        self.assertNotIn("TD BEYOND", self.text)
        # no nickname (None or blank): the formal name, without the (R)/TM marks
        self.assertIn("CREDITLINE", self.text)
        self.assertIn("Citi Gold Mastercard", self.text)
        self.assertNotIn("\u00ae", self.text)
        self.assertNotIn("\u2122", self.text)
        # the bank name is printed once per bank, not once per account
        citi_rows = [ln for ln in lines if ln.startswith("Citibank")]
        self.assertEqual(len(citi_rows), 1)
        # the bank column holds only bank names (nickname text never lands in it)
        self.assertEqual(sum(1 for ln in lines if ln.startswith("TD Bank")), 2)  # credit section + cash section

    def test_amazon_window_is_90_days_inclusive_and_largest_first(self):
        amazon = [ln for ln in self.text.splitlines() if ln.startswith("2026-")]
        dates = [ln[:10] for ln in amazon]
        self.assertIn("2026-08-24", dates)           # first day of the 90-day window: included
        self.assertNotIn("2026-08-23", dates)        # 91 days back: excluded
        amounts = [float(re.search(r"\$\s*(-?[\d,]+)", ln).group(1).replace(",", ""))
                   for ln in amazon if "Amazon" in ln]
        self.assertEqual(sorted(amounts, reverse=True)[:1], amounts[:1])

    def test_split_parent_is_not_double_counted(self):
        self.assertNotIn("999", self.text)

    def test_food_uses_category_plus_vendor_fallback_and_skips_false_positives(self):
        self.assertIn("Harris Teeter (1)", self.text)
        self.assertIn("Lam's Kitchen (1)", self.text)
        self.assertIn("Restaurant X (1)", self.text)
        self.assertNotIn("Bbq Guru", self.text)
        self.assertIn("food (by vendor)", self.text)

    def test_notes_column_is_truncated_and_passed_through(self):
        self.assertIn("gift", self.text)
        self.assertIn("...", self.text)

    def test_html_is_monospace_and_self_contained(self):
        html = wr.render_html(self.report)
        self.assertIn("JetBrainsMono Nerd Font", html)
        self.assertNotIn("http://", html)
        self.assertNotIn("<script", html)
        self.assertIn("<pre>", html)

    def test_report_works_without_user_notes_column(self):
        conn = sqlite3.connect(":memory:")
        conn.executescript("""
        CREATE TABLE accounts (id INTEGER PRIMARY KEY, type TEXT, subtype TEXT, institution_name TEXT,
            mask TEXT, name TEXT, display_name TEXT, is_closed INTEGER DEFAULT 0);
        CREATE TABLE account_balances (id INTEGER PRIMARY KEY, account_id INTEGER, balance_date TEXT, balance REAL);
        CREATE TABLE categories (id INTEGER PRIMARY KEY, name TEXT);
        CREATE TABLE transactions (id INTEGER PRIMARY KEY, date TEXT, payee TEXT, original_name TEXT, amount REAL,
            category_id INTEGER, is_split_parent INTEGER DEFAULT 0, is_group_parent INTEGER DEFAULT 0);
        INSERT INTO transactions VALUES (1,'2026-11-10','Amazon',NULL,12.0,NULL,0,0);
        INSERT INTO accounts VALUES (1,'credit','credit card','Citibank Online','4567','Citi Gold World Mastercard','x',0);
        INSERT INTO account_balances VALUES (1,1,'2026-11-15',100.0);
        """)
        rep = wr.build_report(conn, self.as_of)
        self.assertEqual(rep["counts"]["amazon"], 1)
        # pre-v5 schema (no nickname column): the formal name is shown
        self.assertIn("Citi Gold World Mastercard", wr.render_text(rep))


class FakeGmail(BaseHTTPRequestHandler):
    sent = []

    def log_message(self, *a):  # quiet
        pass

    def _json(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(n)
        if self.path == "/token":
            assert b"grant_type=refresh_token" in raw
            return self._json(200, {"access_token": "AT-test"})
        if self.path.endswith("/messages/send"):
            assert self.headers.get("Authorization") == "Bearer AT-test"
            FakeGmail.sent.append(json.loads(raw)["raw"])
            return self._json(200, {"id": "msg-123"})
        self._json(404, {})

    def do_GET(self):
        if self.path == "/userinfo":
            assert self.headers.get("Authorization") == "Bearer AT-test"
            return self._json(200, {"email": wr.SENDER})
        self._json(404, {})


class SendPath(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = HTTPServer(("127.0.0.1", 0), FakeGmail)
        port = cls.server.server_address[1]
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        base = f"http://127.0.0.1:{port}"
        os.environ["OMAFINANCES_OAUTH_TOKEN_URL"] = f"{base}/token"
        os.environ["OMAFINANCES_USERINFO_URL"] = f"{base}/userinfo"
        os.environ["OMAFINANCES_GMAIL_API_BASE"] = f"{base}/gmail/v1/users/me"
        cls.tmp = tempfile.TemporaryDirectory()
        cls.client_json = Path(cls.tmp.name) / "client.json"
        cls.client_json.write_text(json.dumps({"installed": {"client_id": "cid", "client_secret": "csec"}}))
        os.environ["OMAFINANCES_GMAIL_REFRESH_TOKEN"] = "RT-test"
        cls.settings = {"mail": {"client_secret_file": str(cls.client_json)},
                        "reports": {"recipient": TEST_TO, "allowed_recipients": [TEST_TO]}}
        cls.report = wr.build_report(make_db(), dt.date(2026, 11, 21))

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.tmp.cleanup()

    def test_send_posts_multipart_message_to_the_allowed_recipient(self):
        FakeGmail.sent = []
        msg_id = wr.send_report(self.report, self.settings, TEST_TO)
        self.assertEqual(msg_id, "msg-123")
        self.assertEqual(len(FakeGmail.sent), 1)
        parsed = email.message_from_bytes(base64.urlsafe_b64decode(FakeGmail.sent[0] + "=="))
        self.assertEqual(parsed["Subject"], wr.SUBJECT)
        self.assertEqual(parsed["To"], TEST_TO)
        self.assertEqual(parsed["From"], wr.SENDER)
        types = {p.get_content_type() for p in parsed.walk()}
        self.assertIn("text/plain", types)
        self.assertIn("text/html", types)

    def test_refuses_any_other_recipient_before_touching_the_network(self):
        with self.assertRaises(SystemExit):
            wr.send_report(self.report, self.settings, to="someone@example.com")

    def test_rejects_header_injection(self):
        with self.assertRaises(SystemExit):
            wr.build_message("hi\r\nBcc: x@example.com", TEST_TO, "t", "<p>h</p>")


if __name__ == "__main__":
    unittest.main()
