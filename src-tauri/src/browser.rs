use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

/// Content script injetado em toda página carregada no browser.
/// Monitora fetch/XHR/PerformanceObserver buscando URLs de stream (m3u8, ts, mpd).
/// Os resultados ficam armazenados em window.__VI_DETECTED__ e são lidos via eval.
const CONTENT_SCRIPT: &str = r#"
(function() {
    if (window.__VI_INJECTED__) return;
    window.__VI_INJECTED__ = true;
    window.__VI_DETECTED__ = window.__VI_DETECTED__ || [];

    function report(url, source) {
        if (!url || typeof url !== 'string') return;
        if (url.startsWith('data:') || url.startsWith('blob:') || url.startsWith('mediasource:')) return;
        const dominated = window.__VI_DETECTED__.some(d => d.url === url);
        if (dominated) return;
        window.__VI_DETECTED__.push({ url, source, ts: Date.now() });
        console.log('[ViDownload] stream detectado:', source, url);

        // Enviar para o servidor de callback local (funciona sem IPC)
        try {
            new Image().src = 'http://127.0.0.1:17377/detected?url=' + encodeURIComponent(url) + '&source=' + encodeURIComponent(source);
        } catch(e) {}

        // Tentar IPC também (caso esteja disponível)
        try {
            if (window.__TAURI_INTERNALS__) {
                window.__TAURI_INTERNALS__.invoke('report_detected_stream', { url: url, source: source });
            }
        } catch(e) {}
    }

    function isStreamUrl(url) {
        if (typeof url !== 'string') return false;
        return /\.(m3u8|m3u|mpd|ts|m4s)(\?|$)/i.test(url) ||
               /mpegurl|dash\+xml/i.test(url);
    }

    // Interceptar fetch
    const origFetch = window.fetch;
    window.fetch = function(...args) {
        const input = args[0];
        const url = typeof input === 'string' ? input : input?.url || '';
        if (isStreamUrl(url)) report(url, 'fetch');
        return origFetch.apply(this, args);
    };

    // Interceptar XMLHttpRequest
    const origOpen = XMLHttpRequest.prototype.open;
    XMLHttpRequest.prototype.open = function(method, url, ...rest) {
        if (isStreamUrl(url)) report(url, 'xhr');
        return origOpen.apply(this, [method, url, ...rest]);
    };

    // PerformanceObserver para recursos carregados
    try {
        const obs = new PerformanceObserver(function(list) {
            for (const entry of list.getEntries()) {
                if (isStreamUrl(entry.name)) report(entry.name, 'resource');
            }
        });
        obs.observe({ entryTypes: ['resource'] });
    } catch(e) {}

    // Monitorar <video> e <source> elements
    const moObs = new MutationObserver(function(mutations) {
        for (const m of mutations) {
            for (const node of m.addedNodes) {
                if (node.tagName === 'VIDEO' || node.tagName === 'SOURCE') {
                    const src = node.src || node.getAttribute('src') || '';
                    if (isStreamUrl(src)) report(src, 'dom');
                }
                if (node.querySelectorAll) {
                    node.querySelectorAll('video[src], source[src]').forEach(function(el) {
                        const src = el.src || el.getAttribute('src') || '';
                        if (isStreamUrl(src)) report(src, 'dom');
                    });
                }
            }
        }
    });
    moObs.observe(document.documentElement, { childList: true, subtree: true });

    // Interceptar MediaSource.addSourceBuffer + SourceBuffer.appendBuffer
    // (Capture mode — pega dados que o player injeta no browser)
    try {
        if (window.MediaSource && !window.MediaSource.__vi_patched__) {
            const origAddSourceBuffer = MediaSource.prototype.addSourceBuffer;
            MediaSource.prototype.addSourceBuffer = function(mimeType) {
                const sb = origAddSourceBuffer.apply(this, arguments);
                const origAppendBuffer = sb.appendBuffer.bind(sb);
                sb.appendBuffer = function(data) {
                    if (data && (data.byteLength || data.length)) {
                        report('mediasource://' + mimeType + '/' + Date.now(), 'mediasource');
                    }
                    return origAppendBuffer(data);
                };
                return sb;
            };
            window.MediaSource.__vi_patched__ = true;
        }
    } catch(e) {}

    // Interceptar HTMLMediaElement.src (captura HLS nativo no WKWebView)
    try {
        const origSrcDesc = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'src');
        if (origSrcDesc && origSrcDesc.set) {
            const origSet = origSrcDesc.set;
            Object.defineProperty(HTMLMediaElement.prototype, 'src', {
                set: function(val) {
                    if (isStreamUrl(val)) report(val, 'video-src');
                    return origSet.call(this, val);
                },
                get: origSrcDesc.get,
                configurable: true
            });
        }
    } catch(e) {}

    // Interceptar HTMLSourceElement.src
    try {
        const origSrcDesc = Object.getOwnPropertyDescriptor(HTMLSourceElement.prototype, 'src');
        if (origSrcDesc && origSrcDesc.set) {
            const origSet = origSrcDesc.set;
            Object.defineProperty(HTMLSourceElement.prototype, 'src', {
                set: function(val) {
                    if (isStreamUrl(val)) report(val, 'source-src');
                    return origSet.call(this, val);
                },
                get: origSrcDesc.get,
                configurable: true
            });
        }
    } catch(e) {}

    // Polling: varrer <video> existentes a cada 2s
    setInterval(function() {
        document.querySelectorAll('video, source').forEach(function(el) {
            var s = el.src || el.currentSrc || el.getAttribute('src') || '';
            if (isStreamUrl(s)) report(s, 'poll');
        });
        // Também verificar player videojs
        try {
            if (window.videojs) {
                var players = videojs.getAllPlayers ? videojs.getAllPlayers() : [];
                players.forEach(function(p) {
                    var tech = p.tech_;
                    if (tech && tech.sourceHandler_ && tech.sourceHandler_.src) {
                        report(tech.sourceHandler_.src, 'videojs');
                    }
                    var src = p.currentSrc ? p.currentSrc() : '';
                    if (isStreamUrl(src)) report(src, 'videojs-src');
                });
            }
        } catch(e) {}
    }, 2000);

    console.log('[ViDownload] content script injetado');
})();
"#;

