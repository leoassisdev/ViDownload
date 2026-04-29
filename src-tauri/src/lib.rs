mod browser;
mod commands;
mod engine;

use browser::BrowserState;
use commands::{
    analyze_url, cancel_download, clear_detected_streams, get_detected_streams,
    get_download_progress, report_detected_stream, start_download,
};
use browser::{
    browser_back, browser_forward, browser_get_url, browser_navigate, browser_poll_detected,
    browser_reload, close_browser, open_browser,
};
use engine::sniffer;
use std::sync::Arc;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let sniffer_state = Arc::new(sniffer::SnifferState::default());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(BrowserState::default())
        .manage(sniffer_state)
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
            // Sniffer / detecção
            get_detected_streams,
            clear_detected_streams,
            report_detected_stream,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
