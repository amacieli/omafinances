# Tally — a fast, keyboard-first finance TUI in Rust

> Working name "Tally" (binary `tally`); rename freely. Plan only — no code written.

**Goal:** A local-first, beautiful, keyboard-driven terminal app over the existing encrypted Lunch Money mirror
(`/mnt/ai/projects/omafinances/finances.db`) with slash-search, fast filter/sort/categorize, a
preview-before-apply bulk categorizer, scheduled emailed reports, and an optional MCP server that exists only while the app runs.

**Principle:** intelligence lives in the app (deterministic + statistical, no LLM needed). The MCP server is a convenience door for agents, never a dependency.

---

## 1. What the database says today (measured 2026-11-20)

| Fact | Value |
|---|---|
| Transactions | 1,519 (2025-03-14 → 2026-11-13), 367 distinct payees, all `status='unreviewed'` |
| Accounts | 7 |
| Lunch Money categories (`categories`) | 10 rows, **2 levels** (6 groups, 4 children). 325 txns carry one; **1,194 (79%) have none** |
| Your 3-tier tables | `category_main` 19, `category_sub` 16, `category_subsub` **0** |
| Txns assigned in your 3-tier tables | **0** of 1,519 |
| Splits | 0 |
| Sign convention | 1,380 positive / 139 negative → positive = money out (LM v2). Must be explicit in the app |
| Tags / notes | none used |
| `recurring_id` set | 255 |

**Level analysis.** Designed depth = 3, populated depth = 2, *used* depth = 0. The load flattened a deeper hierarchy: `category_main` contains
entries that are clearly children in your own `categories.json` intent — Loans → *Loan Fees and Charges*, *Loan Insurance*;
Taxes → *Federal Estimated Tax Payment* → *Form 1040: Federal estimated tax, qrtrly*; *Car Insurance*, *Gym*, *Healthcare* (which has *Doctor*) sit beside
PRODUCTIVE/PROTECTIVE EXPENSES and probably belong under them. (This re-parenting is my inference from the names — to be confirmed by you in the category manager, not auto-applied.)

**Starting point:** a 3-level tree (Main › Sub › Subsub), seeded from your 35 entries with the re-parenting proposals above, with depth a soft limit (setting, default 3).
Lunch Money's 10 categories are kept as a *source hint* (e.g., "LM said Groceries") feeding suggestions, not as the truth.

**Schema decision:** collapse `category_main/sub/subsub` into one adjacency-list `category(id, parent_id, name, sort, color, icon, kind)` table + `transaction.category_id_local`.
Reasons: arbitrary re-parenting/merge is a single UPDATE, depth is a policy not a schema, queries use a recursive CTE / materialized `path`. Migration is re-runnable, preceded by an automatic DB backup (project rule).
`sync.py` upserts only name the LM columns, so local category columns survive sync (verified: no `category_main` in `sync.py`).

---

## 2. Architecture

Cargo workspace at `/mnt/ai/projects/omafinances/tally/` (git repo from day one; no feature branches, same convention as omalook; push only when asked).

```
tally-core    db (rusqlite + bundled-sqlcipher), models, migrations, change journal (undo), query language,
              normalizer, similarity/suggest engine, rules, sampling, report builder
tally-tui     ratatui + crossterm UI, theme, keymap, widgets
tally-mail    Gmail send, HTML/CSV report rendering, schedule definitions
tally-mcp     MCP server (rmcp) + stdio bridge binary
tally-sync    Lunch Money client (port of sync.py) — late phase
tally         the binary: `tally` (TUI), `tally report run <id>`, `tally mcp-bridge`, `tally sync`
```

