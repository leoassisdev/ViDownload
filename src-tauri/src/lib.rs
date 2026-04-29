mod browser;
mod commands;
mod engine;

use browser::BrowserState;
use commands::{analyze_url, cancel_download, get_download_progress, start_download};
use browser::{
    browser_back, browser_forward, browser_get_url, browser_navigate, browser_poll_detected,
    browser_reload, close_browser, open_browser,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(BrowserState::default())
        .invoke_handler(tauri::generate_handler![
            // Análise de URL
            analyze_url,
            start_download,
            cancel_download,
            get_download_progress,
            // Browser embutido
            open_browser,
            close_browser,
            browser_navigate,
            browser_back,
            browser_forward,
            browser_reload,
            browser_get_url,
            browser_poll_detected,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
