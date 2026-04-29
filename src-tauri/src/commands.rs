use crate::engine::downloader;
use crate::engine::sniffer::{self, DetectedStream, SharedSniffer, SnifferState};
use crate::engine::types::*;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub async fn analyze_url(
    app: AppHandle,
    sniffer: State<'_, Arc<SnifferState>>,
    url: String,
) -> Result<VideoFound, String> {
    // Limpar detecções anteriores
    sniffer.clear();

    let result = downloader::analyze(&url).await;

    // Se encontrou streams, registrar no sniffer também
    if let Ok(ref video) = result {
        for stream in &video.streams {
            sniffer.register(DetectedStream {
                url: stream.url.clone(),
                content_type: Some("application/x-mpegurl".to_string()),
                source: "analyzer".to_string(),
                headers: std::collections::HashMap::new(),
                timestamp: 0,
            });
        }
        let _ = app.emit("stream-detected", video.streams.len());
    }

    result
}

#[tauri::command]
pub async fn start_download(
    _video_id: String,
    _stream_index: usize,
    _output_path: String,
) -> Result<String, String> {
    // TODO: Fase 5 — download paralelo de segmentos + mux
    Ok(format!(
        "Download na fila: {} stream #{} → {}",
        _video_id, _stream_index, _output_path
    ))
}

#[tauri::command]
pub async fn cancel_download(_video_id: String) -> Result<(), String> {
    // TODO: Fase 5
    Ok(())
}

#[tauri::command]
pub async fn get_download_progress(video_id: String) -> Result<DownloadProgress, String> {
    // TODO: Fase 5
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

/// Retorna os streams detectados pelo sniffer
#[tauri::command]
pub async fn get_detected_streams(
    sniffer: State<'_, Arc<SnifferState>>,
) -> Result<Vec<DetectedStream>, String> {
    Ok(sniffer.get_all())
}

/// Limpa as detecções
#[tauri::command]
pub async fn clear_detected_streams(
    sniffer: State<'_, Arc<SnifferState>>,
) -> Result<(), String> {
    sniffer.clear();
    Ok(())
}

/// Registra um stream detectado pelo content script do browser
#[tauri::command]
pub async fn report_detected_stream(
    app: AppHandle,
    sniffer: State<'_, Arc<SnifferState>>,
    url: String,
    source: String,
) -> Result<bool, String> {
    let stream = DetectedStream {
        url,
        content_type: None,
        source,
        headers: std::collections::HashMap::new(),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64,
    };

    let is_new = sniffer.register(stream);
    if is_new {
        let count = sniffer.get_all().len();
        let _ = app.emit("stream-detected", count);
    }

    Ok(is_new)
}
