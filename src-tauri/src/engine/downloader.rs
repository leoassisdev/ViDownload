use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::m3u8;
use super::types::*;

/// Shared state for active downloads
pub struct DownloadManager {
    pub videos: HashMap<String, VideoFound>,
    pub progress: HashMap<String, DownloadProgress>,
    pub cancel_flags: HashMap<String, bool>,
}

impl DownloadManager {
    pub fn new() -> Self {
        Self {
            videos: HashMap::new(),
            progress: HashMap::new(),
            cancel_flags: HashMap::new(),
        }
    }
}

pub type SharedManager = Arc<Mutex<DownloadManager>>;

pub fn create_manager() -> SharedManager {
    Arc::new(Mutex::new(DownloadManager::new()))
}

/// Analyze a URL: fetch the page/m3u8, detect streams, return available qualities
pub async fn analyze(url: &str) -> Result<VideoFound, String> {
    let client = build_client()?;

    // First, try fetching the URL directly (might be a direct m3u8 link)
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch URL: {}", e))?;

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    // Check if it's a direct m3u8
    if m3u8::is_m3u8(&body) {
        return analyze_m3u8(&client, url, &body).await;
    }

    // If it's an HTML page, scan for m3u8 URLs embedded in the page
    if content_type.contains("html") {
        return analyze_html_page(&client, url, &body).await;
    }

    Err("No HLS streams found at this URL".to_string())
}

async fn analyze_m3u8(
    client: &reqwest::Client,
    url: &str,
    content: &str,
) -> Result<VideoFound, String> {
    let video_id = generate_id();

    if m3u8::is_master_playlist(content) {
        // Master playlist → multiple quality options
        let mut streams = m3u8::parse_master_playlist(content, url);

        // Fetch segment info for each stream to get duration
        for stream in &mut streams {
            if let Ok(resp) = client.get(&stream.url).send().await {
                if let Ok(body) = resp.text().await {
                    let (segments, encrypted, duration) =
                        m3u8::parse_media_playlist(&body, &stream.url);
                    stream.segments = segments;
                    stream.is_encrypted = encrypted;
                    stream.total_duration = duration;
                }
            }
        }

        let best_idx = 0; // Already sorted by bandwidth desc

        Ok(VideoFound {
            id: video_id,
            page_url: url.to_string(),
            streams,
            title: None,
            best_quality_index: best_idx,
        })
    } else {
        // Single media playlist
        let (segments, encrypted, duration) = m3u8::parse_media_playlist(content, url);

        let stream = StreamInfo {
            url: url.to_string(),
            quality: "Original".to_string(),
            bandwidth: 0,
            resolution: None,
            codecs: None,
            segments,
            is_encrypted: encrypted,
            total_duration: duration,
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
    // Extract m3u8 URLs from HTML/JS source
    let m3u8_urls = extract_m3u8_urls(html, page_url);

    if m3u8_urls.is_empty() {
        return Err("No HLS streams found in page".to_string());
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
                        let (segments, encrypted, duration) =
                            m3u8::parse_media_playlist(&body, m3u8_url);
                        all_streams.push(StreamInfo {
                            url: m3u8_url.clone(),
                            quality: "Original".to_string(),
                            bandwidth: 0,
                            resolution: None,
                            codecs: None,
                            segments,
                            is_encrypted: encrypted,
                            total_duration: duration,
                        });
                    }
                }
            }
        }
    }

    if all_streams.is_empty() {
        return Err("Found m3u8 URLs but could not parse any streams".to_string());
    }

    all_streams.sort_by(|a, b| b.bandwidth.cmp(&a.bandwidth));

    // Try to extract page title
    let title = extract_title(html);

    Ok(VideoFound {
        id: video_id,
        page_url: page_url.to_string(),
        streams: all_streams,
        title,
        best_quality_index: 0,
    })
}

/// Extract m3u8 URLs from HTML/JS content
fn extract_m3u8_urls(html: &str, base_url: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Pattern: anything that looks like a URL ending in .m3u8
    // Matches both quoted strings and unquoted URLs
    let patterns = [
        // Quoted URLs
        r#"["']([^"']*\.m3u8[^"']*?)["']"#,
        // src= or href= attributes
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
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("vid_{}", ts)
}
