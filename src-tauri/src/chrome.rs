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

/// Uma aula do curso MemberKit (módulo, título, URL e id do Vimeo).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberkitLesson {
    pub module_index: usize,
    pub module_name: String,
    pub lesson_index: usize,
    pub title: String,
    pub lesson_url: String,
    pub vimeo_id: Option<String>,
    /// Materiais/anexos da aula (nome, url) — PDFs, código-fonte, etc.
    #[serde(default)]
    pub attachments: Vec<(String, String)>,
}

/// JS que coleta links de material/anexo na página da aula (PDFs, zips, código-fonte).
/// Ignora links de navegação de aula (/{id}-slug) e âncoras vazias.
const MEMBERKIT_ATTACH_JS: &str = r#"JSON.stringify((function(){
    var out=[]; var seen={};
    var exts=/\.(pdf|zip|rar|7z|pptx?|docx?|xlsx?|csv|txt|json|sql|ipynb|py|js|md)(\?|$)/i;
    Array.from(document.querySelectorAll('a[href]')).forEach(function(a){
        var href=a.href||''; if(!href) return;
        if(/memberkit\.com\.br\/\d+-/.test(href)) return;         // link de aula
        if(href.indexOf('/comments')>=0||href.indexOf('#')===0) return;
        var name=(a.textContent||'').replace(/\s+/g,' ').trim();
        var isFile = exts.test(href) || /\/download|\/attachment|\/arquivo|\/file|amazonaws|memberkit.*\/uploads/i.test(href);
        if(isFile && !seen[href]){ seen[href]=1; out.push({name: name||href.split('/').pop(), url: href}); }
    });
    return out;
})())"#;

/// JS que raspa a raiz do curso: emite módulos ("Módulo N ...") e aulas em ordem.
/// JS que raspa a raiz do curso: emite módulos ("Módulo N ...") e aulas em ordem.
const MEMBERKIT_SCRAPE_JS: &str = r#"JSON.stringify((function(){
    var out=[]; var curModule=''; var seen={};
    document.querySelectorAll('body *').forEach(function(el){
        var t=(el.textContent||'').replace(/\s+/g,' ').trim();
        if(/^M[oó]dulo\s+\d+/i.test(t) && t.length<48 && el.children.length<=3){
            if(curModule!==t && !seen['M:'+t]){ curModule=t; seen['M:'+t]=1; out.push({type:'module', name:t}); }
        }
        if(el.tagName==='A'){
            var href=el.href||'';
            var m=href.match(/memberkit\.com\.br\/\d+-[^\/]+\/(\d+)-/);
            if(m){
                var lt=(el.textContent||'').replace(/\s+/g,' ').trim();
                if(!seen['L:'+m[1]] && lt){ seen['L:'+m[1]]=1; out.push({type:'lesson', title:lt.slice(0,110), href:href.split('?')[0], module:curModule}); }
            }
        }
    });
    return { items: out };
})())"#;

/// Baixa a árvore do curso MemberKit numa ÚNICA sessão logada (cookie não persiste):
/// loga, raspa a raiz do curso (módulos→aulas) e navega cada aula para pegar o id
/// do Vimeo. O download em si (extract_vimeo) roda depois, sem browser.
pub async fn sniff_memberkit_course(
    profile_dir: &std::path::Path,
    course_root_url: &str,
    email: Option<&str>,
    password: Option<&str>,
) -> Result<Vec<MemberkitLesson>, String> {
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

    let result = sniff_memberkit_course_inner(port, course_root_url, email, password).await;
    let _ = child.kill().await;
    let _ = child.wait().await;
    result
}

