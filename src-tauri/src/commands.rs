use crate::engine::downloader;
use crate::engine::types::*;

#[tauri::command]
pub async fn analyze_url(url: String) -> Result<VideoFound, String> {
    downloader::analyze(&url).await
}

#[tauri::command]
pub async fn start_download(
    video_id: String,
    stream_index: usize,
    output_path: String,
) -> Result<String, String> {
    // TODO: Phase 5 — implement parallel segment download + mux
    Ok(format!(
        "Download queued: {} stream #{} → {}",
        video_id, stream_index, output_path
    ))
}

#[tauri::command]
pub async fn cancel_download(video_id: String) -> Result<(), String> {
    // TODO: Phase 5
    Ok(())
}

#[tauri::command]
pub async fn get_download_progress(video_id: String) -> Result<DownloadProgress, String> {
    // TODO: Phase 5
    Ok(DownloadProgress {
        video_id,
        state: DownloadState::Analyzing,
        segments_done: 0,
        segments_total: 0,
        bytes_downloaded: 0,
        speed_bps: 0,
        eta_seconds: 0.0,
    })
}
