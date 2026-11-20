//! `omafinances gmail-auth [--open]` (one-time consent) and `omafinances gmail-status`.
//! The consent URL is printed for the user to open; `--open` also tries `xdg-open`. Only the refresh token is stored.

use crate::config;
use crate::mail::{self, oauth};
use crate::secrets;
use anyhow::{bail, Context, Result};

pub fn gmail_auth(args: &[String]) -> Result<()> {
    let mut open = false;
    for a in args {
        match a.as_str() {
            "--open" => open = true,
            other => bail!("unknown argument {other:?} (usage: gmail-auth [--open])"),
        }
    }
    let settings = config::load_or_init(&config::config_dir()?)?;
    let path = mail::client_secret_path(&settings)
        .context("[mail] client_secret_file is not set in settings.toml (path to the Google OAuth client JSON)")?;
    let client = oauth::load_client(&path)?;
    let endpoints = oauth::Endpoints::from_env()?;

    let grant = oauth::authorize(&client, &endpoints, oauth::CONSENT_TIMEOUT, &mut |url| {
        println!("Open this URL and approve access (send-only Gmail + your address):\n\n{url}\n");
        println!("Waiting up to 5 minutes for the redirect on 127.0.0.1 ...");
        if open {
            let _ = std::process::Command::new("xdg-open").arg(url).spawn();
        }
    })?;

    let sender = oauth::userinfo_email(&oauth::http_agent(), &endpoints, &grant.access_token)?;
    secrets::set_gmail_refresh_token(&grant.refresh_token)?;
    println!("authorized as {sender}; refresh token stored in the keyring (service omafinances, user gmail-refresh-token)");
    println!("{sender} is always an allowed report recipient; add others under [reports] allowed_recipients in settings.toml");
    Ok(())
}

pub fn gmail_status(args: &[String]) -> Result<()> {
    if let Some(extra) = args.first() {
        bail!("unknown argument {extra:?} (usage: gmail-status)");
    }
    let st = mail::status()?;
    println!("client_configured: {}", if st.client_configured { "yes" } else { "no" });
    println!("authenticated: {}", if st.authenticated { "yes" } else { "no" });
    println!("sender: {}", st.sender.as_deref().unwrap_or("-"));
    if !st.authenticated {
        println!("hint: run `omafinances gmail-auth`");
    }
    Ok(())
}