async fn sniff_memberkit_course_inner(
    port: u16,
    course_root_url: &str,
    email: Option<&str>,
    password: Option<&str>,
) -> Result<Vec<MemberkitLesson>, String> {
    let ws_url = wait_for_cdp(port, 20)
        .await
        .map_err(|e| format!("Chrome CDP não respondeu: {}", e))?;
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|e| format!("Erro ao conectar CDP: {}", e))?;
    let cdp = CdpConn::new();
    let _ = cdp_cmd(&mut ws, &cdp, "Page.enable", serde_json::json!({})).await;
    let _ = cdp_cmd(&mut ws, &cdp, "Runtime.enable", serde_json::json!({})).await;

    // 1) Logar + raspar a raiz do curso.
    login_navigate(&mut ws, &cdp, course_root_url, email, password).await?;
    let scraped = cdp_eval(&mut ws, &cdp, MEMBERKIT_SCRAPE_JS).await?;
    let parsed: serde_json::Value = scraped
        .as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Null);

    // 2) Montar lista ordenada. Índices vêm dos números reais em "Módulo N"/"Aula N"
    // (mais confiáveis que sequência, pois há cabeçalhos duplicados no DOM).
    let num_re = regex_lite::Regex::new(r"(?i)m[oó]dulo\s+(\d+)").unwrap();
    let aula_re = regex_lite::Regex::new(r"(?i)aula\s+(\d+)").unwrap();
    let mut lessons: Vec<MemberkitLesson> = Vec::new();
    let mut seq_module = 0usize;
    let mut module_index = 0usize;
    let mut module_name = String::new();
    let mut lesson_in_module = 0usize;
    if let Some(items) = parsed.get("items").and_then(|v| v.as_array()) {
        for it in items {
            match it.get("type").and_then(|v| v.as_str()) {
                Some("module") => {
                    seq_module += 1;
                    module_name = it
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Módulo")
                        .to_string();
                    module_index = num_re
                        .captures(&module_name)
                        .and_then(|c| c.get(1))
                        .and_then(|m| m.as_str().parse().ok())
                        .unwrap_or(seq_module);
                    lesson_in_module = 0;
                }
                Some("lesson") => {
                    lesson_in_module += 1;
                    let title = it.get("title").and_then(|v| v.as_str()).unwrap_or("Aula");
                    let lesson_index = aula_re
                        .captures(title)
                        .and_then(|c| c.get(1))
                        .and_then(|m| m.as_str().parse().ok())
                        .unwrap_or(lesson_in_module);
                    lessons.push(MemberkitLesson {
                        module_index: module_index.max(0),
                        module_name: if module_name.is_empty() {
                            "Aulas".to_string()
                        } else {
                            module_name.clone()
                        },
                        lesson_index,
                        title: it
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Aula")
                            .to_string(),
                        lesson_url: it
                            .get("href")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        vimeo_id: None,
                        attachments: Vec::new(),
                    });
                }
                _ => {}
            }
        }
    }
    eprintln!("[memberkit] {} aulas listadas em {} módulos", lessons.len(), module_index);

    // 3) Navegar cada aula (mesma sessão logada) e pegar o id do Vimeo.
    for lesson in lessons.iter_mut() {
        if lesson.lesson_url.is_empty() {
            continue;
        }
        if cdp_cmd(&mut ws, &cdp, "Page.navigate", serde_json::json!({ "url": lesson.lesson_url }))
            .await
            .is_err()
        {
            continue;
        }
        let _ = wait_page_settle(&mut ws, &cdp, 10).await;
        // O iframe do Vimeo às vezes carrega LAZY (depois do settle). Pollar por
        // até ~12s, dando scroll para disparar o carregamento preguiçoso.
        let mut vid = None;
        for _ in 0..14 {
            let st = probe_page(&mut ws, &cdp).await;
            vid = st
                .frames
                .iter()
                .find_map(|f| crate::engine::extractors::vimeo_id_from_url(f));
            if vid.is_some() {
                break;
            }
            let _ = cdp_eval(
                &mut ws,
                &cdp,
                "try{window.scrollTo(0,Math.min(400,document.body.scrollHeight));}catch(e){}; true",
            )
            .await;
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        }
        lesson.vimeo_id = vid;
        // Coletar anexos/materiais da aula (PDFs, código-fonte) na mesma sessão.
        if let Ok(att) = cdp_eval(&mut ws, &cdp, MEMBERKIT_ATTACH_JS).await {
            if let Some(arr) = att
                .as_str()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .and_then(|v| v.as_array().cloned())
            {
                for a in arr {
                    let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let url = a.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if !url.is_empty() {
                        lesson.attachments.push((name, url));
                    }
                }
            }
        }
        eprintln!(
            "[memberkit] {}.{} {} — vimeo={:?} anexos={}",
            lesson.module_index,
            lesson.lesson_index,
            lesson.title,
            lesson.vimeo_id,
            lesson.attachments.len()
        );
    }

    Ok(lessons)
}

/// Faz auto-login (se preciso) e então avalia `expr` na MESMA sessão do Chrome,
/// retornando o valor. Necessário para sites cujo cookie de sessão não persiste
/// no disco (ex. MemberKit): login e leitura precisam ser no mesmo Chrome.
pub async fn login_and_eval(
    profile_dir: &std::path::Path,
    url: &str,
    email: Option<&str>,
    password: Option<&str>,
    expr: &str,
) -> Result<serde_json::Value, String> {
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

    let result = async {
        let ws_url = wait_for_cdp(port, 20)
            .await
            .map_err(|e| format!("Chrome CDP não respondeu: {}", e))?;
        let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .map_err(|e| format!("Erro ao conectar CDP: {}", e))?;
        let cdp = CdpConn::new();
        let _ = cdp_cmd(&mut ws, &cdp, "Page.enable", serde_json::json!({})).await;
        let _ = cdp_cmd(&mut ws, &cdp, "Runtime.enable", serde_json::json!({})).await;
        login_navigate(&mut ws, &cdp, url, email, password).await?;
        cdp_eval(&mut ws, &cdp, expr).await
    }
    .await;

    let _ = child.kill().await;
    let _ = child.wait().await;
    result
}

