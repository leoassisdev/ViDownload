use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

use super::m3u8;
use super::types::*;

/// Estado de um download ativo
pub struct ActiveDownload {
    pub video_id: String,
    pub stream: StreamInfo,
    pub output_path: PathBuf,
    pub segments_done: AtomicU64,
    pub bytes_downloaded: AtomicU64,
    pub cancelled: AtomicBool,
    pub paused: AtomicBool,
    pub state: Mutex<DownloadState>,
    pub started_at: std::time::Instant,
}

/// Gerenciador de downloads
pub struct DownloadManager {
    pub downloads: Mutex<HashMap<String, Arc<ActiveDownload>>>,
    pub max_parallel: usize,
}

impl DownloadManager {
    pub fn new() -> Self {
        Self {
            downloads: Mutex::new(HashMap::new()),
            max_parallel: 4,
        }
    }
}

pub type SharedManager = Arc<DownloadManager>;

pub fn create_manager() -> SharedManager {
    Arc::new(DownloadManager::new())
}

/// Analisa uma URL: busca a página/m3u8, detecta streams, retorna qualidades disponíveis
pub async fn analyze(url: &str) -> Result<VideoFound, String> {
    let client = build_client()?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Erro ao acessar URL: {}", e))?;

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    let body = response
        .text()
        .await
        .map_err(|e| format!("Erro ao ler resposta: {}", e))?;

    if m3u8::is_m3u8(&body) {
        return analyze_m3u8(&client, url, &body).await;
    }

    if content_type.contains("html") {
        return analyze_html_page(&client, url, &body).await;
    }

    Err("Nenhum stream HLS encontrado nesta URL".to_string())
}

/// Baixa todos os segmentos de um stream em paralelo
pub async fn download_stream(
    active: Arc<ActiveDownload>,
    parallel: usize,
) -> Result<PathBuf, String> {
    let client = build_client()?;
    let segments = active.stream.segments.clone();
    let total = segments.len() as u64;

    if total == 0 {
        return Err("Nenhum segmento para baixar".to_string());
    }

    *active.state.lock().await = DownloadState::Downloading;

    // Criar diretório temporário para segmentos
    let temp_dir = active.output_path.with_extension("parts");
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|e| format!("Erro ao criar diretório: {}", e))?;

    // Baixar init segment se existir (fMP4)
    if let Some(ref init_url) = active.stream.init_segment_url {
        let init_path = temp_dir.join("init.mp4");
        download_segment_with_retry(&client, init_url, &init_path, &active, None).await?;
    }

    // Download paralelo com semáforo
    let semaphore = Arc::new(tokio::sync::Semaphore::new(parallel));
    let mut handles = Vec::new();

    for (i, segment) in segments.iter().enumerate() {
        if active.cancelled.load(Ordering::Relaxed) {
            break;
        }

        // Esperar enquanto pausado
        while active.paused.load(Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            if active.cancelled.load(Ordering::Relaxed) {
                break;
            }
        }

        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let seg = segment.clone();
        let seg_path = temp_dir.join(format!("seg_{:06}.ts", i));
        let client = client.clone();
        let active = active.clone();

        let handle = tokio::spawn(async move {
            let result = download_segment_with_retry(
                &client,
                &seg.url,
                &seg_path,
                &active,
                seg.byte_range.as_ref(),
            )
            .await;

            drop(permit);

            if result.is_ok() {
                active.segments_done.fetch_add(1, Ordering::Relaxed);
            }

            result
        });

        handles.push(handle);
    }

    // Esperar todos terminarem
    let mut errors = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => errors.push(e),
            Err(e) => errors.push(format!("Task falhou: {}", e)),
        }
    }

    if active.cancelled.load(Ordering::Relaxed) {
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        *active.state.lock().await = DownloadState::Cancelled;
        return Err("Download cancelado".to_string());
    }

    if !errors.is_empty() {
        *active.state.lock().await = DownloadState::Error(errors[0].clone());
        return Err(errors[0].clone());
    }

    // Concatenar segmentos no arquivo final
    *active.state.lock().await = DownloadState::Muxing;
    concatenate_segments(&temp_dir, &active.output_path, total, active.stream.init_segment_url.is_some())
        .await?;

    // Limpar temp
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;

    *active.state.lock().await = DownloadState::Done;
    Ok(active.output_path.clone())
}

