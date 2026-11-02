use crate::ai::settings::{SettingsView, view as settings_view};
use crate::db::{self, ConnConfig, DatabaseList, Kind};
use crate::error::AppResult;
use crate::schema::Schema;
use crate::state::{AppState, Target};
use crate::store::secrets;
use serde::Serialize;
use std::path::Path;
use tauri::State;

const DEMO_ID: &str = "demo";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitView {
    connections: Vec<ConnConfig>,
    settings: SettingsView,
}

fn demo_connection(path: &Path) -> ConnConfig {
    ConnConfig {
        id: DEMO_ID.into(),
        name: "Demo store (SQLite)".into(),
        kind: Kind::Sqlite,
        host: String::new(),
        port: None,
        database: String::new(),
        user: String::new(),
        file: path.to_string_lossy().into_owned(),
        ssl: false,
        trust_cert: false,
        production: false,
    }
}

/// Startup: saved connections and settings. Adds the demo database on first launch.
#[tauri::command]
pub async fn init(state: State<'_, AppState>) -> AppResult<InitView> {
    let _guard = state.init_lock.lock().await;
    // Read every saved file now, so an unreadable one is reported on the startup screen
    // (with where it was moved) instead of quietly showing an empty list later.
    state.store.scripts()?;
    state.store.skills()?;
    let mut settings = state.store.settings()?;
    if !settings.demo_added {
        db::ensure_demo(&state.demo_path).await?;
        let mut list = state.store.connections()?;
        list.insert(0, demo_connection(&state.demo_path));
        state.store.save_connections(&list)?;
        settings.demo_added = true;
        state.store.save_settings(&settings)?;
    }
    Ok(InitView {
        connections: state.store.connections()?,
        settings: settings_view(&settings),
    })
}

/// Creates (empty id) or updates a connection. `password: None` keeps the saved password.
#[tauri::command]
pub fn save_connection(
    state: State<'_, AppState>,
    mut config: ConnConfig,
    password: Option<String>,
) -> AppResult<ConnConfig> {
    refuse_team(&config.id)?;
    if config.id.is_empty() {
        config.id = uuid::Uuid::new_v4().to_string();
    }
    if let Some(pw) = password {
        secrets::set_password(&config.id, &pw)?;
    }
    let mut list = state.store.connections()?;
    match list.iter_mut().find(|c| c.id == config.id) {
        Some(existing) => *existing = config.clone(),
        None => list.push(config.clone()),
    }
    state.store.save_connections(&list)?;
    // The details may have changed: reconnect on next open.
    state.close(&config.id);
    Ok(config)
}

/// Removes the saved connection and its password. The database itself is not touched.
#[tauri::command]
pub fn delete_connection(state: State<'_, AppState>, id: String) -> AppResult<()> {
    refuse_team(&id)?;
    let list: Vec<ConnConfig> = state
        .store
        .connections()?
        .into_iter()
        .filter(|c| c.id != id)
        .collect();
    state.store.save_connections(&list)?;
    state.close(&id);
    if let Err(e) = state.store.forget_agent(&id) {
        eprintln!("sayql: couldn't remove the agent settings of connection {id}: {e}");
    }
    // The connection is already gone; a keychain failure here only leaves a stale secret.
    if let Err(e) = secrets::delete_password(&id) {
        eprintln!("sayql: couldn't remove the saved password of connection {id}: {e}");
    }
    Ok(())
}

/// Team connections are managed by the team admin on the server.
fn refuse_team(id: &str) -> AppResult<()> {
    if id.starts_with(crate::team::ID_PREFIX) {
        return Err("Your team admin manages this connection on your team's SayQL server.".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn test_connection(config: ConnConfig, password: Option<String>) -> AppResult<String> {
    let password = password.or_else(|| secrets::password(&config.id));
    let pool = db::connect(&config, password).await?;
    let schema = db::introspect(&pool, config.kind).await?;
    Ok(format!(
        "Connected. Found {} tables and views.",
        schema.tables.len()
    ))
}

/// Opens one database of a connection (or reuses it) and returns its schema.
/// `database: None` opens the connection's default database.
#[tauri::command]
pub async fn open_database(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    database: Option<String>,
) -> AppResult<Schema> {
    let target = Target::new(id, database);
    match state.schema(&target) {
        Ok(loaded) => Ok(loaded.schema.clone()),
        Err(_) => {
            let schema = state.load_schema(&target).await?;
            // Note checks that haven't run for a week run now, in the background.
            tauri::async_runtime::spawn(async move {
                use tauri::Manager;
                const WEEK: u64 = 7 * 24 * 3600 * 1000;
                let state = app.state::<AppState>();
                let Ok(pool) = state.target_pool(&target).await else {
                    return;
                };
                let place = state.notes_database(&target);
                let _ = crate::notes::check_all(
                    &state.store,
                    &pool,
                    &target.connection_id,
                    place.as_deref(),
                    Some(WEEK),
                )
                .await;
            });
            Ok(schema)
        }
    }
}

/// Re-reads the schema, e.g. after tables were added.
#[tauri::command]
pub async fn refresh_schema(
    state: State<'_, AppState>,
    id: String,
    database: Option<String>,
) -> AppResult<Schema> {
    state.load_schema(&Target::new(id, database)).await
}

/// The databases on the connection's server; `None` for SQLite.
#[tauri::command]
pub async fn list_databases(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Option<DatabaseList>> {
    let pool = state.target_pool(&Target::default_of(&id)).await?;
    db::list_databases(&pool).await
}
