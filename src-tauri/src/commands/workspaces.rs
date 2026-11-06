//! Workspaces and groups. Each command changes the saved layout (see `store::layout`) and returns
//! it, so the UI always shows what's saved.

use crate::db::{self, ConnConfig, DatabaseList};
use crate::error::AppResult;
use crate::state::AppState;
use crate::store::layout::Layout;
use crate::store::secrets;
use tauri::State;

#[tauri::command]
#[specta::specta]
pub fn get_layout(state: State<'_, AppState>) -> AppResult<Layout> {
    state.store.layout()
}

#[tauri::command]
#[specta::specta]
pub fn create_workspace(state: State<'_, AppState>, name: String) -> AppResult<Layout> {
    state
        .store
        .update_layout(|l| l.create_workspace(&name).map(|_| ()))
}

#[tauri::command]
#[specta::specta]
pub fn rename_workspace(state: State<'_, AppState>, id: String, name: String) -> AppResult<Layout> {
    state
        .store
        .update_layout(|l| l.rename_workspace(&id, &name))
}

/// Removes a workspace. Its connections move to `move_to`, or with `None` are deleted the same
/// way as deleting one connection (password, notes, access).
#[tauri::command]
#[specta::specta]
pub fn delete_workspace(
    state: State<'_, AppState>,
    id: String,
    move_to: Option<String>,
) -> AppResult<Layout> {
    if move_to.is_none() {
        // Personal can't be deleted: refuse before any connection goes.
        if id == crate::store::layout::PERSONAL {
            return Err("The Personal workspace can't be deleted.".into());
        }
        for connection in state.store.layout()?.connections_in(&id) {
            if !connection.starts_with(crate::team::ID_PREFIX) {
                super::connections::remove_connection(&state, &connection)?;
            }
        }
    }
    state
        .store
        .update_layout(|l| l.delete_workspace(&id, move_to.as_deref()))
}

#[tauri::command]
#[specta::specta]
pub fn create_group(
    state: State<'_, AppState>,
    workspace_id: String,
    name: String,
) -> AppResult<Layout> {
    state
        .store
        .update_layout(|l| l.create_group(&workspace_id, &name).map(|_| ()))
}

#[tauri::command]
#[specta::specta]
pub fn rename_group(state: State<'_, AppState>, id: String, name: String) -> AppResult<Layout> {
    state.store.update_layout(|l| l.rename_group(&id, &name))
}

/// Removes a group. Its connections and databases stay, with no group.
#[tauri::command]
#[specta::specta]
pub fn delete_group(state: State<'_, AppState>, id: String) -> AppResult<Layout> {
    state.store.update_layout(|l| {
        l.delete_group(&id);
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn move_connection(
    state: State<'_, AppState>,
    id: String,
    workspace_id: String,
    group_id: Option<String>,
) -> AppResult<Layout> {
    state
        .store
        .update_layout(|l| l.move_connection(&id, &workspace_id, group_id.as_deref()))
}

/// Puts one database of a connection in a group, or back with its connection (`None`).
#[tauri::command]
#[specta::specta]
pub fn set_database_group(
    state: State<'_, AppState>,
    id: String,
    database: String,
    group_id: Option<String>,
) -> AppResult<Layout> {
    state
        .store
        .update_layout(|l| l.set_database_group(&id, &database, group_id.as_deref()))
}

#[tauri::command]
#[specta::specta]
pub fn set_team_workspace(state: State<'_, AppState>, workspace_id: String) -> AppResult<Layout> {
    state
        .store
        .update_layout(|l| l.set_team_workspace(&workspace_id))
}

/// Every database a connection's login can open, for choosing which ones it may use. Takes the
/// form's details, since a new connection isn't saved yet.
#[tauri::command]
#[specta::specta]
pub async fn server_databases(
    config: ConnConfig,
    password: Option<String>,
) -> AppResult<Option<DatabaseList>> {
    let password = password.or_else(|| secrets::password(&config.id));
    let pool = db::connect(&config, password).await?;
    db::list_databases(&pool).await
}
