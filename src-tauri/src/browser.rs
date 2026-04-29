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
        if (url.startsWith('data:') || url.startsWith('blob:')) return;
        const dominated = window.__VI_DETECTED__.some(d => d.url === url);
        if (dominated) return;
        window.__VI_DETECTED__.push({ url, source, ts: Date.now() });
        console.log('[ViDownload] stream detectado:', source, url);
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
