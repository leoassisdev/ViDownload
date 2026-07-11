use std::collections::HashMap;

/// Resultado de um extractor
pub struct ExtractResult {
    pub m3u8_url: String,
    pub title: Option<String>,
    pub headers: HashMap<String, String>,
}

/// Tenta extrair o m3u8 direto de sites conhecidos (sem precisar de browser)
pub async fn try_extract(url: &str) -> Option<ExtractResult> {
    if url.contains("sistemas.unip.br") || url.contains("unip.br/ava") {
        return extract_unip(url).await;
    }
    // URL direta de player Vimeo (player.vimeo.com/video/{id})
    if let Some(id) = vimeo_id_from_url(url) {
        return extract_vimeo(&id, None).await;
    }
    // Adicionar mais sites aqui conforme necessário
    None
}

/// Extrai o ID numérico de uma URL de player do Vimeo.
pub fn vimeo_id_from_url(url: &str) -> Option<String> {
    let re = regex_lite::Regex::new(r"(?:player\.)?vimeo\.com/(?:video/)?(\d+)").ok()?;
    re.captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Extractor Vimeo — busca o HTML do player com o Referer do site que embeda
/// (necessário para vídeos com privacidade por domínio, ex. MemberKit) e lê o
/// config inline, que contém o master.m3u8 assinado. Não requer DRM/token extra.
pub async fn extract_vimeo(video_id: &str, referer: Option<&str>) -> Option<ExtractResult> {
    let player_url = format!("https://player.vimeo.com/video/{}", video_id);

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .build()
        .ok()?;

    let mut req = client.get(&player_url);
    if let Some(r) = referer {
        req = req.header("Referer", r);
    }

    eprintln!("[extractor:vimeo] GET {} (referer={:?})", player_url, referer);
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        eprintln!("[extractor:vimeo] player retornou {}", resp.status());
        return None;
    }
    let html = resp.text().await.ok()?;

    let config = extract_inline_config(&html)?;
    let json: serde_json::Value = serde_json::from_str(&config).ok()?;

    // DRM: se marcado, não conseguimos baixar
    if json.pointer("/request/drm").map(|v| !v.is_null()).unwrap_or(false) {
        eprintln!("[extractor:vimeo] vídeo protegido por DRM — não suportado");
        return None;
    }

    let files = json.pointer("/request/files")?;
    let hls = files.get("hls")?;
    let default_cdn = hls.get("default_cdn").and_then(|v| v.as_str());
    let cdns = hls.get("cdns")?.as_object()?;

    // Pegar a URL do CDN default; senão a primeira disponível
    let m3u8_url = default_cdn
        .and_then(|c| cdns.get(c))
        .or_else(|| cdns.values().next())
        .and_then(|c| c.get("url"))
        .and_then(|v| v.as_str())?
        .to_string();

    let title = json
        .pointer("/video/title")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    eprintln!("[extractor:vimeo] master m3u8 OK ({} chars)", m3u8_url.len());

    let mut headers = HashMap::new();
    if let Some(r) = referer {
        headers.insert("referer".to_string(), r.to_string());
    }

    Some(ExtractResult {
        m3u8_url,
        title,
        headers,
    })
}