/// Navega para `url`, faz auto-login por senha se cair no formulário e volta para
/// a página alvo. Compartilhado por extract_iframes e login_and_eval.
async fn login_navigate(
    ws: &mut Ws,
    cdp: &CdpConn,
    url: &str,
    email: Option<&str>,
    password: Option<&str>,
) -> Result<(), String> {
    cdp_cmd(ws, cdp, "Page.navigate", serde_json::json!({ "url": url })).await?;
    let mut state = wait_page_settle(ws, cdp, 20).await;
    if state.has_password && state.frames.is_empty() {
        match (email, password) {
            (Some(e), Some(p)) if !e.is_empty() && !p.is_empty() => {
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
                let _ = cdp_eval(ws, cdp, &fill).await;
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                let submit = r#"(function(){
                    var btn=document.querySelector('button[type=submit], input[type=submit]');
                    var form=document.querySelector('form');
                    if(btn){btn.click();return 'btn';}
                    if(form){form.submit();return 'form';}
                    return 'none';
                })()"#;
                let _ = cdp_eval(ws, cdp, submit).await;
                tokio::time::sleep(std::time::Duration::from_millis(3500)).await;
                cdp_cmd(ws, cdp, "Page.navigate", serde_json::json!({ "url": url })).await?;
                state = wait_page_settle(ws, cdp, 20).await;
                if state.has_password && state.frames.is_empty() {
                    return Err("Login falhou (credenciais inválidas ou captcha).".to_string());
                }
            }
            _ => return Err("Esta página exige login. Cadastre email e senha.".to_string()),
        }
    }
    Ok(())
}

// ============================================================================
// Farejador HLS autenticado (áreas de membros com player próprio, ex. Hotmart)
//
// Diferente do MemberKit (onde o iframe É o player do Vimeo e dá pra reprocessar
// o src), no Hotmart o master.m3u8 é buscado por JS dentro do iframe do player
// (cf-embed.play.hotmart.com) com URL assinada e exige Referer do domínio do
// embed. Então abrimos um Chrome VISÍVEL (perfil persistente = mantém login),
// fazemos login por senha se preciso e farejamos a rede atrás do .m3u8, guardando
// o Referer que o próprio Chrome mandou.
// ============================================================================

/// Stream HLS capturado ao vivo, com o Referer necessário para baixá-lo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HlsHit {
    pub m3u8_url: String,
    pub referer: Option<String>,
    pub title: Option<String>,
}

