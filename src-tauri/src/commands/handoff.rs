//! Receives `sayql://` handoffs from AI apps (see src/handoff.rs), team invite links and the
//! end of a company sign-in, and passes the valid ones to the window, which shows them for the
//! person to act on.

use crate::handoff::{self, Handoff};
use crate::state::AppState;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

/// Handoffs the window hasn't picked up yet: a link can arrive before the UI has loaded.
#[derive(Default)]
pub struct PendingHandoffs(Mutex<Vec<Handoff>>);

/// Checks each link and queues the valid ones; links without a fresh token are dropped.
pub fn receive(app: &AppHandle, links: impl IntoIterator<Item = String>) {
    let state = app.state::<AppState>();
    let mut any = false;
    for link in links {
        if let Some(sso) = handoff::parse_team_sso(&link) {
            match sso {
                Ok(h) if super::team::sso_waiting(app) => {
                    crate::state::lock(&app.state::<PendingHandoffs>().0).push(h);
                    any = true;
                }
                Ok(_) => eprintln!("sayql: ignored a team sign-in link SayQL didn't start"),
                Err(_) => eprintln!("sayql: ignored an invalid team sign-in link"),
            }
            continue;
        }
        if let Some(join) = handoff::parse_team_join(&link) {
            match join {
                Ok(h) => {
                    crate::state::lock(&app.state::<PendingHandoffs>().0).push(h);
                    any = true;
                }
                Err(_) => eprintln!("sayql: ignored an invalid team invite link"),
            }
            continue;
        }
        match handoff::parse_url(&link) {
            Ok((h, token)) if handoff::redeem_token(state.store.dir(), &token) => {
                crate::state::lock(&app.state::<PendingHandoffs>().0).push(h);
                any = true;
            }
            // The link's contents aren't logged: it's untrusted and could hold anything.
            Ok(_) => eprintln!("sayql: ignored a sayql:// link that didn't come from sayql-mcp"),
            Err(_) => eprintln!("sayql: ignored an invalid sayql:// link"),
        }
    }
    if any {
        let _ = app.emit("handoff://new", ());
        focus_main(app);
    }
}

/// Brings the main window to the front. While the splash is up, the window stays hidden: it is
/// shown and focused as soon as its first screen is ready (`super::app::finish_startup`).
pub fn focus_main(app: &AppHandle) {
    if !super::app::started() {
        return;
    }
    if let Some(w) = app.get_webview_window(super::app::MAIN_WINDOW) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// The handoffs waiting for the window, oldest first.
#[tauri::command]
#[specta::specta]
pub fn take_handoffs(pending: State<'_, PendingHandoffs>) -> Vec<Handoff> {
    std::mem::take(&mut *crate::state::lock(&pending.0))
}
