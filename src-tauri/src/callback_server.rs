use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::engine::sniffer::{DetectedStream, SnifferState};

const PORT: u16 = 17377;

/// Servidor HTTP local que recebe URLs detectadas pelo content script do browser.
/// O content script não tem acesso ao Tauri IPC em páginas remotas,
/// então envia via fetch/Image para http://127.0.0.1:17377/detected?url=...
pub async fn start(app: AppHandle, sniffer: Arc<SnifferState>) {
    let listener = match TcpListener::bind(format!("127.0.0.1:{}", PORT)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[callback_server] Erro ao iniciar na porta {}: {}", PORT, e);
            return;
        }
    };

    println!("[callback_server] Escutando em 127.0.0.1:{}", PORT);

    loop {
        match listener.accept().await {
            Ok((mut stream, _)) => {
                let app = app.clone();
                let sniffer = sniffer.clone();

                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = match stream.read(&mut buf).await {
                        Ok(n) if n > 0 => n,
                        _ => return,
                    };

                    let request = String::from_utf8_lossy(&buf[..n]);

                    // Parse: GET /detected?url=...&source=... HTTP/1.1
                    if let Some(first_line) = request.lines().next() {
                        if first_line.contains("/detected") {
                            if let Some(url) = extract_param(first_line, "url") {
                                let source = extract_param(first_line, "source")
                                    .unwrap_or_else(|| "callback".to_string());

                                let decoded_url =
                                    urlencoding::decode(&url).unwrap_or(std::borrow::Cow::Borrowed(&url)).to_string();

                                println!("[callback_server] Stream detectado: {}", decoded_url);

                                let stream_info = DetectedStream {
                                    url: decoded_url,
                                    content_type: None,
                                    source,
                                    headers: std::collections::HashMap::new(),
                                    timestamp: std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap()
                                        .as_millis()
                                        as u64,
                                };

                                if sniffer.register(stream_info) {
                                    let count = sniffer.get_all().len();
                                    let _ = app.emit("stream-detected", count);
                                }
                            }
                        }
                    }

                    // Responder com CORS headers
                    let response = "HTTP/1.1 200 OK\r\n\
                        Access-Control-Allow-Origin: *\r\n\
                        Access-Control-Allow-Methods: GET, OPTIONS\r\n\
                        Content-Type: text/plain\r\n\
                        Content-Length: 2\r\n\
                        \r\n\
                        OK";
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
            Err(e) => {
                eprintln!("[callback_server] Erro ao aceitar conexão: {}", e);
            }
        }
    }
}

fn extract_param(line: &str, name: &str) -> Option<String> {
    let query_start = line.find('?')?;
    let query_end = line[query_start..].find(' ').unwrap_or(line.len() - query_start);
    let query = &line[query_start + 1..query_start + query_end];

    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        if let (Some(key), Some(value)) = (parts.next(), parts.next()) {
            if key == name {
                return Some(value.to_string());
            }
        }
    }

    None
}
