use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamInfo {
    pub url: String,
    pub quality: String,
    pub bandwidth: u64,
    pub resolution: Option<String>,
    pub codecs: Option<String>,
    pub segments: Vec<Segment>,
    pub is_encrypted: bool,
    pub is_live: bool,
    pub init_segment_url: Option<String>,
    pub total_duration: f64,
    pub target_duration: f64,
    pub media_sequence: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub url: String,
    pub duration: f64,
    pub sequence: u64,
    pub byte_range: Option<ByteRange>,
    pub key: Option<EncryptionKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ByteRange {
    pub length: u64,
    pub offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionKey {
    pub method: String,
    pub uri: String,
    pub iv: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoFound {
    pub id: String,
    pub page_url: String,
    pub streams: Vec<StreamInfo>,
    pub title: Option<String>,
    pub best_quality_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRequest {
    pub video_id: String,
    pub stream_index: usize,
    pub output_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub video_id: String,
    pub state: DownloadState,
    pub segments_done: u64,
    pub segments_total: u64,
    pub bytes_downloaded: u64,
    pub speed_bps: u64,
    pub eta_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DownloadState {
    Analyzing,
    Downloading,
    Muxing,
    Done,
    Error(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct CapturedHeaders {
    pub headers: HashMap<String, String>,
    pub referer: Option<String>,
    pub origin: Option<String>,
}

impl Default for CapturedHeaders {
    fn default() -> Self {
        Self {
            headers: HashMap::new(),
            referer: None,
            origin: None,
        }
    }
}