/// Extrai o objeto JSON de `window.playerConfig = {...}` (ou `var config = {...}`)
/// do HTML do player Vimeo, casando as chaves `{}` respeitando strings.
fn extract_inline_config(html: &str) -> Option<String> {
    let re = regex_lite::Regex::new(r"(?:window\.playerConfig|var\s+config)\s*=\s*\{").ok()?;
    let m = re.find(html)?;
    let start = m.end() - 1; // posição do '{'
    let bytes = html.as_bytes();
    let mut depth = 0usize;
    let mut in_str = false;
    let mut esc = false;
    let mut i = start;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
        } else {
            match c {
                '"' => in_str = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(html[start..=i].to_string());
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// Extractor UNIP — chama a API tvweb3 diretamente (não requer auth)
async fn extract_unip(url: &str) -> Option<ExtractResult> {
    let parsed = url::Url::parse(url).ok()?;
    let params: HashMap<String, String> = parsed.query_pairs().map(|(k, v)| (k.to_string(), v.to_string())).collect();

    let video_id = params.get("id")?;

    // API pública do tvweb3 — retorna JSON com midias[].local contendo o m3u8
    let api_url = format!("https://tvweb3.unip.br/api/transmissao/{}", video_id);

    eprintln!("[extractor:unip] Chamando API: {}", api_url);

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .build()
        .ok()?;

    let response = client
        .get(&api_url)
        .header("Accept", "application/json")
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        eprintln!("[extractor:unip] API retornou {}", response.status());
        return None;
    }

    let body = response.text().await.ok()?;
    eprintln!("[extractor:unip] Resposta: {}", &body[..body.len().min(500)]);

    // Parsear JSON e extrair m3u8 de midias[].local
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
        // Extrair título
        let title = json.get("titulo")
            .or_else(|| json.get("nome"))
            .or_else(|| json.get("title"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Buscar m3u8 em midias[].local
        if let Some(midias) = json.get("midias").and_then(|v| v.as_array()) {
            for midia in midias {
                if let Some(url_midia) = midia.get("local").and_then(|v| v.as_str()) {
                    if url_midia.contains(".m3u8") || url_midia.contains("m3u8") {
                        eprintln!("[extractor:unip] m3u8 encontrado: {}", url_midia);
                        return Some(ExtractResult {
                            m3u8_url: url_midia.to_string(),
                            title,
                            headers: HashMap::new(),
                        });
                    }
                }
            }
            // Se nenhum campo local tem m3u8, pegar o primeiro que existir
            for midia in midias {
                if let Some(url_midia) = midia.get("local").and_then(|v| v.as_str()) {
                    if !url_midia.is_empty() {
                        eprintln!("[extractor:unip] mídia encontrada: {}", url_midia);
                        return Some(ExtractResult {
                            m3u8_url: url_midia.to_string(),
                            title,
                            headers: HashMap::new(),
                        });
                    }
                }
            }
        }

        // Fallback: buscar qualquer URL m3u8 no JSON inteiro
        if let Some(m3u8) = find_stream_url_in_json(&body) {
            return Some(ExtractResult {
                m3u8_url: m3u8,
                title,
                headers: HashMap::new(),
            });
        }
    }

    // Fallback final: regex no body
    if let Some(m3u8) = find_stream_url_in_json(&body) {
        return Some(ExtractResult {
            m3u8_url: m3u8,
            title: extract_json_field(&body, "titulo")
                .or_else(|| extract_json_field(&body, "nome")),
            headers: HashMap::new(),
        });
    }

    None
}

/// Busca URL de streaming em um JSON (qualquer campo que contenha m3u8, manifest, streaming, etc)
fn find_stream_url_in_json(json: &str) -> Option<String> {
    // Buscar URLs que parecem ser streams
    let patterns = [
        r#""(https?://[^"]*\.m3u8[^"]*?)""#,
        r#""(https?://[^"]*manifest[^"]*?)""#,
        r#""(https?://[^"]*\.ism/manifest[^"]*?)""#,
        r#""(https?://[^"]*streaming[^"]*?)""#,
        r#""(https?://[^"]*\.mpd[^"]*?)""#,
        r#""(https?://[^"]*media\.azure[^"]*?)""#,
        r#""(https?://[^"]*azureedge[^"]*?)""#,
        r#""(https?://[^"]*blob\.core[^"]*?)""#,
    ];

    for pattern in &patterns {
        if let Ok(re) = regex_lite::Regex::new(pattern) {
            if let Some(cap) = re.captures(json) {
                if let Some(url) = cap.get(1) {
                    let url_str = url.as_str().to_string();
                    // Se não termina em m3u8, tentar adicionar o formato
                    if url_str.contains(".ism") && !url_str.contains("format=") {
                        return Some(format!("{}(format=m3u8-cmaf)", url_str));
                    }
                    return Some(url_str);
                }
            }
        }
    }

    // Fallback: buscar qualquer URL http que contenha video/media/stream
    if let Ok(re) = regex_lite::Regex::new(r#""(https?://[^"]{20,}?)""#) {
        for cap in re.captures_iter(json) {
            if let Some(url) = cap.get(1) {
                let u = url.as_str();
                if u.contains("video") || u.contains("media") || u.contains("stream") || u.contains("azure") {
                    return Some(u.to_string());
                }
            }
        }
    }

    None
}

fn extract_json_field(json: &str, field: &str) -> Option<String> {
    let pattern = format!(r#""{}"\s*:\s*"([^"]+?)""#, field);
    if let Ok(re) = regex_lite::Regex::new(&pattern) {
        if let Some(cap) = re.captures(json) {
            return cap.get(1).map(|m| m.as_str().to_string());
        }
    }
    None
}
