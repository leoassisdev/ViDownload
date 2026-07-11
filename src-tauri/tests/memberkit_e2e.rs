//! Teste de integração real do fluxo MemberKit → Vimeo.
//! Requer rede + Google Chrome instalado + credenciais via env:
//!   POTENZA_EMAIL, POTENZA_PASS, (opcional) POTENZA_URL
//!
//! Rodar explicitamente (não roda no `cargo test` normal):
//!   cargo test --test memberkit_e2e -- --ignored --nocapture

use std::sync::atomic::AtomicBool;
use vidownload_lib::chrome;
use vidownload_lib::engine::{downloader, extractors, ffmpeg};

const DEFAULT_URL: &str = "https://potenza.memberkit.com.br/205019-agentes-multimodais-com-openai/4262548-definindo-routers-rotas-no-fastapi";

#[tokio::test]
#[ignore]
async fn memberkit_vimeo_pipeline() {
    let email = std::env::var("POTENZA_EMAIL").ok();
    let password = std::env::var("POTENZA_PASS").ok();
    let url = std::env::var("POTENZA_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());

    assert!(
        email.is_some() && password.is_some(),
        "defina POTENZA_EMAIL e POTENZA_PASS"
    );

    // Perfil temporário isolado para o teste
    let profile = std::env::temp_dir().join("vidownload-e2e-profile");

    // 1. Login + achar iframes
    let frames = chrome::extract_iframes(&profile, &url, email.as_deref(), password.as_deref())
        .await
        .expect("extract_iframes falhou");
    println!("IFRAMES: {:#?}", frames);
    assert!(!frames.is_empty(), "nenhum iframe encontrado (login falhou?)");

    // 2. Achar o Vimeo
    let video_id = frames
        .iter()
        .find_map(|f| extractors::vimeo_id_from_url(f))
        .expect("nenhum iframe Vimeo");
    println!("VIMEO ID: {}", video_id);

    // 3. Extrair master m3u8 (Referer = página da aula)
    let extracted = extractors::extract_vimeo(&video_id, Some(&url))
        .await
        .expect("extract_vimeo falhou (DRM?)");
    println!("TITLE: {:?}", extracted.title);
    println!("MASTER: {}", &extracted.m3u8_url[..extracted.m3u8_url.len().min(120)]);
    assert!(extracted.m3u8_url.contains("m3u8"));

    // 4. Analisar o master → variantes com áudio separado
    let video = downloader::analyze(&extracted.m3u8_url)
        .await
        .expect("analyze do master falhou");
    println!("STREAMS: {}", video.streams.len());
    assert!(!video.streams.is_empty(), "sem variantes");

    let best = &video.streams[0];
    println!(
        "BEST: {} | audio_url={} | segs={} | dur={:.0}s",
        best.quality,
        best.audio_url.is_some(),
        best.segments.len(),
        best.total_duration
    );
    assert!(
        best.audio_url.is_some(),
        "faixa de áudio separada não detectada — sairia mudo"
    );
    assert!(best.total_duration > 1.0, "duração não parseada");

    // 5. Baixar de fato a MENOR qualidade via ffmpeg (rápido) e validar o .mp4
    if !ffmpeg::is_available() {
        println!("ffmpeg indisponível — pulando download");
        return;
    }
    let smallest = video.streams.last().unwrap();
    let out = std::env::temp_dir().join("vidownload-e2e-out.mp4");
    let _ = std::fs::remove_file(&out);
    let cancel = AtomicBool::new(false);
    ffmpeg::download_mux(
        &smallest.url,
        smallest.audio_url.as_deref(),
        smallest.download_referer.as_deref(),
        &out,
        &cancel,
        |secs| {
            if (secs as u64) % 60 == 0 {
                println!("  ...{}s processados", secs as u64);
            }
        },
    )
    .await
    .expect("download via ffmpeg falhou");

    let meta = std::fs::metadata(&out).expect("mp4 não gerado");
    println!("MP4 OK: {} bytes em {:?}", meta.len(), out);
    assert!(meta.len() > 200_000, "mp4 suspeitosamente pequeno");
}
