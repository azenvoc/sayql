use crate::error::AppResult;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// The event `install_update` sends while it downloads, with an [`UpdateProgress`].
pub const EVENT: &str = "update://progress";

#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    version: String,
    /// The version running now, to show next to the new one.
    current_version: String,
    /// The release date as `YYYY-MM-DD`, when the feed has one.
    date: Option<String>,
    /// The release's changes, in Markdown (the version's CHANGELOG.md section).
    notes: Option<String>,
}

/// How much of the update has downloaded. `total` is missing when the server doesn't say.
#[derive(Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    received: u64,
    total: Option<u64>,
    /// The download is done and the update is being installed.
    installing: bool,
}

/// A newer release on the update feed, if there is one. Being offline or a feed with no
/// release yet reads as "no update". Development builds never check.
#[tauri::command]
#[specta::specta]
pub async fn check_update(app: AppHandle) -> Option<UpdateInfo> {
    if cfg!(debug_assertions) {
        return None;
    }
    let update = app.updater().ok()?.check().await.ok()??;
    let date = update
        .raw_json
        .get("pub_date")
        .and_then(|d| d.as_str())
        .and_then(|d| d.split('T').next())
        .map(str::to_string);
    Some(UpdateInfo {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        date,
        notes: update.body.clone(),
    })
}

/// Downloads the newer release, installs it once its signature checks out against the
/// updater key in tauri.conf.json, and restarts into it. Progress arrives as `update://progress`
/// events.
#[tauri::command]
#[specta::specta]
pub async fn install_update(app: AppHandle) -> AppResult<()> {
    let update = app
        .updater()
        .map_err(|e| format!("Updates aren't available: {e}"))?
        .check()
        .await
        .map_err(|e| format!("Could not check for updates: {e}"))?
        .ok_or("SayQL is already up to date.")?;
    let received = AtomicU64::new(0);
    let mut last = Instant::now() - Duration::from_secs(1);
    update
        .download_and_install(
            |chunk, total| {
                let received = received.fetch_add(chunk as u64, Ordering::Relaxed) + chunk as u64;
                // About 10 updates a second is plenty for a progress bar.
                if last.elapsed() >= Duration::from_millis(100) {
                    last = Instant::now();
                    let progress = UpdateProgress {
                        received,
                        total,
                        installing: false,
                    };
                    let _ = app.emit(EVENT, progress);
                }
            },
            || {
                let received = received.load(Ordering::Relaxed);
                let progress = UpdateProgress {
                    received,
                    total: Some(received),
                    installing: true,
                };
                let _ = app.emit(EVENT, progress);
            },
        )
        .await
        .map_err(|e| format!("The update could not be installed: {e}"))?;
    app.restart();
}
