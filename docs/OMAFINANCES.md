# OmaFinances

Keyboard-first finance manager over the encrypted Lunch Money mirror (`finances.db`, SQLCipher),
maintained by `sync.py`. Rust backend + Quickshell (QML) UI, for Omarchy/Hyprland.

## Architecture

- **Backend** (`src/`, binary `omafinances`): opens the SQLCipher database, serves the data to the UI
  over loopback HTTP on **127.0.0.1:27183**. Every launch generates a new random token; the UI must
  present it, and the server also checks the `Host` header and rejects requests with an `Origin`
  header.
- **UI** (`qml/`, run by Quickshell with `QML_DIR`): talks only to the backend, never to the database.
- **Database**: SQLCipher file at `database.path` in `settings.toml` (default
  `/mnt/ai/projects/finances-tracker/finances.db`). The passphrase is not in any file the app writes:
  it lives in the Secret Service keyring (service `omafinances`, user `db-passphrase`).
- **Migrations**: the backend makes a backup of the database before any schema migration.
- **Shared database**: `sync.py` writes the same file. The app must not add FTS tables or triggers
  to it.

## Build, run, test

```sh
./build.sh                 # debug build -> target/debug/omafinances (or OMAFINANCES_BUILD_DIR)
./build.sh --release       # release build
./build.sh --clean --live  # clean, then install live (see below)
./build.sh --help
cargo test                 # unit tests
```

`--live` implies `--release`. It installs to `${OMAFINANCES_LIVE_DIR:-/opt/omafinances}`:

- `bin/omafinances` (installed by rename, so a running instance is not disturbed mid-write)
- `qml/` (copied from the project, without the Omarchy shell-kit links `Commons` and `Ui`)
- `share/omafinances.svg` (icon)
- `omafinances`: the launcher. If the window is already open, it focuses it instead of starting a
  second one.
- `~/.local/share/applications/OmaFinances.desktop`: the menu entry. It is written only when missing
  or different. Omarchy's Apps menu picks it up from there; no menu JSON is edited.

`--live` stops any running OmaFinances window and backend first (the live copy and the dev copies
under the build dir only; other apps are never matched) and restarts the app afterwards if it was
open. The build log and restart log go to the build dir.

Test hooks: `OMAFINANCES_SKIP_BUILD=1 OMAFINANCES_PREBUILT=<binary>` skips cargo and installs the
given binary. Use it with `OMAFINANCES_LIVE_DIR` and `XDG_DATA_HOME` pointing at scratch dirs to
rehearse the install without touching `/opt` or the real menu.

## Config directory

`${XDG_CONFIG_HOME:-~/.config}/omafinances/` (mode 0700):

- `settings.toml` (mode 0600), created with defaults on first run:
  - `[database] path` — the SQLCipher file.
  - `[ui] scale_factor` — 0.5 to 3.0, default 1.0.
  - `[mcp] enabled` — default `false`. Reserved; the MCP server only runs while the app is open.
- `build.env` (optional, not committed): `OMAFINANCES_BUILD_DIR=/path/to/build` tells `build.sh`
  where cargo writes its output.

No secrets are stored in either file.

## First run

1. `./build.sh --live`. The first run asks for sudo once to create `/opt/omafinances` owned by you.
   Later runs need no sudo.
2. Put the database passphrase in the keyring: `omafinances keyring-import`. This copies
   `DB_PASSPHRASE` from a `.env`-style file into the Secret Service keyring. If the keyring entry is
   missing, run this command, then start the app again.
3. Launch OmaFinances from the Omarchy Apps menu (or `/opt/omafinances/omafinances`).

## Security rules

- No secrets in logs, in error messages, or in the UI. Never print the passphrase or the Lunch Money
  token.
- Bound every SQL parameter; never build SQL by string concatenation from user input.
- Every HTTP request is checked against the per-launch token; the token is never written to disk.
- Never decrypt the database to a plaintext file, not even temporarily.
- Read-only against Lunch Money: the app never writes back to Lunch Money.
- Back up the database before any migration; migrations must be re-runnable.
- Nothing is written outside the database, the config dir, the live dir and the menu entry.
