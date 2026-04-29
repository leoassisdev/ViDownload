use url::Url;

use super::types::{ByteRange, EncryptionKey, Segment, StreamInfo};

/// Parse a master playlist and return available quality levels
pub fn parse_master_playlist(content: &str, base_url: &str) -> Vec<StreamInfo> {
    let mut streams = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        if line.starts_with("#EXT-X-STREAM-INF:") {
            let attrs = &line["#EXT-X-STREAM-INF:".len()..];
            let bandwidth = extract_attr(attrs, "BANDWIDTH")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            let resolution = extract_attr(attrs, "RESOLUTION");
            let codecs = extract_attr(attrs, "CODECS");

            let quality = resolution
                .as_deref()
                .map(|r| format_quality(r, bandwidth))
                .unwrap_or_else(|| format!("{}kbps", bandwidth / 1000));

            // Next non-empty, non-comment line is the URL
            i += 1;
            while i < lines.len() && (lines[i].trim().is_empty() || lines[i].trim().starts_with('#'))
            {
                i += 1;
            }

            if i < lines.len() {
                let stream_url = resolve_url(base_url, lines[i].trim());
                streams.push(StreamInfo {
                    url: stream_url,
                    quality,
                    bandwidth,
                    resolution,
                    codecs,
                    segments: Vec::new(),
                    is_encrypted: false,
                    total_duration: 0.0,
                });
            }
        }

        i += 1;
    }

    // Sort by bandwidth descending (best quality first)
    streams.sort_by(|a, b| b.bandwidth.cmp(&a.bandwidth));
    streams
}

/// Parse a media playlist and return segments
pub fn parse_media_playlist(content: &str, base_url: &str) -> (Vec<Segment>, bool, f64) {
    let mut segments = Vec::new();
    let mut current_key: Option<EncryptionKey> = None;
    let mut sequence: u64 = 0;
    let mut duration: f64 = 0.0;
    let mut total_duration: f64 = 0.0;
    let mut byte_range: Option<ByteRange> = None;
    let mut is_encrypted = false;

    for line in content.lines() {
        let line = line.trim();

        if line.starts_with("#EXT-X-MEDIA-SEQUENCE:") {
            if let Ok(seq) = line["#EXT-X-MEDIA-SEQUENCE:".len()..].parse::<u64>() {
                sequence = seq;
            }
        } else if line.starts_with("#EXT-X-KEY:") {
            let attrs = &line["#EXT-X-KEY:".len()..];
            let method = extract_attr(attrs, "METHOD").unwrap_or_default();

            if method == "NONE" {
                current_key = None;
            } else {
                is_encrypted = true;
                let uri = extract_attr(attrs, "URI").unwrap_or_default();
                let iv = extract_attr(attrs, "IV");
                current_key = Some(EncryptionKey {
                    method,
                    uri: resolve_url(base_url, &uri),
                    iv,
                });
            }
        } else if line.starts_with("#EXTINF:") {
            let dur_str = line["#EXTINF:".len()..].trim_end_matches(',');
            // Duration may have a title after comma
            let dur_str = dur_str.split(',').next().unwrap_or("0");
            duration = dur_str.parse::<f64>().unwrap_or(0.0);
        } else if line.starts_with("#EXT-X-BYTERANGE:") {
            let range_str = &line["#EXT-X-BYTERANGE:".len()..];
            byte_range = parse_byte_range(range_str);
        } else if !line.is_empty() && !line.starts_with('#') {
            let seg_url = resolve_url(base_url, line);
            total_duration += duration;

            segments.push(Segment {
                url: seg_url,
                duration,
                sequence,
                byte_range: byte_range.take(),
                key: current_key.clone(),
            });

            sequence += 1;
            duration = 0.0;
        }
    }

    (segments, is_encrypted, total_duration)
}

/// Check if content looks like an m3u8 playlist
pub fn is_master_playlist(content: &str) -> bool {
    content.contains("#EXT-X-STREAM-INF:")
}