/// Abre um Chrome VISÍVEL com perfil persistente, faz login por senha quando
/// necessário, dá play e fareja a rede atrás do master `.m3u8` assinado.
/// A janela fica visível de propósito: reduz bloqueio anti-bot e deixa o usuário
/// concluir o login (ou clicar em play) manualmente se o automático não bastar.
pub async fn sniff_hls_authenticated(
    profile_dir: &std::path::Path,
    url: &str,
    email: Option<&str>,
    password: Option<&str>,
) -> Result<HlsHit, String> {
    let chrome_path =
        find_chrome().ok_or_else(|| "Chrome não encontrado no sistema.".to_string())?;
    let port = pick_debug_port();
    let _ = std::fs::create_dir_all(profile_dir);

    let mut child = Command::new(&chrome_path)
        .args([
            // Visível de propósito (sem --headless).
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-background-networking",
            "--disable-default-apps",
            "--disable-blink-features=AutomationControlled",
            &format!("--remote-debugging-port={}", port),
            &format!("--user-data-dir={}", profile_dir.display()),
            "--window-size=1280,900",
            "about:blank",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Erro ao iniciar Chrome: {}", e))?;

    let result = sniff_hls_inner(port, url, email, password).await;

    let _ = child.kill().await;
    let _ = child.wait().await;
    // NÃO remover o profile_dir: é persistente para manter o login.

    result
}

/// True se a página atual é uma tela de login (campo de senha ou host de SSO).
fn looks_like_login(state: &PageProbe) -> bool {
    let u = state.url.to_lowercase();
    state.has_password
        || u.contains("sso.hotmart")
        || u.contains("/login")
        || u.contains("account.hotmart")
        || u.contains("signin")
}

async fn sniff_hls_inner(
    port: u16,
    url: &str,
    email: Option<&str>,
    password: Option<&str>,
) -> Result<HlsHit, String> {
    let ws_url = wait_for_cdp(port, 20)
        .await
        .map_err(|e| format!("Chrome CDP não respondeu: {}", e))?;
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|e| format!("Erro ao conectar CDP: {}", e))?;

    let cdp = CdpConn::new();
    let _ = cdp_cmd(&mut ws, &cdp, "Page.enable", serde_json::json!({})).await;
    let _ = cdp_cmd(&mut ws, &cdp, "Runtime.enable", serde_json::json!({})).await;
    let _ = cdp_cmd(&mut ws, &cdp, "Network.enable", serde_json::json!({})).await;

    // Navegar para a aula e aguardar assentar.
    cdp_cmd(&mut ws, &cdp, "Page.navigate", serde_json::json!({ "url": url })).await?;
    let mut state = wait_page_settle(&mut ws, &cdp, 25).await;
    eprintln!(
        "[hotmart] inicial: ready={} pass={} url={} title={}",
        state.ready, state.has_password, state.url, state.title
    );

    // Se caiu no login: o SSO do Hotmart é email→senha em dois passos e tem
    // reCAPTCHA, então o automático é best-effort. Pré-preenchemos o que der para
    // adiantar e, se não concluir sozinho, MANTEMOS a janela aberta esperando o
    // usuário terminar o login (por senha). O perfil é persistente: só precisa da
    // 1ª vez; nas próximas já entra logado e nada disso roda.
    if looks_like_login(&state) {
        if let (Some(e), Some(p)) = (email, password) {
            if !e.is_empty() && !p.is_empty() {
                password_login(&mut ws, &cdp, e, p).await;
            }
        }

        // Reavaliar; se ainda no login, esperar conclusão manual (até 4 min).
        state = wait_page_settle(&mut ws, &cdp, 8).await;
        if looks_like_login(&state) {
            eprintln!("[hotmart] aguardando login manual na janela do Chrome...");
            state = wait_manual_login(&mut ws, &cdp, url, 240).await;
        }

        if looks_like_login(&state) {
            return Err(
                "Login do Hotmart não concluído a tempo. Deixe a janela do Chrome aberta, entre com email e SENHA (opção \"entrar com senha\") e tente o link de novo — o login fica salvo no app."
                    .to_string(),
            );
        }
        eprintln!("[hotmart] logado: url={} title={}", state.url, state.title);
    }

    let title = if state.title.trim().is_empty() {
        None
    } else {
        Some(state.title.clone())
    };

    // O player costuma buscar o m3u8 já no carregamento do iframe. Como os eventos
    // de rede durante o settle/eval são descartados, recarregamos a página AGORA e
    // só então entramos no loop de sniff — assim o fetch do master acontece com a
    // gente ouvindo. O play é reforçado periodicamente dentro do loop.
    let (_, reload_msg) = cdp.make_msg("Page.reload", serde_json::json!({ "ignoreCache": false }));
    let _ = ws.send(Message::Text(reload_msg.into())).await;

    // Farejar a rede atrás do .m3u8.
    let (m3u8_url, referer) = sniff_m3u8_loop(&mut ws, &cdp, 60).await.ok_or_else(|| {
        "Nenhum stream .m3u8 encontrado. Se o vídeo não tocou sozinho, clique em play na janela do Chrome que abriu e tente o link novamente."
            .to_string()
    })?;

    eprintln!("[hotmart] m3u8={} referer={:?}", m3u8_url, referer);
    Ok(HlsHit {
        m3u8_url,
        referer,
        title,
    })
}

/// Mantém a janela aberta e fica sondando até o usuário sair da tela de login.
/// Ao logar, navega de volta para a aula e retorna o estado dela. Se estourar o
/// tempo, retorna o último estado (ainda de login).
async fn wait_manual_login(ws: &mut Ws, cdp: &CdpConn, lesson_url: &str, max_secs: u64) -> PageProbe {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(max_secs);
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let p = probe_page(ws, cdp).await;
        if !looks_like_login(&p) {
            // Saiu do login. Garantir que estamos na página da aula.
            if !p.url.contains("/content/") {
                let _ = cdp_cmd(ws, cdp, "Page.navigate", serde_json::json!({ "url": lesson_url }))
                    .await;
                return wait_page_settle(ws, cdp, 25).await;
            }
            return p;
        }
        if tokio::time::Instant::now() >= deadline {
            return p;
        }
    }
}

/// Login por senha: preenche email/senha, força o fluxo "entrar com senha"
/// (evitando o de código) e submete, por algumas rodadas. Best-effort — a janela
/// é visível, então o usuário pode concluir/resolver captcha se necessário.
async fn password_login(ws: &mut Ws, cdp: &CdpConn, email: &str, password: &str) {
    for round in 0..4 {
        // Preferir a opção de senha antes de qualquer coisa.
        let _ = cdp_eval(ws, cdp, PREFER_PASSWORD_JS).await;
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        let fill = format!(
            r#"(function(){{
                var eF=document.querySelector('input[type=email], input[name*=email i], input[name=username], input[name=login], input[autocomplete=username], input[type=text]');
                var pF=document.querySelector('input[type=password]');
                var did=false;
                if(eF && !eF.value){{eF.value="{}";eF.dispatchEvent(new Event('input',{{bubbles:true}}));eF.dispatchEvent(new Event('change',{{bubbles:true}}));did=true;}}
                if(pF){{pF.value="{}";pF.dispatchEvent(new Event('input',{{bubbles:true}}));pF.dispatchEvent(new Event('change',{{bubbles:true}}));did=true;}}
                return did;
            }})()"#,
            json_escape(email),
            json_escape(password)
        );
        let _ = cdp_eval(ws, cdp, &fill).await;
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        let _ = cdp_eval(ws, cdp, SUBMIT_JS).await;
        // Aguardar o POST/redirect processar.
        tokio::time::sleep(std::time::Duration::from_millis(3500)).await;

        let probe = probe_page(ws, cdp).await;
        eprintln!(
            "[hotmart] login rodada {}: pass={} url={}",
            round, probe.has_password, probe.url
        );
        let u = probe.url.to_lowercase();
        let still_login = probe.has_password
            || u.contains("sso.hotmart")
            || u.contains("account.hotmart")
            || u.contains("/login");
        if !still_login {
            break;
        }
    }
}

