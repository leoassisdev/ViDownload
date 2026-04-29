mod browser;
mod callback_server;
mod commands;
mod engine;

use browser::BrowserState;
use browser::{
    browser_back, browser_forward, browser_get_url, browser_navigate, browser_poll_detected,
    browser_reload, close_browser, open_browser,
};
use commands::{
    analyze_url, cancel_download, clear_detected_streams, get_detected_streams,
    get_download_progress, pause_download, report_detected_stream, resume_download,
    start_download,
};
use engine::sniffer;
use std::sync::Arc;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let sniffer_state = Arc::new(sniffer::SnifferState::default());
    let download_manager = engine::downloader::create_manager();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(BrowserState::default())
        .manage(sniffer_state.clone())
        .manage(download_manager)
        .setup(move |app| {
            // Iniciar servidor de callback local para receber URLs do content script
            let handle = app.handle().clone();
            let sniffer = sniffer_state.clone();
            tokio::spawn(async move {
                callback_server::start(handle, sniffer).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            analyze_url,
            start_download,
            cancel_download,
            pause_download,
            resume_download,
            get_download_progress,
            open_browser,
            close_browser,
            browser_navigate,
            browser_back,
            browser_forward,
            browser_reload,
            browser_get_url,
            browser_poll_detected,
            get_detected_streams,
            clear_detected_streams,
            report_detected_stream,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
