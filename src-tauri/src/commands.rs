use crate::config::{self, SiteCredential};
use crate::engine::downloader::{self, ActiveDownload, SharedManager};
use crate::engine::extractors;
use crate::engine::ffmpeg;
use crate::engine::sniffer::{DetectedStream, SnifferState};
use crate::engine::types::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;

/// Diretório de config do app (credenciais, perfil do Chrome).
fn app_config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|e| format!("Erro ao resolver dir de config: {}", e))
}

#[tauri::command]
pub async fn analyze_url(
    app: AppHandle,
    sniffer: State<'_, Arc<SnifferState>>,
    url: String,
) -> Result<VideoFound, String> {
    sniffer.clear();

    // Áreas de membros (MemberKit): exigem login e servem o vídeo por um player
    // externo embutido (Vimeo). Fazemos login no Chrome, achamos o iframe do
    // player e extraímos o stream direto.
    if url.contains("memberkit.com.br") {
        let video = analyze_memberkit(&app, &url).await?;
        for stream in &video.streams {
            sniffer.register(DetectedStream {
                url: stream.url.clone(),
                content_type: Some("application/x-mpegurl".to_string()),
                source: "memberkit".to_string(),
                headers: std::collections::HashMap::new(),
                timestamp: 0,
            });
        }
        let _ = app.emit("stream-detected", video.streams.len());
        return Ok(video);
    }

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

/// Fluxo MemberKit → Vimeo: Chrome logado lê o iframe do player, extrai o config
/// do Vimeo (com Referer da aula) e analisa o master m3u8 (áudio+vídeo separados).
async fn analyze_memberkit(app: &AppHandle, url: &str) -> Result<VideoFound, String> {
    let cfg = app_config_dir(app)?;
    let host = config::host_of(url).unwrap_or_default();
    let (email, password) = config::find_for_host(&cfg, &host)
        .map(|c| (c.email, c.password))
        .unzip();

    let profile = cfg.join("chrome-profile");
    let frames =
        crate::chrome::extract_iframes(&profile, url, email.as_deref(), password.as_deref())
            .await?;

    let video_id = frames
        .iter()
        .find_map(|f| extractors::vimeo_id_from_url(f))
        .ok_or_else(|| {
            "Nenhum vídeo Vimeo encontrado nesta aula. (Ela usa outro player?)".to_string()
        })?;

    let extracted = extractors::extract_vimeo(&video_id, Some(url))
        .await
        .ok_or_else(|| {
            "Não foi possível extrair o stream do Vimeo (pode estar protegido por DRM)."
                .to_string()
        })?;

    let mut video = downloader::analyze(&extracted.m3u8_url).await?;
    if let Some(title) = extracted.title {
        video.title = Some(title);
    }
    video.page_url = url.to_string();
    Ok(video)
}

#[tauri::command]
pub fn check_ffmpeg() -> bool {
    ffmpeg::is_available()
}

#[tauri::command]
pub fn list_site_credentials(app: AppHandle) -> Result<Vec<SiteCredential>, String> {
    let cfg = app_config_dir(&app)?;
    // Não expor a senha ao frontend; devolver mascarada.
    let list = config::load_all(&cfg)
        .into_iter()
        .map(|mut c| {
            if !c.password.is_empty() {
                c.password = "••••••••".to_string();
            }
            c
        })
        .collect();
    Ok(list)
}

#[tauri::command]
pub fn save_site_credentials(
    app: AppHandle,
    host: String,
    email: String,
    password: String,
) -> Result<(), String> {
    let cfg = app_config_dir(&app)?;
    config::upsert(
        &cfg,
        SiteCredential {
            host,
            email,
            password,
        },
    )
}

#[tauri::command]
pub fn remove_site_credentials(app: AppHandle, host: String) -> Result<(), String> {
    let cfg = app_config_dir(&app)?;
    config::remove(&cfg, &host)
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
        // Streams demuxados (ffmpeg) medem progresso em segundos; o denominador
        // é a duração total. Os demais medem em número de segmentos.
        let total = if active.stream.audio_url.is_some() {
            active.stream.total_duration.ceil() as u64
        } else {
            active.stream.segments.len() as u64
        };
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
