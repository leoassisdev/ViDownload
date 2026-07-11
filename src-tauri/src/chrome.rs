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
                    // Preferir um target do tipo "page" (a aba real), ignorando
                    // background de extensões / service workers.
                    let page_ws = tabs
                        .iter()
                        .find(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
                        .and_then(|t| t.get("webSocketDebuggerUrl").and_then(|v| v.as_str()));
                    if let Some(ws) = page_ws {
                        return Ok(ws.to_string());
                    }
                    // Fallback: qualquer target com webSocketDebuggerUrl
                    for tab in &tabs {
                        if tab.get("type").and_then(|v| v.as_str()) == Some("page") {
                            continue;
                        }
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

// ============================================================================
// Extração de iframes com auto-login (para áreas de membros, ex. MemberKit)
// ============================================================================

type Ws = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

/// Envia um comando CDP e aguarda a resposta com o mesmo id, ignorando eventos.
async fn cdp_cmd(
    ws: &mut Ws,
    cdp: &CdpConn,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let (id, msg) = cdp.make_msg(method, params);
    ws.send(Message::Text(msg.into()))
        .await
        .map_err(|e| e.to_string())?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t.to_string()) {
                    if v.get("id").and_then(|x| x.as_u64()) == Some(id) {
                        if let Some(err) = v.get("error") {
                            return Err(err.to_string());
                        }
                        return Ok(v.get("result").cloned().unwrap_or(serde_json::Value::Null));
                    }
                }
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(e))) => return Err(e.to_string()),
            Ok(None) => return Err("WebSocket CDP fechado".to_string()),
            Err(_) => {}
        }
    }
    Err(format!("Timeout no comando CDP {}", method))
}

/// Avalia JS na página e retorna o valor (returnByValue).
async fn cdp_eval(ws: &mut Ws, cdp: &CdpConn, expr: &str) -> Result<serde_json::Value, String> {
    let r = cdp_cmd(
        ws,
        cdp,
        "Runtime.evaluate",
        serde_json::json!({
            "expression": expr,
            "returnByValue": true,
            "userGesture": true,
            "awaitPromise": true
        }),
    )
    .await?;
    Ok(r.pointer("/result/value").cloned().unwrap_or(serde_json::Value::Null))
}

/// Abre uma página num Chrome com perfil persistente (mantém login entre sessões),
/// faz auto-login quando há formulário e credenciais, e retorna os `src` dos iframes.
pub async fn extract_iframes(
    profile_dir: &std::path::Path,
    url: &str,
    email: Option<&str>,
    password: Option<&str>,
) -> Result<Vec<String>, String> {
    let chrome_path =
        find_chrome().ok_or_else(|| "Chrome não encontrado no sistema.".to_string())?;
    let port = pick_debug_port();
    let _ = std::fs::create_dir_all(profile_dir);

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
            &format!("--user-data-dir={}", profile_dir.display()),
            "--window-size=1280,900",
            "about:blank",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Erro ao iniciar Chrome: {}", e))?;

    let result = extract_iframes_inner(port, url, email, password).await;

    let _ = child.kill().await;
    let _ = child.wait().await;
    // NÃO remover o profile_dir: é persistente para manter o login.

    result
}

/// Estado observado da página: se há campo de senha e os src dos iframes.
struct PageProbe {
    ready: bool,
    has_password: bool,
    frames: Vec<String>,
    url: String,
    title: String,
}

/// Lê o estado atual da página numa única avaliação.
async fn probe_page(ws: &mut Ws, cdp: &CdpConn) -> PageProbe {
    let expr = r#"JSON.stringify({
        ready: document.readyState === 'complete',
        pass: !!document.querySelector('input[type=password]'),
        url: location.href,
        title: document.title,
        frames: Array.from(document.querySelectorAll('iframe')).map(function(f){return f.src;}).filter(Boolean)
    })"#;
    let v = cdp_eval(ws, cdp, expr).await.unwrap_or(serde_json::Value::Null);
    let parsed = v
        .as_str()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .unwrap_or(serde_json::Value::Null);
    PageProbe {
        ready: parsed.get("ready").and_then(|x| x.as_bool()).unwrap_or(false),
        has_password: parsed.get("pass").and_then(|x| x.as_bool()).unwrap_or(false),
        url: parsed.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        title: parsed.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        frames: parsed
            .get("frames")
            .and_then(|x| x.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default(),
    }
}

