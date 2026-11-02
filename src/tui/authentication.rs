use super::*;
use crate::{auth, credentials};

pub(super) fn start_login(app: &mut App, shared: Arc<Mutex<App>>) {
    if app.signing_out || app.login_cancel.is_some() {
        return;
    }
    app.login_generation += 1;
    let generation = app.login_generation;
    let cancel = CancellationToken::new();
    app.login_cancel = Some(cancel.clone());
    app.is_loading = true;
    app.auth_info_text = Some("Preparing secure browser login… Esc cancels.".to_string());
    if app.view != View::LoginRequired {
        app.view = View::AuthResult;
    }
    let base = app.base_url.clone();
    tokio::spawn(async move {
        let result = login_task(&shared, &base, generation, &cancel).await;
        let mut app = shared.lock().await;
        if app.login_generation != generation || app.base_url != base || cancel.is_cancelled() {
            return;
        }
        app.login_cancel = None;
        app.is_loading = false;
        match result {
            Ok(auth) => {
                app.clear_session();
                app.token = Some(auth.token.clone());
                app.needs_reconnect = true;
                app.fatal_error = None;
                app.auth_info_text = Some(format!(
                    "Logged in as {}. Credentials saved in the OS store.",
                    auth.username.as_deref().unwrap_or("Unknown")
                ));
                if app.view == View::LoginRequired {
                    app.view = View::Main;
                }
                drop(app);
                tokio::spawn(async_fetch_queue(shared));
            }
            Err(error) => app.auth_info_text = Some(format!("Login failed: {error}")),
        }
    });
}

async fn login_task(
    shared: &Arc<Mutex<App>>,
    base: &str,
    generation: u64,
    cancel: &CancellationToken,
) -> anyhow::Result<credentials::Auth> {
    credentials::check_available(base).await?;
    if cancel.is_cancelled() {
        anyhow::bail!("login cancelled");
    }
    let session = auth::LoginSession::bind(base).await?;
    let link = session.authorization_url().to_string();
    {
        let mut app = shared.lock().await;
        if app.login_generation != generation || cancel.is_cancelled() {
            anyhow::bail!("login cancelled");
        }
        app.auth_info_text = Some(format!(
            "Complete authorization in your browser.\nIf it did not open, visit:\n{link}\n\nEsc cancels."
        ));
    }
    let browser_cancel = cancel.clone();
    tokio::task::spawn_blocking(move || {
        if !browser_cancel.is_cancelled() {
            let _ = open::that(link);
        }
    });
    let pending = session.wait(cancel).await?;
    let auth = pending.auth.clone();
    let result = credentials::save(base, auth.clone(), cancel.clone()).await;
    let _ = pending
        .respond(result.is_ok() && !cancel.is_cancelled())
        .await;
    result?;
    Ok(auth)
}

pub(super) fn start_signout(app: &mut App, shared: Arc<Mutex<App>>) {
    if app.signing_out {
        return;
    }
    app.cancel_login();
    app.login_generation += 1;
    app.signing_out = true;
    let token = app.token.clone();
    let base = app.base_url.clone();
    let client = app.client.clone();
    app.clear_session();
    app.is_loading = true;
    app.view = View::AuthResult;
    app.auth_info_text = Some("Removing local credentials…".to_string());
    app.signout_task = Some(tokio::spawn(async move {
        let local = credentials::delete(&base).await;
        let remote = auth::revoke(&client, &base, token.as_deref()).await;
        let mut app = shared.lock().await;
        app.signing_out = false;
        app.is_loading = false;
        match local {
            Ok(()) => {
                let message = match remote {
                    Ok(true) => "Signed out locally and remote token revoked.".to_string(),
                    _ => "Signed out locally. Remote revocation was not confirmed.".to_string(),
                };
                app.notify(message);
                if app.view == View::AuthResult {
                    app.view = View::LoginRequired;
                }
                app.auth_info_text = None;
            }
            Err(error) => {
                app.auth_info_text =
                    Some(format!("Local credentials could not be removed: {error}"))
            }
        }
    }));
}

pub(super) fn show_auth_info(app: &mut App, shared: Arc<Mutex<App>>) {
    app.view = View::AuthResult;
    app.auth_info_text = Some("Reading server-scoped credentials…".to_string());
    let base = app.base_url.clone();
    tokio::spawn(async move {
        let saved = credentials::load(&base).await;
        let mut app = shared.lock().await;
        if app.base_url != base || app.view != View::AuthResult {
            return;
        }
        app.auth_info_text = Some(match saved {
            Ok(Some(auth)) => format!(
                "Server: {base}\nStorage: OS credential store\nUser: {}\nToken: [redacted]",
                auth.username.as_deref().unwrap_or("Unknown")
            ),
            Ok(None) => format!("No saved credentials for {base}."),
            Err(error) => format!("Could not read credentials: {error}"),
        });
    });
}

pub(super) fn switch_server(app: &mut App, shared: Arc<Mutex<App>>) {
    app.cancel_login();
    app.login_generation += 1;
    let generation = app.login_generation;
    app.clear_session();
    app.guild_id = None;
    app.error_message = None;
    let base = app.base_url.clone();
    tokio::spawn(async move {
        let saved = credentials::load(&base).await;
        let mut app = shared.lock().await;
        if app.base_url != base || app.login_generation != generation {
            return;
        }
        match saved {
            Ok(Some(auth)) => {
                app.token = Some(auth.token.clone());
                app.needs_reconnect = true;
                app.notify("Loaded this server's credentials.");
                drop(app);
                tokio::spawn(async_fetch_queue(shared));
            }
            Ok(None) => app.notify("Server changed. Log in to this server via : Account."),
            Err(error) => {
                app.error_message = Some(format!("Server changed; no credentials loaded: {error}"))
            }
        }
    });
}
