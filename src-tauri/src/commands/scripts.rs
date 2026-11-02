use crate::error::AppResult;
use crate::state::AppState;
use crate::store::Script;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

/// Saved scripts, most recently saved first.
#[tauri::command]
pub fn list_scripts(state: State<'_, AppState>) -> AppResult<Vec<Script>> {
    let mut list = state.store.scripts()?;
    list.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
    Ok(list)
}

/// Creates (empty id) or updates a script, and returns it as stored.
#[tauri::command]
pub fn save_script(state: State<'_, AppState>, mut script: Script) -> AppResult<Script> {
    script.name = script.name.trim().to_string();
    if script.name.is_empty() {
        return Err("Give the script a name.".into());
    }
    if script.id.is_empty() {
        script.id = uuid::Uuid::new_v4().to_string();
    }
    script.updated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    state.store.update_scripts(|list| {
        match list.iter_mut().find(|s| s.id == script.id) {
            Some(existing) => *existing = script.clone(),
            None => list.push(script.clone()),
        }
        Ok(())
    })?;
    Ok(script)
}

#[tauri::command]
pub fn delete_script(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.store.update_scripts(|list| {
        list.retain(|s| s.id != id);
        Ok(())
    })
}
