//! SayQL backend: wires the Tauri app together. Logic lives in the modules below.

/// Every Tauri command, in one list. `with_commands!(tauri::generate_handler)` is the app's
/// invoke handler and `with_commands!(tauri_specta::collect_commands)` the source of the
/// generated TypeScript (`src/api/bindings.ts`, see `commands::bindings`), so the two can't
/// differ. Defined before the modules so `commands::bindings` can use it.
macro_rules! with_commands {
    ($($handler:tt)*) => {
        $($handler)*![
            crate::commands::app::app_info,
            crate::commands::app::app_ready,
            crate::commands::connections::init,
            crate::commands::connections::save_connection,
            crate::commands::connections::delete_connection,
            crate::commands::connections::test_connection,
            crate::commands::connections::open_database,
            crate::commands::connections::refresh_schema,
            crate::commands::connections::list_databases,
            crate::commands::query::run_query,
            crate::commands::query::cancel_query,
            crate::commands::scripts::list_scripts,
            crate::commands::scripts::save_script,
            crate::commands::scripts::delete_script,
            crate::commands::skills::list_skills,
            crate::commands::skills::save_skill,
            crate::commands::skills::delete_skill,
            crate::commands::generation::generate_sql,
            crate::commands::generation::cancel_generation,
            crate::commands::agent::start_agent,
            crate::commands::agent::stop_agent,
            crate::commands::agent::reply_agent,
            crate::commands::agent::agent_access,
            crate::commands::agent::save_agent_access,
            crate::commands::agent::list_notes,
            crate::commands::agent::list_shared_notes,
            crate::commands::agent::suggest_note,
            crate::commands::agent::save_note,
            crate::commands::agent::delete_note,
            crate::commands::agent::check_note,
            crate::commands::agent::confirm_note,
            crate::commands::agent::check_notes,
            crate::commands::agent::list_note_suggestions,
            crate::commands::agent::accept_note_suggestion,
            crate::commands::agent::dismiss_note_suggestion,
            crate::commands::settings::save_agent_settings,
            crate::commands::settings::choose_model,
            crate::commands::handoff::take_handoffs,
            crate::commands::settings::save_settings,
            crate::commands::settings::list_models,
            crate::commands::settings::credits_balance,
            crate::commands::settings::check_claude_plan,
            crate::commands::settings::finish_onboarding,
            crate::commands::settings::mcp_setup,
            crate::commands::settings::installed_coding_agents,
            crate::commands::settings::open_install_link,
            crate::commands::settings::mcp_activity,
            crate::commands::settings::mcp_apps,
            crate::commands::settings::set_mcp_app_blocked,
            crate::commands::settings::approve_mcp_app,
            crate::commands::settings::set_mcp_only_approved,
            crate::commands::settings::clear_mcp_activity,
            crate::commands::settings::my_activity,
            crate::commands::settings::clear_my_activity,
            crate::commands::settings::mcp_audits,
            crate::commands::settings::audit_prompt,
            crate::commands::audit::audit_agents,
            crate::commands::audit::knowledge_sources,
            crate::commands::audit::add_knowledge_folder,
            crate::commands::audit::add_knowledge_repo,
            crate::commands::audit::add_knowledge_files,
            crate::commands::audit::add_knowledge_url,
            crate::commands::audit::remove_knowledge_source,
            crate::commands::audit::refresh_knowledge_source,
            crate::commands::audit::start_audit,
            crate::commands::audit::cancel_audit,
            crate::commands::audit::open_audit_in_terminal,
            crate::commands::audit::audit_model,
            crate::commands::audit::set_audit_model,
            crate::commands::audit::audit_payment,
            crate::commands::audit::set_audit_payment,
            crate::commands::updates::check_update,
            crate::commands::updates::install_update,
            crate::commands::team::team_sign_in,
            crate::commands::team::team_sso_start,
            crate::commands::team::team_sso_finish,
            crate::commands::team::team_status,
            crate::commands::team::team_sign_out,
            crate::commands::team::team_connections,
            crate::commands::team::team_ai,
            crate::commands::voice::voice_status,
            crate::commands::voice::voice_download,
            crate::commands::voice::voice_cancel_download,
            crate::commands::voice::voice_delete_model,
            crate::commands::voice::voice_start,
            crate::commands::voice::voice_stop,
            crate::commands::voice::voice_cancel,
        ]
    };
}

mod ai;
#[cfg(target_os = "macos")]
mod app_menu;
mod audit;
mod commands;
mod db;
mod error;
pub mod handoff;
pub mod mcp;
mod notes;
mod safety;
mod schema;
mod state;
mod store;
mod team;
mod voice;

use commands::{audit as audits, handoff as handoffs, team as teams};
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

/// Runs one of SayQL's helper modes when the arguments ask for one, before anything of the app
/// starts: its exit code. `None` means start the app. The only mode reads one PDF's text for
/// the app in a child process (`--extract-pdf`, see `audit::knowledge::pdf`).
pub fn helper_mode() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    match (args.next(), args.next()) {
        (Some(flag), None) if flag == audit::knowledge::pdf::FLAG => {
            Some(audit::knowledge::pdf::reader_main())
        }
        _ => None,
    }
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
            // The main window starts hidden behind the splash; this shows it if the page never
            // says it's ready.
            commands::app::start_splash_timeout(app.handle());
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
        .invoke_handler(with_commands!(tauri::generate_handler))
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