pub fn is_m3u8(content: &str) -> bool {
    content.trim_start().starts_with("#EXTM3U")
}

fn extract_attr(attrs: &str, name: &str) -> Option<String> {
    let search = format!("{}=", name);
    let start = attrs.find(&search)?;
    let value_start = start + search.len();
    let rest = &attrs[value_start..];

    if rest.starts_with('"') {
        // Quoted value
        let end = rest[1..].find('"')?;
        Some(rest[1..=end].to_string())
    } else {
        // Unquoted value (ends at comma or end)
        let end = rest.find(',').unwrap_or(rest.len());
        Some(rest[..end].to_string())
    }
}

fn resolve_url(base: &str, relative: &str) -> String {
    if relative.starts_with("http://") || relative.starts_with("https://") {
        return relative.to_string();
    }

    match Url::parse(base) {
        Ok(base_url) => base_url
            .join(relative)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| relative.to_string()),
        Err(_) => relative.to_string(),
    }
}

fn format_quality(resolution: &str, bandwidth: u64) -> String {
    // e.g. "1920x1080" → "1080p"
    if let Some(h) = resolution.split('x').nth(1) {
        format!("{}p", h)
    } else {
        format!("{}kbps", bandwidth / 1000)
    }
}

fn parse_byte_range(s: &str) -> Option<ByteRange> {
    let parts: Vec<&str> = s.split('@').collect();
    let length = parts.first()?.parse::<u64>().ok()?;
    let offset = parts.get(1).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    Some(ByteRange { length, offset })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_master_playlist() {
        let content = r#"#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=1280000,RESOLUTION=1280x720,CODECS="avc1.64001f,mp4a.40.2"
720p.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2560000,RESOLUTION=1920x1080,CODECS="avc1.640028,mp4a.40.2"
1080p.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=640000,RESOLUTION=640x360,CODECS="avc1.4d401e,mp4a.40.2"
360p.m3u8
"#;

        let streams = parse_master_playlist(content, "https://example.com/video/master.m3u8");

        assert_eq!(streams.len(), 3);
        // Sorted by bandwidth desc → 1080p first
        assert_eq!(streams[0].quality, "1080p");
        assert_eq!(streams[0].bandwidth, 2560000);
        assert!(streams[0].url.ends_with("1080p.m3u8"));

        assert_eq!(streams[1].quality, "720p");
        assert_eq!(streams[2].quality, "360p");
    }

    #[test]
    fn test_parse_media_playlist() {
        let content = r#"#EXTM3U
#EXT-X-TARGETDURATION:10
#EXT-X-MEDIA-SEQUENCE:0
#EXTINF:9.009,
segment0.ts
#EXTINF:9.009,
segment1.ts
#EXTINF:3.003,
segment2.ts
#EXT-X-ENDLIST
"#;

        let (segments, encrypted, duration) =
            parse_media_playlist(content, "https://cdn.example.com/video/");

        assert_eq!(segments.len(), 3);
        assert!(!encrypted);
        assert!((duration - 21.021).abs() < 0.01);
        assert_eq!(segments[0].sequence, 0);
        assert!(segments[0].url.contains("segment0.ts"));
    }

    #[test]
    fn test_parse_encrypted_playlist() {
        let content = r#"#EXTM3U
#EXT-X-KEY:METHOD=AES-128,URI="key.bin",IV=0x00000000000000000000000000000001
#EXTINF:10.0,
enc_seg0.ts
#EXTINF:10.0,
enc_seg1.ts
"#;

        let (segments, encrypted, _) =
            parse_media_playlist(content, "https://cdn.example.com/video/");

        assert!(encrypted);
        assert_eq!(segments.len(), 2);
        let key = segments[0].key.as_ref().unwrap();
        assert_eq!(key.method, "AES-128");
        assert!(key.uri.contains("key.bin"));
        assert!(key.iv.is_some());
    }
}
