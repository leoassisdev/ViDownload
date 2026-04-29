use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedStreamCDP {
    pub url: String,
    #[serde(rename = "type")]
    pub stream_type: String,
    #[serde(rename = "contentType")]
    pub content_type: String,
}

#[derive(Debug, Deserialize)]
struct ScriptOutput {
    stream: Option<DetectedStreamCDP>,
    done: Option<bool>,
    streams: Option<Vec<DetectedStreamCDP>>,
    error: Option<String>,
}

/// Abre Chrome real via Puppeteer/CDP, navega até a URL,
/// intercepta rede e retorna URLs de m3u8 encontrados.
#[tauri::command]
pub async fn chrome_find_streams(
    app: AppHandle,
    url: String,
    timeout_secs: Option<u32>,
) -> Result<Vec<DetectedStreamCDP>, String> {
    let timeout = timeout_secs.unwrap_or(30).to_string();

    // Encontrar o diretório do projeto para achar o script
    let script_path = std::env::current_dir()
        .map(|d| d.join("scripts/find-stream.mjs"))
        .or_else(|_| {
            // Quando rodando via Tauri, o cwd pode ser diferente
            std::env::current_exe()
                .map(|e| e.parent().unwrap().parent().unwrap().parent().unwrap()
                    .join("scripts/find-stream.mjs"))
        })
        .map_err(|e| format!("Erro ao encontrar script: {}", e))?;

    // Tentar caminhos alternativos
    let script = if script_path.exists() {
        script_path
    } else {
        // Buscar relativo ao Cargo.toml (dev mode)
        let dev_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("scripts/find-stream.mjs");
        if dev_path.exists() {
            dev_path
        } else {
            return Err(format!("Script não encontrado em {:?}", script_path));
        }
    };

    eprintln!("[chrome] Rodando: node {} {} {}", script.display(), url, timeout);

    let mut child = Command::new("node")
        .arg(&script)
        .arg(&url)
        .arg(&timeout)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Erro ao iniciar Chrome/Node: {}", e))?;

    let stdout = child.stdout.take().ok_or("Sem stdout")?;
    let mut reader = BufReader::new(stdout).lines();
    let mut all_streams: Vec<DetectedStreamCDP> = Vec::new();

    // Ler output linha por linha (cada linha é um JSON)
    while let Some(line) = reader.next_line().await.map_err(|e| e.to_string())? {
        if let Ok(output) = serde_json::from_str::<ScriptOutput>(&line) {
            // Stream individual detectado
            if let Some(stream) = output.stream {
                eprintln!("[chrome] Stream encontrado: {}", stream.url);
                all_streams.push(stream.clone());
                let _ = app.emit("chrome-stream-found", stream);
            }

            // Resultado final
            if output.done == Some(true) {
                if let Some(streams) = output.streams {
                    for s in streams {
                        if !all_streams.iter().any(|existing| existing.url == s.url) {
                            all_streams.push(s);
                        }
                    }
                }
                break;
            }

            // Erro
            if let Some(err) = output.error {
                return Err(err);
            }
        }
    }

    // Esperar o processo terminar
    let _ = child.wait().await;

    if all_streams.is_empty() {
        Err("Nenhum stream encontrado na página".to_string())
    } else {
        Ok(all_streams)
    }
}
