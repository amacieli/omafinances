//! OmaFinances launcher: starts the guarded loopback backend, then the Quickshell QML window, and exits with it.
//!
//!   omafinances                      open the app
//!   omafinances keyring-import [F]   copy DB_PASSPHRASE from F (default: .env beside the database) into the keyring
//!   omafinances check                open + migrate the database and print counts (no secrets, no payees)
//!   omafinances --version

use anyhow::{bail, Context, Result};
use omafinances::{api::AppState, config, db::Db, http, secrets};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const KIT_DIR: &str = "/usr/share/omarchy/shell";

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("omafinances=info,warn")).init();
    if let Err(e) = run() {
        eprintln!("omafinances: {:#}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") | Some("-V") => {
            println!("omafinances {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("keyring-import") => keyring_import(args.get(1).map(PathBuf::from)),
        Some("check") => check(),
        None => launch(),
        Some(other) => bail!("unknown argument {:?} (try: keyring-import, check, --version)", other),
    }
}

fn open_db(settings: &config::Settings) -> std::result::Result<Db, String> {
    let pass = match secrets::db_passphrase() {
        Ok(Some(p)) => p,
        Ok(None) => return Err("database passphrase not in the keyring".to_string()),
        Err(e) => return Err(format!("{:#}", e)),
    };
    let db = Db::open(Path::new(&settings.database.path), &pass).map_err(|e| format!("{:#}", e))?;
    match db.migrate() {
        Ok(r) => {
            if r.from != r.to {
                log::info!("database migrated v{} -> v{} (backup: {})", r.from, r.to, r.backup.map(|p| p.display().to_string()).unwrap_or_default());
            }
            Ok(db)
        }
        Err(e) => Err(format!("migration failed: {:#}", e)),
    }
}

fn keyring_import(file: Option<PathBuf>) -> Result<()> {
    let cfg = config::config_dir()?;
    let settings = config::load_or_init(&cfg)?;
    let file = file.unwrap_or_else(|| Path::new(&settings.database.path).with_file_name(".env"));
    let replaced = secrets::import_from_env_file(&file)?;
    println!("database passphrase {} in the keyring (service {}); the .env file was left untouched", if replaced { "replaced" } else { "stored" }, secrets::SERVICE);
    Ok(())
}

fn check() -> Result<()> {
    let cfg = config::config_dir()?;
    let settings = config::load_or_init(&cfg)?;
    let db = open_db(&settings).map_err(|e| anyhow::anyhow!(e))?;
    let s = db.status()?;
    println!(
        "ok: schema v{}, {} transactions, {} accounts, {} categories, newest txn {}, last sync {}",
        s.schema_version, s.transactions, s.accounts, s.categories,
        s.last_txn_date.as_deref().unwrap_or("-"), s.last_sync.as_deref().unwrap_or("-")
    );
    Ok(())
}

fn launch() -> Result<()> {
    let cfg = config::config_dir()?;
    let settings = config::load_or_init(&cfg)?;
    let port: u16 = std::env::var("OMAFINANCES_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(config::DEFAULT_PORT);
    let qml_dir = resolve_qml_dir()?;
    ensure_kit(&qml_dir)?;

    let db = open_db(&settings);
    if let Err(e) = &db {
        log::warn!("database not available: {}", e);
    }
    let state = AppState::new(settings, db, env!("CARGO_PKG_VERSION"));
    match state.refresh() {
        Ok(n) => log::info!("cache: {} transactions loaded", n),
        Err(e) => log::warn!("cache not loaded: {:#}", e),
    }
    let state = Arc::new(state);
    let token = http::session::create(&cfg, port).context("write session token")?;
    let handle = http::spawn(port, token.clone(), state).with_context(|| format!("cannot listen on 127.0.0.1:{} (already running?)", port))?;

    let status = Command::new("quickshell")
        .arg("-p").arg(&qml_dir)
        .env("QML_DIR", &qml_dir)
        .env("OMAFINANCES_PORT", port.to_string())
        .env(http::session::ENV, &token)
        .status();
    handle.shutdown();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => bail!("quickshell exited with {}", s),
        Err(e) => bail!("could not start quickshell ({}); install it with: pacman -S quickshell", e),
    }
}

fn resolve_qml_dir() -> Result<PathBuf> {
    if let Some(d) = std::env::var_os("QML_DIR").map(PathBuf::from) {
        return Ok(d.canonicalize().unwrap_or(d)); // absolute, so the launcher's pgrep pattern matches
    }
    let mut cands = vec![];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = exe.parent().and_then(|b| b.parent()) {
            cands.push(p.join("qml")); // <live>/bin/omafinances -> <live>/qml
        }
    }
    cands.push(PathBuf::from("qml"));
    cands.into_iter().find(|d| d.join("shell.qml").is_file()).and_then(|d| d.canonicalize().ok()).context("qml/shell.qml not found (set QML_DIR)")
}

/// Quickshell resolves `qs.*` against the config root: link the installed Omarchy kit in (idempotent).
fn ensure_kit(qml_dir: &Path) -> Result<()> {
    for name in ["Commons", "Ui"] {
        let target = Path::new(KIT_DIR).join(name);
        if !target.exists() {
            bail!("{} not found: OmaFinances needs the Omarchy shell UI kit", target.display());
        }
        let link = qml_dir.join(name);
        if link.symlink_metadata().is_err() {
            std::os::unix::fs::symlink(&target, &link).with_context(|| format!("link {}", link.display()))?;
        }
    }
    Ok(())
}
