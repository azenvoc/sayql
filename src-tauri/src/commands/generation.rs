use crate::ai::endpoint::endpoint;
use crate::ai::{self, GenRequest, Task, context::schema_text};
use crate::error::{AppError, AppResult};
use crate::state::{AppState, Target};
use crate::team::ai::source_of;
use tauri::{AppHandle, State};

/// Starts streaming SQL for `question`; results arrive as `ai://` events tagged with `req_id`.
/// `task` says whether to draft (fast model), fix a rejected query or improve one (expert
/// model). `focus_tables` (qualified names picked in the navigator) narrow what a draft sees.
/// `skill` (a saved skill's slug) adds that skill's instruction to the request. Supersedes the generation still running in the same `slot` (one per editor tab).
#[tauri::command]
#[specta::specta]
#[allow(
    clippy::too_many_arguments,
    reason = "a Tauri command takes each webview argument by name"
)]
pub async fn generate_sql(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    database: Option<String>,
    question: String,
    req_id: u64,
    slot: String,
    task: Task,
    focus_tables: Option<Vec<String>>,
    skill: Option<String>,
) -> AppResult<()> {
    let target = Target::new(id.clone(), database);
    // The database said a table or column doesn't exist: the schema may have changed since
    // it was read, so read it again before the fix.
    match &task {
        Task::Fix { error, .. } if crate::notes::says_missing(error) => {
            let _ = state.refresh_now(&target).await;
        }
        _ => state.ensure_fresh(&target).await,
    }
    // What the database says about never-send columns stays out of the prompt. A signed-out
    // team connection has no team rules to add: the person's own list.
    let never_send = match crate::commands::agent::effective_access(&state, &id) {
        Ok(view) => view.access.never_send,
        Err(_) => state.store.agent_access(&id)?.never_send,
    };
    let live = state.schema(&target)?.for_ai(&never_send);
    let skill = match skill {
        None => None,
        Some(slug) => Some(
            state
                .store
                .skills()?
                .into_iter()
                .find(|s| s.slug == slug)
                .map(|s| s.instruction)
                .ok_or_else(|| {
                    AppError::User(format!(
                        "There's no skill called /{slug}. Create it in Settings."
                    ))
                })?,
        ),
    };
    let settings = state.store.settings()?;
    // A team connection with team AI goes through the team server; everything else as before.
    let own = endpoint(&settings);
    let (endpoint, model) = match state.team_ai_route(&id, source_of(&task), own.is_ok()) {
        Some((route, ai)) => (
            route,
            if task.needs_expert() {
                ai.default_model()
            } else {
                ai.fast_model()
            },
        ),
        None => (
            own?,
            settings
                .provider_settings(&settings.provider)
                .model(task.needs_expert())
                .to_string(),
        ),
    };
    let schema_part = schema_text(
        &live.schema,
        &live.schema_text,
        &question,
        &task,
        focus_tables.as_deref().unwrap_or(&[]),
    );
    // Only the notes about the tables the AI is shown, plus those about no table. On a team
    // connection whose server keeps shared notes, the team's; else the ones saved here.
    let tables = crate::schema::anchors::tables_in(&schema_part, &live.schema);
    let notes =
        crate::store::agent::render_notes(&state.prompt_notes(&target, &tables).await, false);
    let request = GenRequest {
        req_id,
        endpoint,
        model,
        dialect: live.schema.dialect.clone(),
        schema_text: schema_part + &notes,
        question,
        task,
        skill,
    };
    let handle = tauri::async_runtime::spawn(ai::stream_sql(app, state.http.clone(), request));
    state.replace_generation(&slot, handle);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn cancel_generation(state: State<'_, AppState>, slot: String) {
    state.cancel_generation(&slot);
}
