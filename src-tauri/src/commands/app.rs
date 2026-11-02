use serde::Serialize;

/// This copy of SayQL, for pre-filling feedback forms. Nothing else about the computer.
#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    /// `macos`, `windows` or `linux`.
    os: &'static str,
    /// `aarch64` or `x86_64`.
    arch: &'static str,
}

#[tauri::command]
#[specta::specta]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    }
}
