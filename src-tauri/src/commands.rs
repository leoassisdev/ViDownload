use crate::engine::downloader::{self, ActiveDownload, SharedManager};
use crate::engine::sniffer::{DetectedStream, SnifferState};
use crate::engine::types::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

#[tauri::command]
pub async fn analyze_url(
    app: AppHandle,
    sniffer: State<'_, Arc<SnifferState>>,
    url: String,
) -> Result<VideoFound, String> {
    sniffer.clear();

    let result = downloader::analyze(&url).await;

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
    app: AppHandle,
    manager: State<'_, SharedManager>,
    video_id: String,
    stream: StreamInfo,
    output_path: String,
) -> Result<String, String> {
    let path = PathBuf::from(&output_path);

    let active = Arc::new(ActiveDownload {
        video_id: video_id.clone(),
        stream,
        output_path: path,
        segments_done: AtomicU64::new(0),
        bytes_downloaded: AtomicU64::new(0),
        cancelled: AtomicBool::new(false),
        paused: AtomicBool::new(false),
        state: Mutex::new(DownloadState::Analyzing),
        started_at: std::time::Instant::now(),
    });

    manager
        .downloads
        .lock()
        .await
        .insert(video_id.clone(), active.clone());

    let vid = video_id.clone();
    let handle = app.clone();
    let parallel = manager.max_parallel;

    // Iniciar download em background
    tokio::spawn(async move {
        let result = downloader::download_stream(active.clone(), parallel).await;

        // Emitir evento de conclusão
        match result {
            Ok(path) => {
                let _ = handle.emit(
                    "download-complete",
                    serde_json::json!({
                        "video_id": vid,
                        "path": path.to_string_lossy()
                    }),
                );
            }
            Err(e) => {
                let _ = handle.emit(
                    "download-error",
                    serde_json::json!({
                        "video_id": vid,
                        "error": e
                    }),
                );
            }
        }
    });

    Ok(video_id)
}

#[tauri::command]
pub async fn cancel_download(
    manager: State<'_, SharedManager>,
    video_id: String,
) -> Result<(), String> {
    let downloads = manager.downloads.lock().await;
    if let Some(active) = downloads.get(&video_id) {
        active.cancelled.store(true, Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
pub async fn pause_download(
    manager: State<'_, SharedManager>,
    video_id: String,
) -> Result<(), String> {
    let downloads = manager.downloads.lock().await;
    if let Some(active) = downloads.get(&video_id) {
        active.paused.store(true, Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
pub async fn resume_download(
    manager: State<'_, SharedManager>,
    video_id: String,
) -> Result<(), String> {
    let downloads = manager.downloads.lock().await;
    if let Some(active) = downloads.get(&video_id) {
        active.paused.store(false, Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
pub async fn get_download_progress(
    manager: State<'_, SharedManager>,
    video_id: String,
) -> Result<DownloadProgress, String> {
    let downloads = manager.downloads.lock().await;
    if let Some(active) = downloads.get(&video_id) {
        let done = active.segments_done.load(Ordering::Relaxed);
        let total = active.stream.segments.len() as u64;
        let bytes = active.bytes_downloaded.load(Ordering::Relaxed);
        let state = active.state.lock().await.clone();
        let elapsed = active.started_at.elapsed().as_secs_f64();

        let speed = if elapsed > 0.0 {
            (bytes as f64 / elapsed) as u64
        } else {
            0
        };

        let eta = if speed > 0 && done < total {
            let remaining_ratio = (total - done) as f64 / total.max(1) as f64;
            elapsed * remaining_ratio / (1.0 - remaining_ratio).max(0.01)
        } else {
            0.0
        };

        Ok(DownloadProgress {
            video_id,
            state,
            segments_done: done,
            segments_total: total,
            bytes_downloaded: bytes,
            speed_bps: speed,
            eta_seconds: eta,
        })
    } else {
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
}

#[tauri::command]
pub async fn get_detected_streams(
    sniffer: State<'_, Arc<SnifferState>>,
) -> Result<Vec<DetectedStream>, String> {
    Ok(sniffer.get_all())
}

#[tauri::command]
pub async fn clear_detected_streams(
    sniffer: State<'_, Arc<SnifferState>>,
) -> Result<(), String> {
    sniffer.clear();
    Ok(())
}

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