/// Estado compartilhado: se o browser está aberto
pub struct BrowserState {
    pub is_open: Mutex<bool>,
}

impl Default for BrowserState {
    fn default() -> Self {
        Self {
            is_open: Mutex::new(false),
        }
    }
}

#[tauri::command]
pub async fn open_browser(
    app: AppHandle,
    state: State<'_, BrowserState>,
    url: String,
) -> Result<(), String> {
    // Fechar browser existente se houver
    if let Some(win) = app.get_webview_window("browser") {
        let _ = win.close();
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    let parsed: url::Url = url.parse().map_err(|e: url::ParseError| e.to_string())?;

    let handle = app.clone();
    let _browser = WebviewWindowBuilder::new(&app, "browser", WebviewUrl::External(parsed))
        .title("ViDownload — Navegador")
        .inner_size(1200.0, 800.0)
        .center()
        .initialization_script(CONTENT_SCRIPT)
        .on_navigation(move |nav_url| {
            let _ = handle.emit("browser-navigated", nav_url.to_string());
            true // permitir toda navegação
        })
        .build()
        .map_err(|e| e.to_string())?;

    *state.is_open.lock().unwrap() = true;
    app.emit("browser-opened", ()).map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn close_browser(
    app: AppHandle,
    state: State<'_, BrowserState>,
) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("browser") {
        win.close().map_err(|e| e.to_string())?;
    }
    *state.is_open.lock().unwrap() = false;
    app.emit("browser-closed", ()).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn browser_navigate(app: AppHandle, url: String) -> Result<(), String> {
    let win = app
        .get_webview_window("browser")
        .ok_or("Navegador não está aberto")?;
    let parsed: url::Url = url.parse().map_err(|e: url::ParseError| e.to_string())?;
    win.navigate(parsed).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_back(app: AppHandle) -> Result<(), String> {
    let win = app
        .get_webview_window("browser")
        .ok_or("Navegador não está aberto")?;
    win.eval("history.back()").map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_forward(app: AppHandle) -> Result<(), String> {
    let win = app
        .get_webview_window("browser")
        .ok_or("Navegador não está aberto")?;
    win.eval("history.forward()").map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_reload(app: AppHandle) -> Result<(), String> {
    let win = app
        .get_webview_window("browser")
        .ok_or("Navegador não está aberto")?;
    win.eval("location.reload()").map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_get_url(app: AppHandle) -> Result<String, String> {
    let win = app
        .get_webview_window("browser")
        .ok_or("Navegador não está aberto")?;
    let url = win.url().map_err(|e| e.to_string())?;
    Ok(url.to_string())
}

/// Lê os streams detectados pelo content script e limpa a lista
#[tauri::command]
pub async fn browser_poll_detected(app: AppHandle) -> Result<(), String> {
    let win = app
        .get_webview_window("browser")
        .ok_or("Navegador não está aberto")?;

    // O eval injeta um script que lê __VI_DETECTED__, envia via document.title hack,
    // e limpa a lista. O evento browser-navigated será emitido com os dados.
    // Porém, eval() não retorna valor em Tauri 2.
    // Então usamos uma abordagem diferente: o content script já logou tudo.
    // Para Phase 2, o polling visual será feito na Phase 3 com network sniffing.
    // Por ora, o content script apenas detecta e loga no console do browser.
    let _ = win.eval(
        r#"
        if (window.__VI_DETECTED__ && window.__VI_DETECTED__.length > 0) {
            document.title = 'VI_STREAMS:' + JSON.stringify(window.__VI_DETECTED__) + ':VI_END|' + document.title;
            window.__VI_DETECTED__ = [];
        }
    "#,
    );

    Ok(())
}