/// Fareja eventos de rede atrás de um `.m3u8`, guardando o Referer que o Chrome
/// enviou. Prefere um master playlist; continua clicando em play; retorna o
/// primeiro bom achado (com uma folga de 2s para pegar o master se veio junto).
async fn sniff_m3u8_loop(
    ws: &mut Ws,
    cdp: &CdpConn,
    timeout_secs: u64,
) -> Option<(String, Option<String>)> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let mut best: Option<(String, Option<String>)> = None;
    let mut best_is_master = false;
    let mut found_at: Option<tokio::time::Instant> = None;
    let mut last_click = tokio::time::Instant::now();

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        // Depois de achar algo, esperar só mais 2s por um master melhor.
        if let Some(ft) = found_at {
            if best_is_master || ft.elapsed() > std::time::Duration::from_secs(2) {
                break;
            }
        }
        // Reclicar play a cada ~4s enquanto nada foi achado.
        if found_at.is_none() && last_click.elapsed() > std::time::Duration::from_secs(4) {
            last_click = tokio::time::Instant::now();
            let (_, msg) = cdp.make_msg(
                "Runtime.evaluate",
                serde_json::json!({ "expression": PLAY_JS, "userGesture": true }),
            );
            let _ = ws.send(Message::Text(msg.into())).await;
        }

        match tokio::time::timeout(
            remaining.min(std::time::Duration::from_millis(500)),
            ws.next(),
        )
        .await
        {
            Ok(Some(Ok(Message::Text(text)))) => {
                let msg: serde_json::Value = match serde_json::from_str(&text.to_string()) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
                let (u, referer, ct) = match method {
                    "Network.requestWillBeSent" => {
                        let p = msg.get("params");
                        let req = p.and_then(|p| p.get("request"));
                        let u = req
                            .and_then(|r| r.get("url"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let referer = req
                            .and_then(|r| r.get("headers"))
                            .and_then(|h| {
                                h.get("Referer").or_else(|| h.get("referer"))
                            })
                            .and_then(|v| v.as_str())
                            .map(String::from)
                            .or_else(|| {
                                p.and_then(|p| p.get("documentURL"))
                                    .and_then(|v| v.as_str())
                                    .map(String::from)
                            });
                        (u.to_string(), referer, String::new())
                    }
                    "Network.responseReceived" => {
                        let resp = msg.get("params").and_then(|p| p.get("response"));
                        let u = resp
                            .and_then(|r| r.get("url"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let ct = resp
                            .and_then(|r| r.get("mimeType"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        (u.to_string(), None, ct.to_string())
                    }
                    _ => continue,
                };

                if u.is_empty() || !is_stream_url(&u, &ct) {
                    continue;
                }
                // Só HLS (ignorar .ts/.mpd soltos já é feito por is_stream_url,
                // mas garantimos que seja m3u8).
                if !regex_lite::Regex::new(r"(?i)\.m3u8")
                    .unwrap()
                    .is_match(&u)
                {
                    continue;
                }

                let is_master = {
                    let lu = u.to_lowercase();
                    lu.contains("master") || lu.contains("playlist.m3u8")
                };
                eprintln!("[hotmart] m3u8 detectado (master={}): {}", is_master, u);

                if best.is_none() || (is_master && !best_is_master) {
                    best = Some((u, referer));
                    best_is_master = is_master;
                    if found_at.is_none() {
                        found_at = Some(tokio::time::Instant::now());
                    }
                }
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(_))) => break,
            Ok(None) => break,
            Err(_) => {}
        }
    }

    best
}

/// Força o fluxo de senha: clica em links/botões cujo texto sugere "entrar com
/// senha" (ignorando "esqueci"/"recuperar").
const PREFER_PASSWORD_JS: &str = r#"(function(){
    var els = Array.from(document.querySelectorAll('a,button,span,div,[role=button]'));
    for (var i=0;i<els.length;i++){
        var t = (els[i].textContent||'').trim().toLowerCase();
        if(!t) continue;
        if(t.indexOf('esquec')>=0 || t.indexOf('recuper')>=0 || t.indexOf('forgot')>=0) continue;
        if((t.indexOf('senha')>=0 && t.length<40) || t.indexOf('com senha')>=0 || t.indexOf('usar senha')>=0 || t.indexOf('password')>=0){
            try{ els[i].click(); return t; }catch(e){}
        }
    }
    return '';
})()"#;

/// Submete o formulário de login (botão submit ou form.submit()).
const SUBMIT_JS: &str = r#"(function(){
    var btn=document.querySelector('button[type=submit], input[type=submit]');
    if(btn){btn.click();return 'btn';}
    var byText=Array.from(document.querySelectorAll('button')).find(function(b){
        var t=(b.textContent||'').trim().toLowerCase();
        return t.indexOf('entrar')>=0||t.indexOf('acessar')>=0||t.indexOf('continuar')>=0||t.indexOf('login')>=0;
    });
    if(byText){byText.click();return 'text';}
    var form=document.querySelector('form');
    if(form){form.submit();return 'form';}
    return 'none';
})()"#;

