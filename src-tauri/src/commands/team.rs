//! Settings → Team: signing in to a company's SayQL server, and its shared connections.
//! Signed out, none of these make a network request.

use crate::db::ConnConfig;
use crate::error::AppResult;
use crate::state::AppState;
use crate::store::secrets;
use crate::team::ai::AiStatus;
use crate::team::{self, TeamAccount, TeamConnection, TeamStatus};
use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, State};

/// How long SayQL waits for the browser to come back from the company's sign-in page.
const SSO_WAIT: Duration = Duration::from_secs(10 * 60);

/// A sign-in with the company's identity provider that SayQL started and is waiting on.
struct SsoStarted {
    server: String,
    verifier: String,
    at: Instant,
}

#[derive(Default)]
pub struct PendingSso(Mutex<Option<SsoStarted>>);

/// True while a sign-in SayQL started is waiting for the browser.
pub fn sso_waiting(app: &AppHandle) -> bool {
    crate::state::lock(&app.state::<PendingSso>().0)
        .as_ref()
        .is_some_and(|s| s.at.elapsed() < SSO_WAIT)
}

/// A team connection as the navigator shows it: no host, user or password, ever.
#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TeamConnectionView {
    #[serde(flatten)]
    config: ConnConfig,
    team: TeamInfo,
}

#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
struct TeamInfo {
    /// The database's name on the server, for display.
    database: String,
    notes: String,
}

fn view(c: &TeamConnection) -> Option<TeamConnectionView> {
    let kind = c.kind()?;
    Some(TeamConnectionView {
        config: ConnConfig {
            id: c.local_id(),
            name: c.name.clone(),
            kind,
            host: String::new(),
            port: None,
            // Empty: a team connection has one database, chosen by the server.
            database: String::new(),
            user: String::new(),
            file: String::new(),
            ssl: false,
            trust_cert: false,
            production: false,
        },
        team: TeamInfo {
            database: c.database.clone(),
            notes: c.notes.clone(),
        },
    })
}

/// Redeems an invite code, keeps the token in the keychain and the server in settings.
#[tauri::command]
#[specta::specta]
pub async fn team_sign_in(
    state: State<'_, AppState>,
    server: String,
    code: String,
) -> AppResult<TeamAccount> {
    let server = team::clean_server(&server)?;
    let code = team::clean_code(&code)?;
    let (token, account) = team::redeem(&state.http, &server, &code, &team::device_name()).await?;
    keep_sign_in(&state, &token, &account)?;
    Ok(account)
}

fn keep_sign_in(state: &AppState, token: &str, account: &TeamAccount) -> AppResult<()> {
    secrets::set_team_token(token)?;
    let mut settings = state.store.settings()?;
    settings.team = Some(account.clone());
    state.store.save_settings(&settings)?;
    state.forget_team();
    Ok(())
}

/// "Sign in with your company": opens the team server's sign-in page in the browser, which
/// sends the person to the company's identity provider and back to SayQL (`sayql://team-sso`).
#[tauri::command]
#[specta::specta]
pub async fn team_sso_start(
    state: State<'_, AppState>,
    pending: State<'_, PendingSso>,
    server: String,
) -> AppResult<()> {
    let server = team::clean_server(&server)?;
    if !team::sso_info(&state.http, &server).await?.enabled {
        return Err(
            "Your team's SayQL server doesn't have sign-in with your company turned on. Use an invite code from your team admin."
                .into(),
        );
    }
    let (verifier, challenge) = team::pkce_pair();
    let url = team::sso_start_url(&server, &challenge, &team::device_name())?;
    *crate::state::lock(&pending.0) = Some(SsoStarted {
        server,
        verifier,
        at: Instant::now(),
    });
    open::that_detached(&url)
        .map_err(|e| format!("Couldn't open your browser: {e}. Open this page yourself: {url}"))?;
    Ok(())
}

/// The browser came back with the server's one-time code: swap it for a sign-in. The code
/// goes to the server SayQL started the sign-in with, whatever the link said.
#[tauri::command]
#[specta::specta]
pub async fn team_sso_finish(
    state: State<'_, AppState>,
    pending: State<'_, PendingSso>,
    code: String,
) -> AppResult<TeamAccount> {
    let code = team::clean_sso_code(&code)?;
    let started = crate::state::lock(&pending.0)
        .take()
        .filter(|s| s.at.elapsed() < SSO_WAIT)
        .ok_or("This sign-in took too long or wasn't started here. Sign in again from SayQL.")?;
    let (token, account) = team::sso_redeem(
        &state.http,
        &started.server,
        &code,
        &started.verifier,
        &team::device_name(),
    )
    .await?;
    keep_sign_in(&state, &token, &account)?;
    Ok(account)
}

/// Who is signed in, with the server's licence; `None` when signed out (without asking anyone).
#[tauri::command]
#[specta::specta]
pub async fn team_status(state: State<'_, AppState>) -> AppResult<Option<TeamStatus>> {
    let Some(client) = state.team_client()? else {
        return Ok(None);
    };
    let (user, server) = client.status().await?;
    state.knowledge.set_server(&server);
    let mut settings = state.store.settings()?;
    let Some(saved) = settings.team.clone() else {
        return Ok(None);
    };
    let account = TeamAccount {
        email: user.email,
        name: user.name,
        role: user.role,
        ..saved.clone()
    };
    if account != saved {
        settings.team = Some(account.clone());
        state.store.save_settings(&settings)?;
    }
    // With /me: team AI, so Settings shows it and AI requests on team connections use it.
    let ai = state.refresh_team_ai().await.ok().flatten();
    Ok(Some(TeamStatus {
        account,
        server,
        ai,
    }))
}

/// Signs out: the server revokes the token (when it can be reached), and this computer forgets it.
#[tauri::command]
#[specta::specta]
pub async fn team_sign_out(state: State<'_, AppState>) -> AppResult<()> {
    let client = state.team_client().ok().flatten();
    if let Some(client) = client
        && let Err(e) = client.sign_out().await
    {
        // Signing out here still works offline; the admin can revoke the token on the server.
        eprintln!("sayql: the team server didn't confirm the sign-out: {e}");
    }
    team::forget_sign_in(&state.store);
    state.forget_team();
    Ok(())
}

/// The team's shared connections; empty when signed out, without asking anyone.
#[tauri::command]
#[specta::specta]
pub async fn team_connections(state: State<'_, AppState>) -> AppResult<Vec<TeamConnectionView>> {
    let list = match state.team_client()? {
        None => Vec::new(),
        Some(client) => client.connections().await?,
    };
    let views = list.iter().filter_map(view).collect();
    state.set_team_connections(list);
    // Team AI applies to these connections; an older server without it simply has none.
    if state.refresh_team_ai().await.is_err() {
        state.set_team_ai(None);
    }
    Ok(views)
}

/// Team AI's state, asked again (Settings opening); `None` when signed out, without asking anyone.
#[tauri::command]
#[specta::specta]
pub async fn team_ai(state: State<'_, AppState>) -> AppResult<Option<AiStatus>> {
    state.refresh_team_ai().await
}