/// Aguarda a página assentar: retorna assim que houver iframe ou campo de senha,
/// ou quando o carregamento completa, ou no timeout.
async fn wait_page_settle(ws: &mut Ws, cdp: &CdpConn, max_secs: u64) -> PageProbe {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(max_secs);
    let mut last = PageProbe {
        ready: false,
        has_password: false,
        frames: Vec::new(),
        url: String::new(),
        title: String::new(),
    };
    while tokio::time::Instant::now() < deadline {
        last = probe_page(ws, cdp).await;
        if !last.frames.is_empty() || last.has_password {
            // Se achou iframe, dar um instante para carregarem os demais
            if !last.frames.is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(700)).await;
                last = probe_page(ws, cdp).await;
            }
            return last;
        }
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    }
    last
}

async fn extract_iframes_inner(
    port: u16,
    url: &str,
    email: Option<&str>,
    password: Option<&str>,
) -> Result<Vec<String>, String> {
    let ws_url = wait_for_cdp(port, 20)
        .await
        .map_err(|e| format!("Chrome CDP não respondeu: {}", e))?;

    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|e| format!("Erro ao conectar CDP: {}", e))?;

    let cdp = CdpConn::new();
    let _ = cdp_cmd(&mut ws, &cdp, "Page.enable", serde_json::json!({})).await;
    let _ = cdp_cmd(&mut ws, &cdp, "Runtime.enable", serde_json::json!({})).await;

    // Navegar para a página alvo e aguardar assentar
    cdp_cmd(&mut ws, &cdp, "Page.navigate", serde_json::json!({ "url": url })).await?;
    let mut state = wait_page_settle(&mut ws, &cdp, 20).await;
    eprintln!(
        "[chrome] estado inicial: ready={} pass={} frames={} url={} title={}",
        state.ready,
        state.has_password,
        state.frames.len(),
        state.url,
        state.title
    );

    // Se caiu na tela de login, tentar auto-login
    if state.has_password && state.frames.is_empty() {
        match (email, password) {
            (Some(e), Some(p)) if !e.is_empty() && !p.is_empty() => {
                eprintln!("[chrome] Login detectado — preenchendo credenciais");
                let fill = format!(
                    r#"(function(){{
                        var eF=document.querySelector('input[type=email], input[name*=email i], input[name=login], input[name=user], input[type=text]');
                        var pF=document.querySelector('input[type=password]');
                        if(eF){{eF.value="{}";eF.dispatchEvent(new Event('input',{{bubbles:true}}));eF.dispatchEvent(new Event('change',{{bubbles:true}}));}}
                        if(pF){{pF.value="{}";pF.dispatchEvent(new Event('input',{{bubbles:true}}));pF.dispatchEvent(new Event('change',{{bubbles:true}}));}}
                        return !!(eF&&pF);
                    }})()"#,
                    json_escape(e),
                    json_escape(p)
                );
                let _ = cdp_eval(&mut ws, &cdp, &fill).await;
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                let submit = r#"(function(){
                    var btn=document.querySelector('button[type=submit], input[type=submit]');
                    var form=document.querySelector('form');
                    if(btn){btn.click();return 'btn';}
                    if(form){form.submit();return 'form';}
                    return 'none';
                })()"#;
                let _ = cdp_eval(&mut ws, &cdp, submit).await;
                // Aguardar o POST de login processar/redirecionar
                tokio::time::sleep(std::time::Duration::from_millis(3500)).await;

                // Ir para a página alvo novamente e aguardar o player
                cdp_cmd(&mut ws, &cdp, "Page.navigate", serde_json::json!({ "url": url })).await?;
                state = wait_page_settle(&mut ws, &cdp, 20).await;
                eprintln!(
                    "[chrome] pós-login: ready={} pass={} frames={}",
                    state.ready,
                    state.has_password,
                    state.frames.len()
                );

                if state.has_password && state.frames.is_empty() {
                    return Err(
                        "Login falhou (credenciais inválidas ou captcha). Verifique email/senha nas configurações."
                            .to_string(),
                    );
                }
            }
            _ => {
                return Err(
                    "Esta página exige login. Cadastre email e senha do site nas configurações."
                        .to_string(),
                );
            }
        }
    }

    eprintln!("[chrome] {} iframe(s) encontrados", state.frames.len());
    Ok(state.frames)
}

/// Escapa uma string para embutir com segurança dentro de aspas duplas em JS.
fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
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
