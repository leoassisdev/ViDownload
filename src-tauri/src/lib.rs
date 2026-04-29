mod commands;
mod engine;

use commands::{analyze_url, cancel_download, get_download_progress, start_download};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            analyze_url,
            start_download,
            cancel_download,
            get_download_progress,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
