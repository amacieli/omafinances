# Phase 6 contract: reports + emailed schedule

Report = saved query (the slash-search grammar, same parser as the UI) + shape + period + schedule + recipients.
Rules: own Gmail token in the keyring (service `omafinances`, user `gmail-refresh-token`), send-only scope
(`https://www.googleapis.com/auth/gmail.send`) + `userinfo.email`; OAuth client (installed app, loopback + PKCE) from a Google
client JSON whose path is in settings `[mail] client_secret_file` (no secrets in TOML, no secret printed/logged ever).
Recipients allowlist: settings `[reports] allowed_recipients` (default = only the authenticated sender). Never send to anyone else
(400 on create/update, hard refusal at send time too). Never writes to Lunch Money. No Python. No triggers/FTS/WAL.

## Migration v3 (re-runnable, backup first like earlier migrations; `ALTER`/`CREATE IF NOT EXISTS` only)
```
report(id INTEGER PK, name TEXT UNIQUE NOT NULL, query TEXT NOT NULL DEFAULT '', shape TEXT NOT NULL,   -- table|by_category|by_payee|by_month|summary
       period TEXT NOT NULL, schedule_json TEXT NOT NULL, recipients_json TEXT NOT NULL, redact INTEGER NOT NULL DEFAULT 0,
       enabled INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL DEFAULT (datetime('now')))
report_run(id INTEGER PK, report_id INTEGER NOT NULL REFERENCES report(id) ON DELETE CASCADE, started_at TEXT NOT NULL,
           finished_at TEXT, status TEXT NOT NULL,   -- ok|failed|skipped
           detail TEXT, rows INTEGER, message_id TEXT, trigger TEXT NOT NULL)   -- timer|manual|test|catchup
```
## Period (resolved at run time against local "today")
`last_7d`, `last_full_week` (Mon-Sun), `last_full_month`, `month_to_date`, `last_full_quarter`, `ytd`, `all`. A `d:` filter inside
the query is ANDed with the period unless period = `all`.
## Schedule JSON
`{"kind":"daily","time":"07:30"}` | `{"kind":"weekly","weekday":"mon","time":"07:30"}` | `{"kind":"monthly","day":1,"time":"07:30"}`
(day 1..28, or `"last"`) | `{"kind":"quarterly","day":1,"time":"07:30"}` (Jan/Apr/Jul/Oct). `src/report/schedule.rs`:
`next_runs(&Schedule, after: NaiveDateTime, n) -> Vec<NaiveDateTime>` and `to_oncalendar(&Schedule) -> String` (systemd
OnCalendar). Both unit-tested incl. month ends, DST-free local time, quarter boundaries.
## Shapes and rendering (`src/report/`: model.rs, run.rs, render.rs, schedule.rs, units.rs, cli.rs)
Run: parse query with `search::parse`, evaluate against the cache (load the Cache from the DB headlessly), apply period, apply
transfer exclusion (reuse `insight::transfers`; setting default true), compute shape.
 - `table`: rows (date, payee, account, category, amount), cap 500 rows in HTML (full list in CSV), totals line.
 - `by_category` / `by_payee` / `by_month`: grouped out/in/net/count, sorted by out desc (month: chronological), share %.
 - `summary`: totals out/in/net for the period vs the previous period of equal length with delta and delta %, top 5 categories, top 5 payees, new
   anomalies in the period (reuse `insight::anomalies`), recurring charges due in the next 14 days (reuse `insight::recurring`).