- **Why a TUI (ratatui), not Qt/QML like omalook:** you said "text-based". Runs in your terminal, instant start, inherits the Omarchy palette, no Quickshell crash class (OL-004). `tally-core` has no UI dependency, so a QML front end remains possible later. *Say so if you meant a Quickshell QML UI instead.*
- **Security/DB:** SQLCipher passphrase moves from `.env` to the keyring (service `tally`), with `.env` fallback during transition. Never decrypt to disk, never log secrets/payees/amounts at info level. New dirs 0700, files 0600. All SQL via bound params, `LIKE … ESCAPE`.
- **Speed:** whole dataset (~1.5k rows today, plausibly 50k) loaded into an in-memory columnar cache at start; filtering/sorting are in-process, target <5 ms per keystroke at 50k rows. Full re-render only on change (dirty-flag), 60 fps cap. Startup to interactive <150 ms.
- **Beauty:** truecolor theme read from Omarchy's current theme (`~/.config/omarchy/current/theme/`), live-reloaded; rounded borders, subtle row banding, amounts right-aligned with tabular figures and red/green by sign, category chips with colors, braille/block sparklines per payee and per category, month bar charts, smooth scrolling, toast notifications, a help overlay that is generated from the keymap (so docs never drift). Nerd Font icons optional with ASCII fallback.
- **Test strategy:** `insta` snapshot tests on ratatui `TestBackend` for every screen; property tests (`proptest`) for the query parser round-trip and normalizer; golden fixtures for the suggestion engine from a synthetic encrypted DB; `criterion` benches with a 50k-row generated DB.

---

## 3. Core design pieces

### 3.1 Slash search (the heart of the UX)
`/` opens a bar anywhere. One grammar, live results, ghost-text completion, history (`↑`), `Esc` cancels, `Enter` pins.

```
/amazon                      free text: fuzzy on payee+notes (FTS5 + trigram)
/p:harris acct:td >50        field filters; bare numbers/comparators = amount
/c:home/maint                category path (prefix, fuzzy); /c:none or /uncat  → uncategorized
/d:2026-10..2026-11  /d:last-month  /d:ytd  /d:>30d
/is:recurring  /is:pending  /is:transfer  /is:income
/amount:>=100 /amount:10..50
/-p:amazon                   negation
/sort:-amount  /group:category  /by:month
```
- Implementation: hand-written Pratt/`winnow` parser → typed AST → evaluated against the in-memory cache; the same AST also compiles to parameterized SQL (used by reports/MCP/email so all three agree). Error messages inline, never block typing.
- Leading `/` also opens **slash commands** when the first token is a verb: `/cat Home/Maintenance`, `/save weekly-groceries`, `/report weekly-groceries`, `/undo`, `/settings`, `/mcp on`. Palette (`Ctrl-p`) lists the same commands fuzzily.
- **Saved views:** `/save name` stores the AST + sort + columns; listed in sidebar, jump with `g` + letter or `/view name`.

### 3.2 Keyboard model (vim-ish, discoverable)
`j/k` move, `g/G` ends, `Ctrl-d/u` page, `Space` mark, `v` visual range, `a` mark all in view, `x` clear.
`f` quick-filter menu (account/category/date/amount), `s` sort picker (then `1-9` or letter; same key reverses), `o` columns.
`c` categorize (picker with fuzzy type-ahead over the tree), `C` categorize + find similar (§3.4), `m` category manager, `u`/`Ctrl-r` undo/redo, `Enter` detail pane, `Tab` cycles panes, `?` help, `q` quit. Text fields own their keys; global bindings only when no field has focus (omalook rule). Everything documented in `docs/KEYS.md` and generated into the help overlay.

### 3.3 Category manager (`m`)
Tree view with per-node totals (txn count, YTD spend). Keys: `a` add child, `A` add sibling, `r` rename, `H/L` outdent/indent (re-parent), `J/K` reorder, `d` delete (forced reassign dialog: choose target; shows impacted count), `M` merge into…, `e` color/icon. Depth-limit warning. Every action goes through the change journal so it is undoable. Includes the one-time **"Review proposed hierarchy"** wizard for the re-parenting in §1.

### 3.4 Categorize-with-preview (the headline feature)
Flow: select txn (or several) → `C` → pick Category/Sub/Subsub → app searches **the whole base** → preview screen → confirm → apply.