/// Tenta iniciar a reprodução (dispara o carregamento do m3u8 nos players que
/// só buscam o playlist após o play).
const PLAY_JS: &str = r#"(function(){
    try{
        var v=document.querySelector('video');
        if(v){v.muted=true; v.play&&v.play().catch(function(){});}
    }catch(e){}
    var sel='[class*="play" i],[id*="play" i],[aria-label*="play" i],[title*="play" i],[class*="poster" i],[class*="thumb" i]';
    var btns=document.querySelectorAll(sel);
    for(var i=0;i<Math.min(btns.length,6);i++){try{btns[i].click();}catch(e){}}
    try{
        var el=document.elementFromPoint(window.innerWidth/2, window.innerHeight/2);
        if(el)el.click();
    }catch(e){}
    return true;
})()"#;

/// Escapa uma string para embutir com segurança dentro de aspas duplas em JS.
fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// Abre a página num Chrome headless com perfil persistente (já logado), aguarda
/// assentar, roda um JS de "preparo" (ex.: expandir módulos) com uma pausa e então
/// avalia o `extract_expr`, devolvendo o valor. Primitivo reutilizável para raspar
/// a estrutura de um curso.
pub async fn eval_on_page(
    profile_dir: &std::path::Path,
    url: &str,
    prep_expr: Option<&str>,
    prep_wait_ms: u64,
    extract_expr: &str,
    settle_secs: u64,
) -> Result<serde_json::Value, String> {
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
            "--window-size=1400,1000",
            "about:blank",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Erro ao iniciar Chrome: {}", e))?;

    let result = async {
        let ws_url = wait_for_cdp(port, 20)
            .await
            .map_err(|e| format!("Chrome CDP não respondeu: {}", e))?;
        let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .map_err(|e| format!("Erro ao conectar CDP: {}", e))?;
        let cdp = CdpConn::new();
        let _ = cdp_cmd(&mut ws, &cdp, "Page.enable", serde_json::json!({})).await;
        let _ = cdp_cmd(&mut ws, &cdp, "Runtime.enable", serde_json::json!({})).await;
        cdp_cmd(&mut ws, &cdp, "Page.navigate", serde_json::json!({ "url": url })).await?;
        let _ = wait_page_settle(&mut ws, &cdp, settle_secs).await;

        if let Some(prep) = prep_expr {
            let _ = cdp_eval(&mut ws, &cdp, prep).await;
            tokio::time::sleep(std::time::Duration::from_millis(prep_wait_ms)).await;
        }
        cdp_eval(&mut ws, &cdp, extract_expr).await
    }
    .await;

    let _ = child.kill().await;
    let _ = child.wait().await;
    result
}

/// Uma aula do curso (módulo, título, URL de conteúdo). O m3u8 é resolvido na
/// hora do download (link assinado expira em ~8min).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseLesson {
    pub module_index: usize,
    pub module_name: String,
    pub lesson_index: usize,
    pub title: String,
    pub content_url: String,
}

/// Lista a árvore do curso Hotmart Club (módulos→aulas em ordem) capturando o
/// `/v1/navigation` numa sessão headless logada. NÃO fareja vídeo — só monta a
/// lista ordenada, para o chamador farejar+baixar cada aula na hora.
pub async fn list_course(
    profile_dir: &std::path::Path,
    course_url: &str,
) -> Result<Vec<CourseLesson>, String> {
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
            "--window-size=1400,1000",
            "about:blank",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Erro ao iniciar Chrome: {}", e))?;

    let result = list_course_inner(port, course_url).await;
    let _ = child.kill().await;
    let _ = child.wait().await;
    result
}

/// Fareja o master m3u8 de UMA aula (Chrome headless, perfil já logado). Rápido,
/// usado imediatamente antes de baixar para o link assinado estar fresco.
pub async fn sniff_lesson(
    profile_dir: &std::path::Path,
    content_url: &str,
) -> Result<(String, Option<String>), String> {
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
            "--autoplay-policy=no-user-gesture-required",
            &format!("--remote-debugging-port={}", port),
            &format!("--user-data-dir={}", profile_dir.display()),
            "--window-size=1400,1000",
            "about:blank",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Erro ao iniciar Chrome: {}", e))?;

    let result = async {
        let ws_url = wait_for_cdp(port, 20)
            .await
            .map_err(|e| format!("CDP não respondeu: {}", e))?;
        let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .map_err(|e| format!("Erro CDP: {}", e))?;
        let cdp = CdpConn::new();
        let _ = cdp_cmd(&mut ws, &cdp, "Network.enable", serde_json::json!({})).await;
        let _ = cdp_cmd(&mut ws, &cdp, "Page.enable", serde_json::json!({})).await;
        cdp_cmd(&mut ws, &cdp, "Page.navigate", serde_json::json!({ "url": content_url })).await?;
        sniff_m3u8_loop(&mut ws, &cdp, 35)
            .await
            .ok_or_else(|| "m3u8 não capturado".to_string())
    }
    .await;

    let _ = child.kill().await;
    let _ = child.wait().await;
    result
}

