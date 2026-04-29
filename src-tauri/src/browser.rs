use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

/// Content script injetado em toda página carregada no browser.
const CONTENT_SCRIPT: &str = r#"
(function() {
    if (window.__VI_INJECTED__) return;
    window.__VI_INJECTED__ = true;
    window.__VI_DETECTED__ = [];
    window.__VI_ORIGINAL_TITLE__ = document.title;

    function isStreamUrl(url) {
        if (typeof url !== 'string') return false;
        return /\.(m3u8|m3u|mpd)(\?|$)/i.test(url) ||
               /mpegurl|dash\+xml/i.test(url) ||
               /manifest.*format.*m3u8/i.test(url);
    }

    function report(url, source) {
        if (!url || typeof url !== 'string') return;
        if (url.startsWith('data:') || url.startsWith('blob:')) return;
        if (window.__VI_DETECTED__.some(function(d) { return d.url === url; })) return;
        window.__VI_DETECTED__.push({ url: url, source: source });
        console.log('[ViDownload] detectado:', source, url);
    }

    // Interceptar fetch
    var origFetch = window.fetch;
    window.fetch = function() {
        var url = typeof arguments[0] === 'string' ? arguments[0] : (arguments[0] && arguments[0].url || '');
        if (isStreamUrl(url)) report(url, 'fetch');
        return origFetch.apply(this, arguments);
    };

    // Interceptar XMLHttpRequest
    var origOpen = XMLHttpRequest.prototype.open;
    XMLHttpRequest.prototype.open = function(method, url) {
        if (isStreamUrl(url)) report(url, 'xhr');
        return origOpen.apply(this, arguments);
    };

    // Interceptar video.src
    try {
        var desc = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'src');
        if (desc && desc.set) {
            var origSet = desc.set;
            Object.defineProperty(HTMLMediaElement.prototype, 'src', {
                set: function(val) { if (isStreamUrl(val)) report(val, 'video-src'); return origSet.call(this, val); },
                get: desc.get, configurable: true
            });
        }
    } catch(e) {}

    // PerformanceObserver
    try {
        new PerformanceObserver(function(list) {
            list.getEntries().forEach(function(e) { if (isStreamUrl(e.name)) report(e.name, 'resource'); });
        }).observe({ entryTypes: ['resource'] });
    } catch(e) {}

    // MutationObserver
    new MutationObserver(function(muts) {
        muts.forEach(function(m) {
            m.addedNodes.forEach(function(n) {
                if (n.tagName === 'VIDEO' || n.tagName === 'SOURCE') {
                    var s = n.src || n.getAttribute('src') || '';
                    if (isStreamUrl(s)) report(s, 'dom');
                }
                if (n.querySelectorAll) {
                    n.querySelectorAll('video[src],source[src]').forEach(function(el) {
                        var s = el.src || el.getAttribute('src') || '';
                        if (isStreamUrl(s)) report(s, 'dom');
                    });
                }
            });
        });
    }).observe(document.documentElement, { childList: true, subtree: true });

    // Polling video elements e videojs
    setInterval(function() {
        document.querySelectorAll('video').forEach(function(v) {
            ['src','currentSrc'].forEach(function(prop) {
                var s = v[prop]; if (s && isStreamUrl(s)) report(s, 'poll');
            });
        });
        try {
            if (window.videojs && videojs.getAllPlayers) {
                videojs.getAllPlayers().forEach(function(p) {
                    try {
                        var s = p.currentSrc(); if (s && isStreamUrl(s)) report(s, 'videojs');
                        var t = p.tech_; if (t) {
                            var vhs = t.vhs || t.hls;
                            if (vhs && vhs.playlists && vhs.playlists.master) {
                                var master = vhs.playlists.master;
                                if (master.uri) report(master.uri, 'vhs-master');
                                (master.playlists || []).forEach(function(pl) {
                                    if (pl.uri) report(pl.uri, 'vhs-variant');
                                });
                            }
                        }
                    } catch(e) {}
                });
            }
        } catch(e) {}
    }, 1500);

    console.log('[ViDownload] content script injetado');
})();
"#;

/// JS que extrai URLs detectadas e coloca no document.title
const EXTRACT_JS: &str = r#"
(function() {
    var urls = [];
    // De __VI_DETECTED__
    if (window.__VI_DETECTED__) {
        window.__VI_DETECTED__.forEach(function(d) { urls.push(d.url); });
    }
    // De video elements
    document.querySelectorAll('video').forEach(function(v) {
        ['src','currentSrc'].forEach(function(p) {
            var s = v[p];
            if (s && /\.(m3u8|m3u|mpd)(\?|$)/i.test(s)) urls.push(s);
        });
    });
    // De videojs
    try {
        if (window.videojs && videojs.getAllPlayers) {
            videojs.getAllPlayers().forEach(function(p) {
                try {
                    var s = p.currentSrc(); if (s && /m3u8|mpd/i.test(s)) urls.push(s);
                    var t = p.tech_; if (t && (t.vhs||t.hls)) {
                        var vhs = t.vhs||t.hls;
                        if (vhs.playlists && vhs.playlists.master && vhs.playlists.master.uri)
                            urls.push(vhs.playlists.master.uri);
                    }
                } catch(e) {}
            });
        }
    } catch(e) {}
    // Unique
    urls = urls.filter(function(v,i,a) { return a.indexOf(v) === i; });
    if (urls.length > 0) {
        document.title = 'VI_STREAMS:' + JSON.stringify(urls);
    }
})();
"#;

const RESTORE_TITLE_JS: &str = r#"
if (window.__VI_ORIGINAL_TITLE__) document.title = window.__VI_ORIGINAL_TITLE__;
"#;

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
            true
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
    let win = app.get_webview_window("browser").ok_or("Navegador não está aberto")?;
    let parsed: url::Url = url.parse().map_err(|e: url::ParseError| e.to_string())?;
    win.navigate(parsed).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_back(app: AppHandle) -> Result<(), String> {
    let win = app.get_webview_window("browser").ok_or("Navegador não está aberto")?;
    win.eval("history.back()").map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_forward(app: AppHandle) -> Result<(), String> {
    let win = app.get_webview_window("browser").ok_or("Navegador não está aberto")?;
    win.eval("history.forward()").map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_reload(app: AppHandle) -> Result<(), String> {
    let win = app.get_webview_window("browser").ok_or("Navegador não está aberto")?;
    win.eval("location.reload()").map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_get_url(app: AppHandle) -> Result<String, String> {
    let win = app.get_webview_window("browser").ok_or("Navegador não está aberto")?;
    let url = win.url().map_err(|e| e.to_string())?;
    Ok(url.to_string())
}

/// Polling: injeta JS que extrai streams e coloca no title, depois lê o title
#[tauri::command]
pub async fn browser_poll_detected(app: AppHandle) -> Result<Vec<String>, String> {
    let win = app.get_webview_window("browser").ok_or("Navegador não está aberto")?;

    // Injetar JS que coloca URLs no document.title
    win.eval(EXTRACT_JS).map_err(|e| e.to_string())?;

    // Esperar o JS executar
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // Ler o título
    let title = win.title().map_err(|e| e.to_string())?;

    // Restaurar título original
    let _ = win.eval(RESTORE_TITLE_JS);

    // Parsear URLs do título
    if title.starts_with("VI_STREAMS:") {
        let json_str = &title["VI_STREAMS:".len()..];
        if let Ok(urls) = serde_json::from_str::<Vec<String>>(json_str) {
            return Ok(urls);
        }
    }

    Ok(vec![])
}
