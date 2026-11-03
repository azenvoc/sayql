use crate::error::AppResult;
use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    version: String,
    notes: Option<String>,
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
    Some(UpdateInfo {
        version: update.version.clone(),
        notes: update.body.clone(),
    })
}

/// Downloads the newer release, installs it once its signature checks out against the
/// updater key in tauri.conf.json, and restarts into it.
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
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| format!("The update could not be installed: {e}"))?;
    app.restart();
}