async fn list_course_inner(port: u16, course_url: &str) -> Result<Vec<CourseLesson>, String> {
    let ws_url = wait_for_cdp(port, 20)
        .await
        .map_err(|e| format!("Chrome CDP não respondeu: {}", e))?;
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|e| format!("Erro ao conectar CDP: {}", e))?;
    let cdp = CdpConn::new();
    // Buffers grandes: a página faz 140+ requests e o corpo do /v1/navigation
    // (16KB) é despejado do buffer padrão antes de conseguirmos lê-lo.
    let _ = cdp_cmd(
        &mut ws,
        &cdp,
        "Network.enable",
        serde_json::json!({ "maxTotalBufferSize": 200_000_000, "maxResourceBufferSize": 100_000_000 }),
    )
    .await;
    let _ = cdp_cmd(&mut ws, &cdp, "Page.enable", serde_json::json!({})).await;

    // 1) Navegar ao curso e capturar o corpo de /v1/navigation.
    cdp_cmd(&mut ws, &cdp, "Page.navigate", serde_json::json!({ "url": course_url })).await?;
    let nav_body = capture_response_body_matching(&mut ws, &cdp, "/v1/navigation", 25)
        .await
        .ok_or_else(|| {
            "Não consegui capturar a navegação do curso (login expirado?).".to_string()
        })?;
    let nav: serde_json::Value =
        serde_json::from_str(&nav_body).map_err(|e| format!("Navegação inválida: {}", e))?;

    // Base da URL de conteúdo (troca o hash final).
    let base = course_url
        .split("/content/")
        .next()
        .map(|b| format!("{}/content/", b))
        .ok_or_else(|| "URL do curso sem /content/".to_string())?;

    // 2) Montar a lista ordenada de aulas com vídeo.
    let mut lessons: Vec<CourseLesson> = Vec::new();
    if let Some(modules) = nav.get("modules").and_then(|v| v.as_array()) {
        for (mi, module) in modules.iter().enumerate() {
            let mname = module
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Módulo")
                .to_string();
            if let Some(pages) = module.get("pages").and_then(|v| v.as_array()) {
                for (li, page) in pages.iter().enumerate() {
                    let has_media = page
                        .get("hasPlayerMedia")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let is_video = page
                        .get("firstMediaType")
                        .and_then(|v| v.as_str())
                        .map(|t| t.eq_ignore_ascii_case("VIDEO"))
                        .unwrap_or(false);
                    let title = page
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Aula")
                        .to_string();
                    let hash = page.get("hash").and_then(|v| v.as_str()).unwrap_or("");
                    if hash.is_empty() || !has_media || !is_video {
                        continue;
                    }
                    lessons.push(CourseLesson {
                        module_index: mi + 1,
                        module_name: mname.clone(),
                        lesson_index: li + 1,
                        title,
                        content_url: format!("{}{}", base, hash),
                    });
                }
            }
        }
    }
    eprintln!("[curso] {} aulas com vídeo encontradas", lessons.len());
    Ok(lessons)
}

