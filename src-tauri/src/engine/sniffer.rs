use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Tipos de stream detectáveis
const STREAM_CONTENT_TYPES: &[&str] = &[
    "application/x-mpegurl",
    "application/vnd.apple.mpegurl",
    "audio/mpegurl",
    "audio/x-mpegurl",
    "video/mp2t",
    "application/dash+xml",
    "video/mp4",
    "audio/mp4",
    "video/webm",
];

/// Extensões de arquivo de stream
const STREAM_EXTENSIONS: &[&str] = &[
    "m3u8", "m3u", "ts", "m4s", "mpd", "m4a", "m4v", "mp4", "fmp4",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedStream {
    pub url: String,
    pub content_type: Option<String>,
    pub source: String, // "content-type", "extension", "content-script"
    pub headers: HashMap<String, String>,
    pub timestamp: u64,
}

/// Estado do sniffer compartilhado entre threads
pub struct SnifferState {
    pub detected: Mutex<Vec<DetectedStream>>,
    seen_urls: Mutex<std::collections::HashSet<String>>,
}

impl Default for SnifferState {
    fn default() -> Self {
        Self {
            detected: Mutex::new(Vec::new()),
            seen_urls: Mutex::new(std::collections::HashSet::new()),
        }
    }
}

impl SnifferState {
    /// Registra um stream detectado, evitando duplicatas
    pub fn register(&self, stream: DetectedStream) -> bool {
        let mut seen = self.seen_urls.lock().unwrap();
        if seen.contains(&stream.url) {
            return false;
        }
        seen.insert(stream.url.clone());

        let mut detected = self.detected.lock().unwrap();
        detected.push(stream);
        true
    }

    /// Retorna todos os streams detectados
    pub fn get_all(&self) -> Vec<DetectedStream> {
        self.detected.lock().unwrap().clone()
    }

    /// Limpa os streams detectados
    pub fn clear(&self) {
        self.detected.lock().unwrap().clear();
        self.seen_urls.lock().unwrap().clear();
    }
}

pub type SharedSniffer = Arc<SnifferState>;

pub fn create_sniffer() -> SharedSniffer {
    Arc::new(SnifferState::default())
}

/// Verifica se uma URL é de stream pelo Content-Type
pub fn is_stream_content_type(content_type: &str) -> bool {
    let ct = content_type.to_lowercase();
    STREAM_CONTENT_TYPES.iter().any(|&t| ct.contains(t))
}

/// Verifica se uma URL é de stream pela extensão
pub fn is_stream_url(url: &str) -> bool {
    // Extrair a parte antes de query params
    let path = url.split('?').next().unwrap_or(url);
    let path = path.split('#').next().unwrap_or(path);

    if let Some(ext) = path.rsplit('.').next() {
        STREAM_EXTENSIONS.iter().any(|&e| ext.eq_ignore_ascii_case(e))
    } else {
        false
    }
}

/// Verifica se URL contém padrões comuns de streaming
pub fn is_stream_url_pattern(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("/manifest") && (lower.contains("video") || lower.contains("audio"))
        || lower.contains("/playlist.m3u8")
        || lower.contains("/master.m3u8")
        || lower.contains("/index.m3u8")
        || lower.contains("/chunklist")
        || lower.contains("hls") && lower.contains("m3u")
}

/// Captura headers relevantes de uma resposta HTTP
pub fn capture_relevant_headers(
    headers: &reqwest::header::HeaderMap,
) -> HashMap<String, String> {
    let mut captured = HashMap::new();
    let relevant = [
        "referer",
        "origin",
        "cookie",
        "authorization",
        "user-agent",
        "accept",
        "x-requested-with",
    ];

    for (name, value) in headers.iter() {
        let name_str = name.as_str().to_lowercase();
        if relevant.contains(&name_str.as_str()) {
            if let Ok(v) = value.to_str() {
                captured.insert(name_str, v.to_string());
            }
        }
    }

    captured
}

fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Analisa uma resposta HTTP e detecta se é um stream
pub fn check_response(
    url: &str,
    content_type: Option<&str>,
    headers: &reqwest::header::HeaderMap,
) -> Option<DetectedStream> {
    let mut source = String::new();

    // Verificar por Content-Type
    if let Some(ct) = content_type {
        if is_stream_content_type(ct) {
            source = "content-type".to_string();
        }
    }

    // Verificar por extensão
    if source.is_empty() && is_stream_url(url) {
        source = "extension".to_string();
    }

    // Verificar por padrão de URL
    if source.is_empty() && is_stream_url_pattern(url) {
        source = "url-pattern".to_string();
    }

    if source.is_empty() {
        return None;
    }

    Some(DetectedStream {
        url: url.to_string(),
        content_type: content_type.map(|s| s.to_string()),
        source,
        headers: capture_relevant_headers(headers),
        timestamp: now_millis(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_stream_content_type() {
        assert!(is_stream_content_type("application/x-mpegurl"));
        assert!(is_stream_content_type("Application/vnd.apple.mpegURL"));
        assert!(is_stream_content_type("video/mp2t"));
        assert!(is_stream_content_type("application/dash+xml"));
        assert!(!is_stream_content_type("text/html"));
        assert!(!is_stream_content_type("application/json"));
    }

    #[test]
    fn test_is_stream_url() {
        assert!(is_stream_url("https://cdn.example.com/video/master.m3u8"));
        assert!(is_stream_url("https://cdn.example.com/video/seg001.ts"));
        assert!(is_stream_url("https://cdn.example.com/video/init.m4s"));
        assert!(is_stream_url(
            "https://cdn.example.com/video/manifest.mpd"
        ));
        assert!(is_stream_url(
            "https://cdn.example.com/video/chunk.m3u8?token=abc"
        ));
        assert!(!is_stream_url("https://example.com/page.html"));
        assert!(!is_stream_url("https://example.com/script.js"));
    }

    #[test]
    fn test_is_stream_url_pattern() {
        assert!(is_stream_url_pattern(
            "https://cdn.example.com/hls/master.m3u"
        ));
        assert!(is_stream_url_pattern(
            "https://cdn.example.com/video/playlist.m3u8"
        ));
        assert!(!is_stream_url_pattern("https://example.com/index.html"));
    }

    #[test]
    fn test_sniffer_dedup() {
        let sniffer = SnifferState::default();
        let stream = DetectedStream {
            url: "https://example.com/video.m3u8".to_string(),
            content_type: Some("application/x-mpegurl".to_string()),
            source: "content-type".to_string(),
            headers: HashMap::new(),
            timestamp: 0,
        };

        assert!(sniffer.register(stream.clone()));
        assert!(!sniffer.register(stream.clone())); // duplicata
        assert_eq!(sniffer.get_all().len(), 1);
    }
}
