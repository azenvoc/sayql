use crate::error::AppResult;
use crate::state::AppState;
use crate::store::{Skill, skills::validate};
use tauri::State;

/// Saved skills, in the order they were created.
#[tauri::command]
#[specta::specta]
pub fn list_skills(state: State<'_, AppState>) -> AppResult<Vec<Skill>> {
    state.store.skills()
}

/// Creates a skill, or replaces the one called `original` (its slug before an edit).
#[tauri::command]
#[specta::specta]
pub fn save_skill(
    state: State<'_, AppState>,
    skill: Skill,
    original: Option<String>,
) -> AppResult<Skill> {
    let mut list = state.store.skills()?;
    let skill = validate(skill, &list, original.as_deref())?;
    match original.and_then(|o| list.iter().position(|s| s.slug == o)) {
        Some(i) => list[i] = skill.clone(),
        None => list.push(skill.clone()),
    }
    state.store.save_skills(&list)?;
    Ok(skill)
}

#[tauri::command]
#[specta::specta]
pub fn delete_skill(state: State<'_, AppState>, slug: String) -> AppResult<()> {
    let list: Vec<Skill> = state
        .store
        .skills()?
        .into_iter()
        .filter(|s| s.slug != slug)
        .collect();
    state.store.save_skills(&list)
}