Output: HTML email (inline CSS only, no remote assets, table-based layout, dark text on white), plain-text alternative, CSV attachment
`<name>-<period start>-<end>.csv` (CSV injection guard: prefix cells starting with `= + - @` with `'`). `redact` = true hides account names
and truncates payees to 12 chars + hides notes. Amount sign convention stated in the footer ("positive = money out").
Subject: `[OmaFinances] <name>: <period label>`. First lines of the text part never contain amounts' sign confusion.
## Routes (token-guarded; POST/PUT/DELETE bodies JSON; errors as elsewhere)
`GET /reports` -> `{reports:[{id,name,query,shape,period,schedule,recipients,redact,enabled,last_run:{status,finished_at,detail}|null,next_runs:[ISO x3]}]}`
`POST /reports` {name,query,shape,period,schedule,recipients?,redact?} -> 200 {id}; recipients default [sender]; 400 on bad query (parser error text), bad schedule,
duplicate name (409), non-allowlisted recipient.   `PUT /reports/{id}` same fields (all optional)   `DELETE /reports/{id}` (also removes timers).
`POST /reports/{id}/preview` -> `{subject,text,html,csv_rows,rows,period:{from,to,label}}` (no send, no run log)
`POST /reports/{id}/send-test` -> sends to the authenticated sender only; logs a run with trigger `test`; 409 `not_authenticated` when no Gmail token.
`POST /reports/{id}/enable` {enabled:bool} -> writes/removes the systemd user timer (enabling requires a successful earlier send-test OR preview in this app
session? NO: simply requires Gmail authenticated, else 409 `not_authenticated`).   `GET /reports/{id}/runs?limit=20` -> `{runs:[...]}`
`GET /schedule/preview?kind=weekly&weekday=mon&time=07:30` (and day=) -> `{next_runs:[ISO x3], oncalendar}`   `GET /mail/status` -> `{authenticated:bool,sender:null|"..","client_configured":bool}`
## Headless CLI (dispatch in main.rs, owned by the reports lane)
`omafinances report run <id> [--trigger timer|catchup|manual]`  opens DB via keyring (no UI, no quickshell), runs, sends via `GmailMailer`, writes report_run,
exit code 0 ok / 1 failed (message to stderr without row text). `omafinances report timers sync` makes systemd user units match enabled reports.
`omafinances report list`. `omafinances gmail-auth` / `gmail-status` (mail lane, in `src/mail/cli.rs`; main.rs already dispatches to them or the reports lane adds the dispatch).
## systemd (`units.rs`)
For each ENABLED report write `~/.config/systemd/user/omafinances-report-<id>.service` (Type=oneshot, `ExecStart=<installed binary path> report run <id> --trigger timer`,
binary path = `/opt/omafinances/bin/omafinances` when it exists else `current_exe()`) and `.timer` (`OnCalendar=`, `Persistent=true`, `RandomizedDelaySec=60`,
`WantedBy=timers.target`). Enable/disable with `systemctl --user enable --now / disable --now` and `daemon-reload`; all command invocations go through a small
`Systemd` trait so tests use a fake and write to a temp dir (env `OMAFINANCES_SYSTEMD_DIR`). Disabled/deleted reports have units removed; `timers sync` is idempotent
and removes orphaned `omafinances-report-*` units.
## Gmail (mail lane: src/mail/{gmail.rs,oauth.rs,mime.rs,cli.rs})
Loopback authorization-code + PKCE flow on 127.0.0.1:<ephemeral>; `gmail-auth` prints the consent URL (does NOT open a browser itself unless `--open`), waits for the
redirect (timeout 5 min), stores ONLY the refresh token in the keyring, prints the sender address. Access token cached in memory only; refresh on 401. `GmailMailer`
implements `mail::Mailer`: builds RFC 5322 multipart/mixed(alternative(text,html), attachments) with proper header encoding, base64url, `POST users/me/messages/send`.
HTTP base URLs overridable by env (`OMAFINANCES_GMAIL_API_BASE`, `OMAFINANCES_OAUTH_TOKEN_URL`) so tests run against a local fake server (tiny_http is already a dependency).
Choose a blocking HTTPS client (e.g. `ureq` with rustls) and add it to Cargo.toml. MIME builder unit-tested (UTF-8 subjects, CRLF, boundaries, header injection refused in
subject/to), PKCE verifier/challenge test vector, token refresh + 401 retry test against the fake, never log tokens (test that error strings contain no token).
## Frontend (QML, later lane): Reports overlay (`P`), slash commands `:report` / `:reports`; list, edit (name, query default = current query, shape, period, schedule presets with next 3 runs, recipients), `p` preview, `t` send test, `e` enable/disable, run history, mail status line with "run: omafinances gmail-auth" hint when unauthenticated.
