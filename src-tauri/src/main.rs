// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // A helper mode (reading a PDF for the app) exits here, before Tauri, the single-instance
    // check or deep links start.
    if let Some(code) = sayql_lib::helper_mode() {
        std::process::exit(code);
    }
    sayql_lib::run()
}