/// Baixa um segmento com retry e backoff exponencial
async fn download_segment_with_retry(
    client: &reqwest::Client,
    url: &str,
    path: &PathBuf,
    active: &ActiveDownload,
    byte_range: Option<&ByteRange>,
) -> Result<(), String> {
    let max_retries = 6;
    let mut attempt = 0;

    loop {
        if active.cancelled.load(Ordering::Relaxed) {
            return Err("Cancelado".to_string());
        }

        match download_segment(client, url, path, byte_range).await {
            Ok(bytes) => {
                active.bytes_downloaded.fetch_add(bytes, Ordering::Relaxed);
                return Ok(());
            }
            Err(e) => {
                attempt += 1;
                if attempt >= max_retries {
                    return Err(format!("Falhou após {} tentativas: {}", max_retries, e));
                }
                // Backoff exponencial: 200ms, 400ms, 800ms, 1600ms, 3200ms
                let delay = std::time::Duration::from_millis(200 * (1 << attempt));
                tokio::time::sleep(delay).await;
            }
        }
    }
}

/// Baixa um único segmento
async fn download_segment(
    client: &reqwest::Client,
    url: &str,
    path: &PathBuf,
    byte_range: Option<&ByteRange>,
) -> Result<u64, String> {
    let mut request = client.get(url);

    if let Some(br) = byte_range {
        let range = format!("bytes={}-{}", br.offset, br.offset + br.length - 1);
        request = request.header("Range", range);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("Erro HTTP: {}", e))?;

    if !response.status().is_success() && response.status().as_u16() != 206 {
        return Err(format!("HTTP {}", response.status()));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Erro ao ler: {}", e))?;

    let size = bytes.len() as u64;
    tokio::fs::write(path, &bytes)
        .await
        .map_err(|e| format!("Erro ao salvar: {}", e))?;

    Ok(size)
}

/// Concatena todos os segmentos em um arquivo final
async fn concatenate_segments(
    temp_dir: &PathBuf,
    output: &PathBuf,
    total: u64,
    has_init: bool,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    let mut file = tokio::fs::File::create(output)
        .await
        .map_err(|e| format!("Erro ao criar arquivo: {}", e))?;

    // Escrever init segment primeiro se existir
    if has_init {
        let init_path = temp_dir.join("init.mp4");
        if init_path.exists() {
            let data = tokio::fs::read(&init_path)
                .await
                .map_err(|e| format!("Erro ao ler init: {}", e))?;
            file.write_all(&data)
                .await
                .map_err(|e| format!("Erro ao escrever init: {}", e))?;
        }
    }

    // Concatenar segmentos em ordem
    for i in 0..total {
        let seg_path = temp_dir.join(format!("seg_{:06}.ts", i));
        if seg_path.exists() {
            let data = tokio::fs::read(&seg_path)
                .await
                .map_err(|e| format!("Erro ao ler segmento {}: {}", i, e))?;
            file.write_all(&data)
                .await
                .map_err(|e| format!("Erro ao escrever segmento {}: {}", i, e))?;
        }
    }

    file.flush()
        .await
        .map_err(|e| format!("Erro ao finalizar: {}", e))?;

    Ok(())
}

async fn analyze_m3u8(
    client: &reqwest::Client,
    url: &str,
    content: &str,
) -> Result<VideoFound, String> {
    let video_id = generate_id();

    if m3u8::is_master_playlist(content) {
        let mut streams = m3u8::parse_master_playlist(content, url);

        for stream in &mut streams {
            if let Ok(resp) = client.get(&stream.url).send().await {
                if let Ok(body) = resp.text().await {
                    let result = m3u8::parse_media_playlist(&body, &stream.url);
                    stream.segments = result.segments;
                    stream.is_encrypted = result.is_encrypted;
                    stream.is_live = result.is_live;
                    stream.init_segment_url = result.init_segment_url;
                    stream.total_duration = result.total_duration;
                    stream.target_duration = result.target_duration;
                    stream.media_sequence = result.media_sequence;
                }
            }
        }

        Ok(VideoFound {
            id: video_id,
            page_url: url.to_string(),
            streams,
            title: None,
            best_quality_index: 0,
        })
    } else {
        let result = m3u8::parse_media_playlist(content, url);

        let stream = StreamInfo {
            url: url.to_string(),
            quality: "Original".to_string(),
            bandwidth: 0,
            resolution: None,
            codecs: None,
            segments: result.segments,
            is_encrypted: result.is_encrypted,
            is_live: result.is_live,
            init_segment_url: result.init_segment_url,
            total_duration: result.total_duration,
            target_duration: result.target_duration,
            media_sequence: result.media_sequence,
        };

        Ok(VideoFound {
            id: video_id,
            page_url: url.to_string(),
            streams: vec![stream],
            title: None,
            best_quality_index: 0,
        })
    }
}

async fn analyze_html_page(
    client: &reqwest::Client,
    page_url: &str,
    html: &str,
) -> Result<VideoFound, String> {
    let m3u8_urls = extract_m3u8_urls(html, page_url);

    if m3u8_urls.is_empty() {
        return Err("Nenhum stream HLS encontrado na página".to_string());
    }

    let video_id = generate_id();
    let mut all_streams: Vec<StreamInfo> = Vec::new();

    for m3u8_url in &m3u8_urls {
        let resp = client.get(m3u8_url).send().await;
        if let Ok(resp) = resp {
            if let Ok(body) = resp.text().await {
                if m3u8::is_m3u8(&body) {
                    if m3u8::is_master_playlist(&body) {
                        let streams = m3u8::parse_master_playlist(&body, m3u8_url);
                        all_streams.extend(streams);
                    } else {
                        let result = m3u8::parse_media_playlist(&body, m3u8_url);
                        all_streams.push(StreamInfo {
                            url: m3u8_url.clone(),
                            quality: "Original".to_string(),
                            bandwidth: 0,
                            resolution: None,
                            codecs: None,
                            segments: result.segments,
                            is_encrypted: result.is_encrypted,
                            is_live: result.is_live,
                            init_segment_url: result.init_segment_url,
                            total_duration: result.total_duration,
                            target_duration: result.target_duration,
                            media_sequence: result.media_sequence,
                        });
                    }
                }
            }
        }
    }

    if all_streams.is_empty() {
        return Err("Encontrou URLs m3u8 mas não conseguiu parsear nenhum stream".to_string());
    }

    all_streams.sort_by(|a, b| b.bandwidth.cmp(&a.bandwidth));
    let title = extract_title(html);

    Ok(VideoFound {
        id: video_id,
        page_url: page_url.to_string(),
        streams: all_streams,
        title,
        best_quality_index: 0,
    })
}

fn extract_m3u8_urls(html: &str, base_url: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let patterns = [
        r#"["']([^"']*\.m3u8[^"']*?)["']"#,
        r#"(?:src|href)\s*=\s*["']([^"']*\.m3u8[^"']*?)["']"#,
    ];

    for pattern in &patterns {
        if let Ok(re) = regex_lite::Regex::new(pattern) {
            for cap in re.captures_iter(html) {
                if let Some(url_match) = cap.get(1) {
                    let url = url_match.as_str().to_string();
                    let resolved = if url.starts_with("http") {
                        url
                    } else if let Ok(base) = url::Url::parse(base_url) {
                        base.join(&url).map(|u| u.to_string()).unwrap_or(url)
                    } else {
                        url
                    };

                    if seen.insert(resolved.clone()) {
                        urls.push(resolved);
                    }
                }
            }
        }
    }

    urls
}

fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let start = lower.find("<title>")?;
    let end = lower[start..].find("</title>")?;
    let title = &html[start + 7..start + end];
    let title = title.trim();
    if title.is_empty() {
        None
    } else {
        Some(html_escape_decode(title))
    }
}

fn html_escape_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Erro ao criar cliente HTTP: {}", e))
}

fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("vid_{}", ts)
}