1. **Normalize** payee: lowercase, strip store/terminal numbers, `POS`/`SQ *`/`TST*`/`PAYPAL *` style prefixes, trailing city/state, dates, card tails; keep a user-editable alias table (`AMZN Mktp US*1A2B3` → `Amazon`).
2. **Find candidates** in ranked tiers: (a) same normalized payee; (b) fuzzy payee (trigram Jaccard + Jaro-Winkler ≥ threshold); (c) same `recurring_id`; (d) same payee-token set with similar amount band. Each candidate gets a confidence and a reason.
3. **Representative sample**, not a dump: group matches into clusters (payee variant × amount band); show per cluster count, total $, date span, and 2–3 exemplars chosen stratified (earliest, latest, median amount, and the outlier farthest from cluster centroid). Always surface **conflicts**: matched txns already categorized differently (shown in a distinct color, excluded by default).
4. **Confirm UI:** cluster list with checkboxes (`Space` toggle, `a` all, `n` none), expand a cluster with `Enter` to see every row, header shows "Will categorize 214 of 1,194 uncategorized · 11 conflicts skipped · $18,402". Dry-run summary before `y`. Threshold slider (`[`/`]`) widens/narrows the match live.
5. **Apply** in one DB transaction, recorded as one journal entry (undo = one key). Optional checkbox **"Also make this a rule"** for future syncs (`payee ~ X [amount range] → category`), rules are visible/editable in `/rules`.
6. Never writes back to Lunch Money (project rule), unless a later explicit opt-in feature is built.

