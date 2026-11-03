use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewWindow};

/// This copy of SayQL, for pre-filling feedback forms. Nothing else about the computer.
#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    /// `macos`, `windows` or `linux`.
    os: &'static str,
    /// `aarch64` or `x86_64`.
    arch: &'static str,
}

#[tauri::command]
#[specta::specta]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    }
}

/// The window SayQL works in. It starts hidden (see tauri.conf.json) while it loads.
pub const MAIN_WINDOW: &str = "main";
/// The splash card shown while the main window loads: `splash.html`, with no scripts.
const SPLASH_WINDOW: &str = "splash";
/// How long the splash may stay up if the main window never says it's ready, for example when
/// its page fails to load. The main window is shown then anyway, so SayQL can't seem stuck.
const SPLASH_TIMEOUT: Duration = Duration::from_secs(10);

/// Whether the main window has been shown. Until then, links that arrive leave it hidden: it
/// is shown, and focused, once its first screen is ready.
static STARTED: AtomicBool = AtomicBool::new(false);

/// True once the main window has been shown after the splash.
pub fn started() -> bool {
    STARTED.load(Ordering::SeqCst)
}

/// Shows and focuses the main window and closes the splash, once. Called when the page has
/// drawn its first screen (`app_ready`), or by the timeout in `start_splash_timeout`.
pub fn finish_startup(app: &AppHandle) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
        let _ = w.show();
        let _ = w.set_focus();
    }
    if let Some(w) = app.get_webview_window(SPLASH_WINDOW) {
        let _ = w.close();
    }
}

/// Starts the splash's safety timeout: after `SPLASH_TIMEOUT` the main window is shown even if
/// the page never called `app_ready`.
pub fn start_splash_timeout(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SPLASH_TIMEOUT).await;
        finish_startup(&app);
    });
}

/// The main window's first screen (the workbench, or the reason SayQL couldn't start) is on
/// the page: shows the window and closes the splash. Calls from any other window are ignored.
#[tauri::command]
#[specta::specta]
pub fn app_ready(window: WebviewWindow) {
    if window.label() == MAIN_WINDOW {
        finish_startup(window.app_handle());
    }
}
