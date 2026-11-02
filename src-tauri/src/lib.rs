//! SayQL backend: wires the Tauri app together. Logic lives in the modules below.

mod agent_process;
mod ai;
#[cfg(target_os = "macos")]
mod app_menu;
mod audit;
mod audit_opencode;
mod audit_runner;
mod coding_agents;
mod commands;
mod db;
mod error;
pub mod handoff;
mod knowledge;
pub mod mcp;
mod never_send;
mod notes;
mod pii;
mod readonly;
mod repos;
mod schema;
mod sql_lex;
mod state;
mod store;
mod team;
mod voice;

use commands::{
    agent, app, audit as audits, connections, generation, handoff as handoffs, query, scripts,
    settings, skills, team as teams, updates,
};
use state::AppState;
use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;

/// On macOS, SayQL's menu bar (⌘W closes a tab, ⌘, opens Settings, a Help menu); elsewhere
/// there is no menu bar and the page handles the keys.
#[cfg(target_os = "macos")]
fn with_menu(b: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    b.menu(app_menu::build)
        .on_menu_event(|h, e| app_menu::on_event(h, e.id().as_ref()))
}
#[cfg(not(target_os = "macos"))]
fn with_menu(b: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    b
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    with_menu(tauri::Builder::default())
        // First: a second launch (e.g. from a sayql:// link) hands its link to this one.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            handoffs::focus_main(app)
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // The app's identifier names its keychain item, so a test build keeps its own.
            store::secrets::set_service(&app.config().identifier);
            let state = AppState::new(app.path().app_config_dir()?, app.path().app_data_dir()?)?;
            app.manage(state);
            app.manage(handoffs::PendingHandoffs::default());
            app.manage(teams::PendingSso::default());
            app.manage(audits::AuditRuns::default());
            audits::clean_leftover_runs(app.handle());
            // Installers register sayql:// on Windows and Linux; this covers AppImages and dev builds.
            #[cfg(any(windows, target_os = "linux"))]
            let _ = app.deep_link().register_all();
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |e| {
                handoffs::receive(&handle, e.urls().into_iter().map(String::from))
            });
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                handoffs::receive(app.handle(), urls.into_iter().map(String::from));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app::app_info,
            connections::init,
            connections::save_connection,
            connections::delete_connection,
            connections::test_connection,
            connections::open_database,
            connections::refresh_schema,
            connections::list_databases,
            query::run_query,
            query::cancel_query,
            scripts::list_scripts,
            scripts::save_script,
            scripts::delete_script,
            skills::list_skills,
            skills::save_skill,
            skills::delete_skill,
            generation::generate_sql,
            generation::cancel_generation,
            agent::start_agent,
            agent::stop_agent,
            agent::reply_agent,
            agent::agent_access,
            agent::save_agent_access,
            agent::list_notes,
            agent::list_shared_notes,
            agent::suggest_note,
            agent::save_note,
            agent::delete_note,
            agent::check_note,
            agent::confirm_note,
            agent::check_notes,
            agent::list_note_suggestions,
            agent::accept_note_suggestion,
            agent::dismiss_note_suggestion,
            settings::save_agent_settings,
            settings::choose_model,
            handoffs::take_handoffs,
            settings::save_settings,
            settings::list_models,
            settings::credits_balance,
            settings::check_claude_plan,
            settings::finish_onboarding,
            settings::mcp_setup,
            settings::installed_coding_agents,
            settings::open_install_link,
            settings::mcp_activity,
            settings::mcp_apps,
            settings::set_mcp_app_blocked,
            settings::approve_mcp_app,
            settings::set_mcp_only_approved,
            settings::clear_mcp_activity,
            settings::my_activity,
            settings::clear_my_activity,
            settings::mcp_audits,
            settings::audit_prompt,
            audits::audit_agents,
            audits::knowledge_sources,
            audits::add_knowledge_folder,
            audits::add_knowledge_repo,
            audits::add_knowledge_files,
            audits::add_knowledge_url,
            audits::remove_knowledge_source,
            audits::refresh_knowledge_source,
            audits::start_audit,
            audits::cancel_audit,
            audits::open_audit_in_terminal,
            audits::audit_model,
            audits::set_audit_model,
            audits::audit_payment,
            audits::set_audit_payment,
            updates::check_update,
            updates::install_update,
            teams::team_sign_in,
            teams::team_sso_start,
            teams::team_sso_finish,
            teams::team_status,
            teams::team_sign_out,
            teams::team_connections,
            teams::team_ai,
            commands::voice::voice_status,
            commands::voice::voice_download,
            commands::voice::voice_cancel_download,
            commands::voice::voice_delete_model,
            commands::voice::voice_start,
            commands::voice::voice_stop,
            commands::voice::voice_cancel,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
