use serde::Deserialize;
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
    // Adicionar mais sites aqui conforme necessário
    None
}

/// Extractor UNIP — chama a API diretamente
async fn extract_unip(url: &str) -> Option<ExtractResult> {
    let parsed = url::Url::parse(url).ok()?;
    let params: HashMap<String, String> = parsed.query_pairs().map(|(k, v)| (k.to_string(), v.to_string())).collect();

    let video_id = params.get("id")?;
    let token = params.get("token")?;

    let api_url = format!(
        "https://api.unip.br/sistemas/ava/servico/video/transmissao/{}",
        video_id
    );

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15")
        .build()
        .ok()?;

    let response = client
        .get(&api_url)
        .header("Authorization", format!("Bearer {}", token))
        .header("ChavePublica", "HHMWqVULA0gtGujnz9J1x2LKTGaZxShrPiHfma1Jafu8QesvlE1RVEBPZZLL1NmIfEGpBFuTFtz6wq5IBTxsR4rTqxDuE4WHfWmV")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        eprintln!("[extractor:unip] API retornou {}", response.status());
        return None;
    }

    let body = response.text().await.ok()?;
    eprintln!("[extractor:unip] Resposta da API: {}", &body[..body.len().min(500)]);

    // Tentar parsear como JSON e encontrar a URL do vídeo
    // A resposta pode ter vários formatos, vamos tentar extrair qualquer URL de streaming
    if let Some(m3u8) = find_stream_url_in_json(&body) {
        return Some(ExtractResult {
            m3u8_url: m3u8,
            title: extract_json_field(&body, "titulo")
                .or_else(|| extract_json_field(&body, "nome"))
                .or_else(|| extract_json_field(&body, "title")),
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
