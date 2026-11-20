//! The per-launch session token for the UI's loopback port (SEC-001 local-process vector).
//!
//! Each backend start makes a new 256-bit token and writes it to
//! `$XDG_RUNTIME_DIR/omafinances/ui-<port>.token` (directory 0700, file 0600, written to a temp file
//! and renamed). Without a runtime dir (no login session) it goes to `<config dir>/run/` instead.
//! The launcher hands it to the UI in `OMAFINANCES_TOKEN`. `http::guard` refuses every request
//! without it. One file per port, so a scratch instance on another port has its own token.
//! Same-uid processes can still read the file; they can read the keyring and the database too,
//! so that is the accepted residual.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Environment variable the launcher sets for the UI process.
pub const ENV: &str = "OMAFINANCES_TOKEN";

/// Where the token for `port` lives, given the runtime dir (if any) and the config dir.
pub fn token_path_in(runtime_dir: Option<&Path>, config_dir: &Path, port: u16) -> PathBuf {
    let base = match runtime_dir {
        Some(d) if d.is_absolute() && d.is_dir() => d.join("omafinances"),
        _ => config_dir.join("run"),
    };
    base.join(format!("ui-{}.token", port))
}

/// Where the token for `port` lives: `$XDG_RUNTIME_DIR/omafinances/`, or `<config dir>/run/`.
pub fn token_path(config_dir: &Path, port: u16) -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    token_path_in(runtime.as_deref(), config_dir, port)
}

fn random_hex() -> std::io::Result<String> {
    let mut buf = [0u8; 32];
    fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{:02x}", b)).collect())
}

/// Make a new token for this backend start and write it (replacing any earlier one for the port).
pub fn create(config_dir: &Path, port: u16) -> std::io::Result<String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let path = token_path(config_dir, port);
    let dir = path.parent().expect("token path has a directory");
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    let token = random_hex()?;
    let tmp = path.with_extension("token.tmp");
    let _ = fs::remove_file(&tmp);
    let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
    f.write_all(token.as_bytes())?;
    f.sync_all()?;
    fs::rename(&tmp, &path)?;
    Ok(token)
}

/// The token of the backend running on `port`, if its file is there and well-formed.
pub fn read(config_dir: &Path, port: u16) -> Option<String> {
    let t = fs::read_to_string(token_path(config_dir, port)).ok()?;
    let t = t.trim();
    (t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit())).then(|| t.to_string())
}

/// The `Authorization` header value for `token`.
pub fn bearer(token: &str) -> String {
    format!("Bearer {}", token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn path_uses_runtime_dir_or_falls_back_to_config_run() {
        let cfg = Path::new("/home/x/.config/omafinances");
        assert_eq!(
            token_path_in(Some(Path::new("/tmp")), cfg, 27183),
            PathBuf::from("/tmp/omafinances/ui-27183.token")
        );
        assert_eq!(token_path_in(None, cfg, 27183), cfg.join("run/ui-27183.token"));
        // a relative or missing runtime dir is not trusted
        assert_eq!(token_path_in(Some(Path::new("relative")), cfg, 1), cfg.join("run/ui-1.token"));
        assert_eq!(
            token_path_in(Some(Path::new("/does/not/exist")), cfg, 1),
            cfg.join("run/ui-1.token")
        );
    }

    #[test]
    fn create_writes_a_private_file_and_read_finds_it() {
        let cfg = std::env::temp_dir().join(format!("omafinances-session-{}", std::process::id()));
        let _ = fs::remove_dir_all(&cfg);
        let port = 40000 + (std::process::id() % 20000) as u16;
        let t1 = create(&cfg, port).unwrap();
        assert_eq!(t1.len(), 64);
        let p = token_path(&cfg, port);
        assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(p.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(read(&cfg, port).as_deref(), Some(t1.as_str()));
        // every start makes a new one
        let t2 = create(&cfg, port).unwrap();
        assert_ne!(t1, t2);
        assert_eq!(read(&cfg, port).as_deref(), Some(t2.as_str()));
        // one file per port
        assert_ne!(token_path(&cfg, port), token_path(&cfg, port + 1));
        assert_eq!(read(&cfg, port + 1), None);
        let _ = fs::remove_file(&p);
        let _ = fs::remove_dir_all(&cfg);
    }

    #[test]
    fn read_rejects_junk() {
        let cfg = std::env::temp_dir().join(format!("omafinances-session-junk-{}", std::process::id()));
        let port = 39000 + (std::process::id() % 1000) as u16;
        let p = token_path(&cfg, port);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, "short").unwrap();
        assert_eq!(read(&cfg, port), None);
        fs::write(&p, "z".repeat(64)).unwrap();
        assert_eq!(read(&cfg, port), None);
        let _ = fs::remove_file(&p);
        let _ = fs::remove_dir_all(&cfg);
    }

    #[test]
    fn bearer_formats_the_header() {
        assert_eq!(bearer("abc"), "Bearer abc");
    }
}
