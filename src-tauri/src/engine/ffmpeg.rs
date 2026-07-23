/// Integração com ffmpeg para baixar e muxar streams HLS com áudio e vídeo
/// separados (ex. Vimeo), que a concatenação nativa não resolve.
///
/// O ffmpeg baixa as playlists diretamente (URLs assinadas do CDN), copia os
/// streams sem re-encodar (`-c copy`) e escreve um .mp4 final com faststart.
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Localiza o binário do ffmpeg no sistema.
pub fn find_ffmpeg() -> Option<String> {
    // Caminhos comuns (Homebrew Intel/ARM, MacPorts, Linux)
    let candidates = [
        "/opt/homebrew/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "/opt/local/bin/ffmpeg",
        "/usr/bin/ffmpeg",
    ];
    for c in &candidates {
        if Path::new(c).exists() {
            return Some(c.to_string());
        }
    }
    // Fallback: procurar no PATH
    if let Ok(out) = std::process::Command::new("which").arg("ffmpeg").output() {
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() {
                return Some(p);
            }
        }
    }
    None
}

/// Retorna true se o ffmpeg está disponível.
pub fn is_available() -> bool {
    find_ffmpeg().is_some()
}

/// Baixa e muxa via ffmpeg.
///
/// - `video_url`: playlist de vídeo (variante HLS) ou master.
/// - `audio_url`: playlist de áudio separada (opcional). Se `None`, assume que o
///   vídeo já contém áudio.
/// - `referer`: enviado como header HTTP quando o CDN exige.
/// - `total_duration`: duração em segundos, para calcular progresso.
/// - `on_progress`: callback com (segundos_processados) a cada atualização.
pub async fn download_mux<F: Fn(f64)>(
    video_url: &str,
    audio_url: Option<&str>,
    referer: Option<&str>,
    output: &Path,
    cancel: &AtomicBool,
    on_progress: F,
) -> Result<(), String> {
    let ffmpeg = find_ffmpeg().ok_or_else(|| {
        "ffmpeg não encontrado. Instale com: brew install ffmpeg".to_string()
    })?;

    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
    ];

    let header_arg = referer.map(|r| format!("Referer: {}\r\n", r));
    // Alguns CDNs (Hotmart) só entregam para um User-Agent de navegador; o UA
    // padrão do ffmpeg ("Lavf/...") toma 403. Enviamos o mesmo do reqwest.
    const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

    // Opções de rede aplicadas a cada input (propagadas aos segmentos HLS).
    let input_opts = |args: &mut Vec<String>| {
        args.push("-user_agent".into());
        args.push(BROWSER_UA.into());
        if let Some(ref h) = header_arg {
            args.push("-headers".into());
            args.push(h.clone());
        }
    };

    // Input 0: vídeo
    input_opts(&mut args);
    args.push("-i".into());
    args.push(video_url.to_string());

    // Input 1: áudio (opcional)
    if let Some(a) = audio_url {
        input_opts(&mut args);
        args.push("-i".into());
        args.push(a.to_string());
    }

    // Mapeamento de streams
    if audio_url.is_some() {
        args.push("-map".into());
        args.push("0:v:0".into());
        args.push("-map".into());
        args.push("1:a:0".into());
    } else {
        // Copiar todos os streams do único input
        args.push("-map".into());
        args.push("0".into());
    }

    args.extend([
        "-c".into(),
        "copy".into(),
        "-movflags".into(),
        "+faststart".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-nostats".into(),
        output.to_string_lossy().to_string(),
    ]);

    eprintln!("[ffmpeg] {} {}", ffmpeg, args.join(" "));

    let mut child = Command::new(&ffmpeg)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Erro ao iniciar ffmpeg: {}", e))?;

    let stdout = child.stdout.take().ok_or("Sem stdout do ffmpeg")?;
    let mut reader = BufReader::new(stdout).lines();

    // Ler linhas de progresso (`out_time_us=...`, `progress=continue|end`)
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            let _ = tokio::fs::remove_file(output).await;
            return Err("Download cancelado".to_string());
        }

        match tokio::time::timeout(std::time::Duration::from_millis(500), reader.next_line()).await
        {
            Ok(Ok(Some(line))) => {
                if let Some(us) = line.strip_prefix("out_time_us=") {
                    if let Ok(v) = us.trim().parse::<i64>() {
                        if v >= 0 {
                            on_progress(v as f64 / 1_000_000.0);
                        }
                    }
                } else if line.starts_with("progress=end") {
                    break;
                }
            }
            Ok(Ok(None)) => break, // stdout fechou
            Ok(Err(e)) => return Err(format!("Erro lendo progresso: {}", e)),
            Err(_) => {} // timeout de 500ms: re-checa cancelamento
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("Erro aguardando ffmpeg: {}", e))?;

    if !status.success() {
        // Capturar stderr para diagnóstico
        let mut err_out = String::new();
        if let Some(stderr) = child.stderr.take() {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                err_out.push_str(&l);
                err_out.push('\n');
                if err_out.len() > 2000 {
                    break;
                }
            }
        }
        return Err(format!(
            "ffmpeg falhou (código {}): {}",
            status.code().unwrap_or(-1),
            err_out.trim()
        ));
    }

    Ok(())
}