/// Escuta responseReceived até achar uma URL contendo `needle`, então devolve o
/// corpo daquela resposta.
async fn capture_response_body_matching(
    ws: &mut Ws,
    cdp: &CdpConn,
    needle: &str,
    timeout_secs: u64,
) -> Option<String> {
    // Escutar a janela INTEIRA coletando o requestId do match (e parar cedo só
    // quando também virmos o loadingFinished dele, garantindo body pronto).
    // Coletar TODOS os requestIds cujas URLs casam. Fetch imediato no loadingFinished
    // de cada um (antes de despejo/eviction) — retorna o primeiro corpo não vazio.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let mut match_ids: Vec<String> = Vec::new();
    let mut resp_count = 0u32;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(
            remaining.min(std::time::Duration::from_millis(500)),
            ws.next(),
        )
        .await
        {
            Ok(Some(Ok(Message::Text(text)))) => {
                if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&text.to_string()) {
                    let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
                    let p = msg.get("params");
                    if method == "Network.responseReceived" {
                        resp_count += 1;
                        let u = p
                            .and_then(|p| p.get("response"))
                            .and_then(|r| r.get("url"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if u.contains(needle) {
                            if let Some(rid) = p
                                .and_then(|p| p.get("requestId"))
                                .and_then(|v| v.as_str())
                            {
                                eprintln!("[curso] navegação casou (rid={}): {}", rid, u);
                                match_ids.push(rid.to_string());
                            }
                        }
                    } else if method == "Network.loadingFinished" {
                        let rid = p
                            .and_then(|p| p.get("requestId"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if match_ids.iter().any(|m| m == rid) {
                            // Buscar o corpo AGORA, imediatamente após finalizar.
                            if let Some(body) = try_get_body(ws, cdp, rid).await {
                                eprintln!("[curso] corpo obtido no loadingFinished (rid={})", rid);
                                return Some(body);
                            }
                        }
                    }
                }
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(e))) => {
                eprintln!("[curso] ws erro: {}", e);
                break;
            }
            Ok(None) => {
                eprintln!("[curso] ws fechou (None)");
                break;
            }
            Err(_) => {}
        }
    }
    eprintln!(
        "[curso] fim escuta: resp_count={} matches={}",
        resp_count,
        match_ids.len()
    );
    // Fallback: tentar todos os ids coletados.
    for rid in match_ids.iter().rev() {
        if let Some(body) = try_get_body(ws, cdp, rid).await {
            return Some(body);
        }
    }
    None
}

/// Busca o corpo de uma resposta por requestId, decodificando base64 se preciso.
async fn try_get_body(ws: &mut Ws, cdp: &CdpConn, request_id: &str) -> Option<String> {
    let r = cdp_cmd(
        ws,
        cdp,
        "Network.getResponseBody",
        serde_json::json!({ "requestId": request_id }),
    )
    .await
    .ok()?;
    let body = r.get("body").and_then(|v| v.as_str()).unwrap_or("");
    if body.is_empty() {
        return None;
    }
    let is_b64 = r
        .get("base64Encoded")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if is_b64 {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(body)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
    } else {
        Some(body.to_string())
    }
}

/// Abre a página logada e captura os corpos das respostas JSON cujas URLs contêm
/// algum dos `needles`. Usado para pegar a API de navegação (árvore módulos/aulas)
/// do Hotmart Club sem ter que reproduzir o token de auth.
pub async fn capture_api_json(
    profile_dir: &std::path::Path,
    url: &str,
    needles: &[&str],
    settle_secs: u64,
) -> Result<Vec<(String, String)>, String> {
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
            "--window-size=1400,1000",
            "about:blank",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Erro ao iniciar Chrome: {}", e))?;

    let result = capture_api_json_inner(port, url, needles, settle_secs).await;
    let _ = child.kill().await;
    let _ = child.wait().await;
    result
}

async fn capture_api_json_inner(
    port: u16,
    url: &str,
    needles: &[&str],
    settle_secs: u64,
) -> Result<Vec<(String, String)>, String> {
    let ws_url = wait_for_cdp(port, 20)
        .await
        .map_err(|e| format!("Chrome CDP não respondeu: {}", e))?;
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|e| format!("Erro ao conectar CDP: {}", e))?;
    let cdp = CdpConn::new();
    let _ = cdp_cmd(&mut ws, &cdp, "Network.enable", serde_json::json!({})).await;
    let _ = cdp_cmd(&mut ws, &cdp, "Page.enable", serde_json::json!({})).await;
    cdp_cmd(&mut ws, &cdp, "Page.navigate", serde_json::json!({ "url": url })).await?;

    // Fase 1: escutar responseReceived e guardar (requestId, url) que casam.
    let mut matches: Vec<(String, String)> = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(settle_secs);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(
            remaining.min(std::time::Duration::from_millis(500)),
            ws.next(),
        )
        .await
        {
            Ok(Some(Ok(Message::Text(text)))) => {
                if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&text.to_string()) {
                    if msg.get("method").and_then(|v| v.as_str()) == Some("Network.responseReceived")
                    {
                        let p = msg.get("params");
                        let resp = p.and_then(|p| p.get("response"));
                        let u = resp
                            .and_then(|r| r.get("url"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let rid = p
                            .and_then(|p| p.get("requestId"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if !u.is_empty()
                            && !rid.is_empty()
                            && needles.iter().any(|n| u.contains(n))
                        {
                            matches.push((rid.to_string(), u.to_string()));
                        }
                    }
                }
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(_))) | Ok(None) => break,
            Err(_) => {}
        }
    }

    // Fase 2: buscar o corpo de cada resposta casada.
    let mut out = Vec::new();
    for (rid, u) in matches {
        if let Ok(r) = cdp_cmd(
            &mut ws,
            &cdp,
            "Network.getResponseBody",
            serde_json::json!({ "requestId": rid }),
        )
        .await
        {
            let body = r.get("body").and_then(|v| v.as_str()).unwrap_or("");
            let is_b64 = r
                .get("base64Encoded")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let decoded = if is_b64 {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .decode(body)
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
                    .unwrap_or_default()
            } else {
                body.to_string()
            };
            if !decoded.is_empty() {
                out.push((u, decoded));
            }
        }
    }
    Ok(out)
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
