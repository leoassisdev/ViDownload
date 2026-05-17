use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::process::Command;
use std::process::Stdio;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedStreamCDP {
    pub url: String,
    #[serde(rename = "type")]
    pub stream_type: String,
    #[serde(rename = "contentType")]
    pub content_type: String,
}

/// Encontra o Chrome instalado no sistema.
fn find_chrome() -> Option<String> {
    let paths = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium-browser",
    ];
    for p in &paths {
        if std::path::Path::new(p).exists() {
            return Some(p.to_string());
        }
    }
    None
}

/// Escolhe uma porta livre para debugging.
fn pick_debug_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").ok();
    listener.map(|l| l.local_addr().unwrap().port()).unwrap_or(9222)
}

/// Abre Chrome real via CDP (WebSocket direto do Rust), intercepta rede
/// e retorna URLs de streams HLS/m3u8 encontrados.
/// Zero dependência de Node.js.
#[tauri::command]
pub async fn chrome_find_streams(
    app: AppHandle,
    url: String,
    timeout_secs: Option<u32>,
) -> Result<Vec<DetectedStreamCDP>, String> {
    let timeout = timeout_secs.unwrap_or(30) as u64;

    let chrome_path = find_chrome()
        .ok_or_else(|| "Chrome não encontrado no sistema. Instale o Google Chrome.".to_string())?;

    // Porta dinâmica para não conflitar
    let port = pick_debug_port();

    // Criar user data dir temporário para não conflitar com Chrome do usuário
    let tmp_dir = std::env::temp_dir().join(format!("vidownload-chrome-{}", port));
    let _ = std::fs::create_dir_all(&tmp_dir);

    let mut child = Command::new(&chrome_path)
        .args([
            "--headless=new",
            "--disable-gpu",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-background-networking",
            "--disable-default-apps",
            &format!("--remote-debugging-port={}", port),
            &format!("--user-data-dir={}", tmp_dir.display()),
            "--window-size=1280,900",
            "about:blank",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Erro ao iniciar Chrome: {}", e))?;

    eprintln!("[chrome] Chrome lançado na porta {}", port);

    // Esperar Chrome subir e expor CDP
    let ws_url = wait_for_cdp(port, 15).await.map_err(|e| {
        let _ = child.kill();
        let _ = std::fs::remove_dir_all(&tmp_dir);
        format!("Chrome não respondeu via CDP: {}", e)
    })?;

    eprintln!("[chrome] WebSocket CDP: {}", ws_url);

    let result = run_cdp_session(&app, &ws_url, &url, timeout).await;

    // Fechar Chrome e limpar
    let _ = child.kill().await;
    let _ = child.wait().await;
    let _ = std::fs::remove_dir_all(&tmp_dir);

    result
}

/// Espera o Chrome expor o endpoint CDP e retorna a WebSocket URL da primeira página.
async fn wait_for_cdp(port: u16, max_retries: u32) -> Result<String, String> {
    for i in 0..max_retries {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let url = format!("http://127.0.0.1:{}/json", port);
        if let Ok(resp) = reqwest::get(&url).await {
            if let Ok(text) = resp.text().await {
                if let Ok(tabs) = serde_json::from_str::<Vec<serde_json::Value>>(&text) {
                    for tab in &tabs {
                        if let Some(ws) = tab.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) {
                            return Ok(ws.to_string());
                        }
                    }
                }
            }
        }
        if i % 3 == 0 {
            eprintln!("[chrome] Aguardando CDP... tentativa {}/{}", i + 1, max_retries);
        }
    }
    Err("Timeout esperando Chrome CDP".to_string())
}

/// Helper para enviar comando CDP e ler respostas.
struct CdpConn {
    id_counter: AtomicU64,
}

impl CdpConn {
    fn new() -> Self {
        Self { id_counter: AtomicU64::new(1) }
    }

    fn make_msg(&self, method: &str, params: serde_json::Value) -> (u64, String) {
        let id = self.id_counter.fetch_add(1, Ordering::SeqCst);
        let msg = serde_json::json!({ "id": id, "method": method, "params": params });
        (id, msg.to_string())
    }
}

/// Executa sessão CDP: habilita Network, navega, coleta streams.
async fn run_cdp_session(
    app: &AppHandle,
    ws_url: &str,
    target_url: &str,
    timeout_secs: u64,
) -> Result<Vec<DetectedStreamCDP>, String> {
    let (ws_stream, _) = tokio_tungstenite::connect_async(ws_url)
        .await
        .map_err(|e| format!("Erro ao conectar WebSocket CDP: {}", e))?;

    let (mut write, mut read) = ws_stream.split();
    let cdp = CdpConn::new();

    // 1. Habilitar Network e Page domains
    let (net_id, net_msg) = cdp.make_msg("Network.enable", serde_json::json!({}));
    write.send(Message::Text(net_msg.into())).await.map_err(|e| e.to_string())?;

    let (page_id, page_msg) = cdp.make_msg("Page.enable", serde_json::json!({}));
    write.send(Message::Text(page_msg.into())).await.map_err(|e| e.to_string())?;

    // Esperar confirmação de Network.enable e Page.enable antes de navegar
    let mut net_ready = false;
    let mut page_ready = false;
    let enable_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);

    while (!net_ready || !page_ready) && tokio::time::Instant::now() < enable_deadline {
        match tokio::time::timeout(std::time::Duration::from_secs(3), read.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&text.to_string()) {
                    if let Some(id) = msg.get("id").and_then(|v| v.as_u64()) {
                        if id == net_id { net_ready = true; }
                        if id == page_id { page_ready = true; }
                    }
                }
            }
            _ => break,
        }
    }

    eprintln!("[chrome] Network.enable={}, Page.enable={}", net_ready, page_ready);

    // 2. Navegar para a URL do usuário
    let (_, nav_msg) = cdp.make_msg("Page.navigate", serde_json::json!({ "url": target_url }));
    write.send(Message::Text(nav_msg.into())).await.map_err(|e| e.to_string())?;

    eprintln!("[chrome] Navegando para: {}", target_url);

    let mut detected: Vec<DetectedStreamCDP> = Vec::new();
    let mut seen_urls = std::collections::HashSet::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let mut found_time: Option<tokio::time::Instant> = None;
    let mut page_loaded = false;
    let mut tried_click = false;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            eprintln!("[chrome] Timeout atingido");
            break;
        }

        // Se já encontrou stream, esperar mais 3s por adicionais
        if let Some(ft) = found_time {
            if ft.elapsed() > std::time::Duration::from_secs(3) {
                break;
            }
        }

        match tokio::time::timeout(remaining.min(std::time::Duration::from_millis(500)), read.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let text_str = text.to_string();
                if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&text_str) {
                    let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");

                    // Detectar quando a página carregou
                    if method == "Page.loadEventFired" || method == "Page.domContentEventFired" {
                        page_loaded = true;
                        eprintln!("[chrome] Página carregada");
                    }

                    let (resp_url, content_type) = match method {
                        "Network.responseReceived" => {
                            let response = msg.get("params").and_then(|p| p.get("response"));
                            let u = response.and_then(|r| r.get("url")).and_then(|v| v.as_str()).unwrap_or("");
                            let ct = response.and_then(|r| r.get("mimeType")).and_then(|v| v.as_str()).unwrap_or("");
                            let status = response.and_then(|r| r.get("status")).and_then(|v| v.as_u64()).unwrap_or(0);
                            if status >= 400 { continue; }
                            (u.to_string(), ct.to_string())
                        }
                        "Network.requestWillBeSent" => {
                            let u = msg.get("params")
                                .and_then(|p| p.get("request"))
                                .and_then(|r| r.get("url"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            (u.to_string(), String::new())
                        }
                        _ => continue,
                    };

                    if resp_url.is_empty() || seen_urls.contains(&resp_url) {
                        continue;
                    }

                    if is_stream_url(&resp_url, &content_type) {
                        seen_urls.insert(resp_url.clone());

                        let combined = format!("{}{}", resp_url, content_type);
                        let stream_type = if regex_lite::Regex::new(r"(?i)m3u8|m3u|mpegurl").unwrap().is_match(&combined) {
                            "hls"
                        } else if regex_lite::Regex::new(r"(?i)\.mpd|dash").unwrap().is_match(&combined) {
                            "dash"
                        } else {
                            "unknown"
                        };

                        let stream = DetectedStreamCDP {
                            url: resp_url,
                            stream_type: stream_type.to_string(),
                            content_type,
                        };

                        eprintln!("[chrome] Stream encontrado: {} ({})", stream.url, stream.stream_type);
                        detected.push(stream.clone());
                        let _ = app.emit("chrome-stream-found", stream);

                        if found_time.is_none() {
                            found_time = Some(tokio::time::Instant::now());
                        }
                    }
                }
            }
            Ok(Some(Ok(_))) => {} // binary/ping/pong
            Ok(Some(Err(e))) => {
                eprintln!("[chrome] WebSocket erro: {}", e);
                break;
            }
            Ok(None) => break,
            Err(_) => {
                // Timeout de 500ms — aproveitar para tentar clicar no player se página carregou
                if page_loaded && !tried_click && detected.is_empty() {
                    tried_click = true;
                    eprintln!("[chrome] Tentando clicar no player...");

                    // Clicar no centro da página (muitos players precisam de click para iniciar)
                    let js_click = r#"
                        (function() {
                            // Tentar clicar em video elements ou play buttons
                            var v = document.querySelector('video');
                            if (v) { v.click(); v.play && v.play().catch(function(){}); }
                            // Clicar em botões de play comuns
                            var btns = document.querySelectorAll('[class*="play"], [id*="play"], [aria-label*="play"], [aria-label*="Play"], button');
                            for (var i = 0; i < Math.min(btns.length, 5); i++) { btns[i].click(); }
                            // Click no centro da viewport
                            var el = document.elementFromPoint(window.innerWidth/2, window.innerHeight/2);
                            if (el) el.click();
                        })()
                    "#;
                    let (_, click_msg) = cdp.make_msg("Runtime.evaluate", serde_json::json!({
                        "expression": js_click,
                        "userGesture": true
                    }));
                    let _ = write.send(Message::Text(click_msg.into())).await;
                }
            }
        }
    }

    eprintln!("[chrome] Sessão finalizada. {} streams encontrados.", detected.len());

    if detected.is_empty() {
        Err("Nenhum stream encontrado na página".to_string())
    } else {
        Ok(detected)
    }
}

/// Verifica se uma URL/content-type corresponde a um stream HLS/DASH.
fn is_stream_url(url: &str, content_type: &str) -> bool {
    // Ignorar data: e chrome-extension: URLs
    if url.starts_with("data:") || url.starts_with("chrome") || url.starts_with("blob:") {
        return false;
    }

    let re_url = regex_lite::Regex::new(r"(?i)\.(m3u8|m3u|mpd)(\?|$|#)").unwrap();
    let re_ct = regex_lite::Regex::new(r"(?i)mpegurl|dash\+xml|x-mpegurl|vnd\.apple\.mpegurl").unwrap();
    let re_manifest = regex_lite::Regex::new(r"(?i)(manifest|playlist|master|index)\.(m3u8|m3u|mpd)").unwrap();
    let re_path = regex_lite::Regex::new(r"(?i)/hls/|/manifest/|/playlist\.m3u|format=m3u|\.m3u8").unwrap();

    re_url.is_match(url) || re_ct.is_match(content_type) || re_manifest.is_match(url) || re_path.is_match(url)
}