### 3.5 Local intelligence (no agent required)
- **Rules engine** (ordered, first-match, explainable: "matched rule #7").
- **Suggest engine** for uncategorized rows: weighted kNN / multinomial naive Bayes over normalized-payee tokens + amount bucket + account + day-of-month, trained on your categorized rows and LM's category as a feature; shows top-3 with confidence. High-confidence suggestions appear as a ghost chip; `Tab` accepts. Review queue `/is:uncat sort:-confidence` makes clearing the 1,194 backlog fast (accept/skip loop).
- **Recurring/subscription detection** (period + amount stability), anomaly flags (amount 3σ above that payee's norm, duplicate charge within N days, new payee).
- **Transfer pairing** (the 101 "Payment, Transfer" rows): match opposite amounts across accounts within a few days and exclude from spend totals.
- Deterministic, unit-tested, runs in-process in milliseconds. A local embedding model is explicitly *out* of scope until the above proves insufficient (YAGNI).

### 3.6 Scheduled emailed reports
- **Report = saved view + shape + schedule + recipients.** Shapes: table, grouped totals (by category/payee/month), summary with delta vs previous period. Output: HTML email (inline CSS, plain-text alternative) + CSV attachment.
- **Schedule UI** (`/report`): presets (daily / weekly on a weekday+time / monthly on day N / quarterly) plus advanced cron-style; always shows the next 3 run times; "send test now". Relative periods ("last full week") resolved at run time.
- **Runs even when the TUI is closed:** the app writes **systemd user timer + service units** (`tally-report@<id>.timer`, `Persistent=true` so a missed run fires after sleep/boot) that call `tally report run <id>` headlessly (opens the DB via keyring). A run log table records status; failures surface as a banner in the TUI.
- **Gmail:** send via the Gmail API (`gmail.send` scope) using the existing Google connection. Candidates: Hermes's `~/.hermes/google_token.json` (the `google-workspace` skill; `gws` is not currently on PATH) or omalook's Gmail account. Recommendation: Tally holds its own refresh token in the keyring obtained through a one-time consent, send-only scope — keeps it decoupled from both Hermes and omalook's running daemon. (See open question 2.)
- **Guardrails:** recipients allowlist (default: your own address); report previews in-app before enabling; an "amounts redaction" option (hide account numbers/payee notes); email is plaintext in transit/at rest in Gmail, so the UI warns once on first enable.

### 3.7 MCP server (switchable, only while running)
- Settings toggle `MCP: on/off` (and `/mcp on|off`). Enabling requires an on-screen confirmation (omalook rule: anything widening agent capability needs UI confirmation). Default off.
- **In-process listener**, so lifetime == app lifetime: Unix socket `$XDG_RUNTIME_DIR/tally/mcp.sock` (0600), per-launch token. Hermes config points at a tiny stdio shim `tally mcp-bridge` that connects to the socket; if the app is closed or MCP is off the shim exposes no tools / returns "tally not running". This is the opposite of omalook's always-available model, as requested.
- **Tools:** `search_transactions(query)` (uses the same slash grammar), `summarize(query, group_by)`, `list_categories`, `list_views`, `get_report_preview`, `suggest_categories(txn_ids)`, `propose_categorization(txn_ids|query, category)`.
- **Write policy:** agents can only *propose*. A proposal appears in the TUI as a pending-review banner that opens the same preview screen from §3.4; the human confirms. No raw SQL tool, no arbitrary file paths, row caps per call, redaction honors settings, every call logged (method + tool name only, no payloads) and shown in an MCP activity pane.
- Crate: `rmcp`, pinned and recorded in `COMPONENT_REFERENCE.md` (create one for this project, like omalook's).

---

## 4. Delivery chunks (each ships something usable)

| # | Chunk | You can… | Exit criteria |
|---|---|---|---|
| **0** | Foundations | run `tally` and see a themed shell opening the real DB | SQLCipher opens via keyring; auto-backup; migration framework with a re-runnable v1 migration (category tree, journal, rules, views, settings, aliases); CI-style `cargo test` green; startup <150 ms |
| **1** | Browse | scroll all transactions, sort/filter via menus, see detail, accounts & category sidebar | 50k-row synthetic bench: filter+sort <5 ms; snapshot tests; KEYS.md v1 |
| **2** | Slash search + views | type `/` queries, save/recall views, completions, history | Parser property tests; AST→SQL equals cache results on a fuzz corpus; p50 <10 ms |
| **3** | Categories + manual categorize | manage the tree, review proposed hierarchy, categorize single/multi with undo | Migration of the 35 entries verified; undo/redo journal tests; delete-with-reassign safe |
| **4** | **Smart categorize with preview** | `C` on a txn → clusters + samples + conflicts → confirm → undo | Sampling/ranking golden tests on real-shape data; apply is atomic; "make rule" works; never overwrites existing without opt-in |
| **5** | Intelligence & insights | suggestion chips, review queue, recurring/anomaly/transfer detection, dashboard (spend by category, monthly trend) | Measured top-1/top-3 suggestion accuracy reported on your labeled 325 rows (held-out) before turning on auto-suggest |
| **6** | Reports + email schedule | define a filter, schedule it weekly, get it in Gmail | Test-send OK; systemd timers generated/removed cleanly; missed-run catch-up verified; run log visible |
| **7** | MCP | toggle on in settings; Hermes can search & propose | Socket/token tests; off ⇒ zero tools; proposals can't apply without TUI confirm; guard tests per route |
| **8** | Native sync + polish | `r` refreshes from Lunch Money, retire Python cron | Rust port parity-tested against `sync.py` output; rules auto-run on newly synced rows; packaging (`PKGBUILD`/install script), Omarchy launcher entry via the `omarchy-app-integration` skill |

Suggested order is as listed; 5 can overlap 4. Chunks 0–4 deliver the core of what you described; 6 and 7 are independent of each other.

---

## 5. Risks and trade-offs
- **Sign/transfer semantics** can quietly corrupt totals; fix them in chunk 1 with explicit tests rather than trusting column names.
- **Only 325 labeled rows** (and 0 in your own tree) → early suggestions are weak; the preview+rule flow is what bootstraps the labels. Accuracy is measured before it's shown.
- **Schema consolidation** changes a project you've documented; mitigated by backup + re-runnable migration + compat view (`category_main/sub/subsub` as views) so `query_categories.py` and existing skills keep working until retired.
- **Dual writers** (Python sync cron + Tally) on one SQLCipher file: use WAL + busy timeout, and keep sync from touching local columns until chunk 8 replaces it.
- **Email of finances** to Gmail: allowlist, redaction option, first-use warning.
- **Terminal limits:** charts are braille/blocks, not pixels. If this proves too constraining the QML option exists thanks to the core/UI split.
- **Doc/skill upkeep:** update the `finances-tracker` skill and AGENTS.md each chunk; stale "19 main + 16 sub" facts will change.

## 6. Open questions (defaults in brackets; I'll proceed on defaults unless you say otherwise)
1. TUI in the terminal vs Qt/QML like omalook? [**ratatui TUI**]
2. Gmail path: dedicated send-only token for Tally vs reuse Hermes's `google_token.json` vs go through omalook? [**dedicated send-only token**]
3. Confirm the 3-level starting depth and the proposed re-parenting (Loans, Taxes, Car Insurance/Gym/Healthcare under PROTECTIVE?). [**3 levels; you review in the wizard**]
4. Scope of the categories: spending only, or also income/transfer handling and budgets? [**expenses + income + transfer kinds; budgets deferred**]
5. Keep Lunch Money as the source of record with Tally strictly read-only toward it? [**yes**]
6. Project location `/mnt/ai/projects/omafinances/tally/` in the existing folder, or a new repo? [**subfolder, own git repo**]
