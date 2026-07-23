//! Teste de integração real do fluxo Hotmart Club → HLS.
//! Requer rede + Google Chrome instalado. Abre um Chrome VISÍVEL: se o login
//! automático não bastar, dá pra concluir na mão (login fica salvo no perfil).
//! Credenciais via env (senão usa as do teste da Laura):
//!   HOTMART_EMAIL, HOTMART_PASS, (opcional) HOTMART_URL
//!
//! Rodar explicitamente (não roda no `cargo test` normal):
//!   cargo test --test hotmart_e2e -- --ignored --nocapture

use vidownload_lib::chrome;
use vidownload_lib::engine::downloader;

const DEFAULT_URL: &str =
    "https://hotmart.com/pt-BR/club/linkedin-em-acao/products/4662418/content/0OvoWRZ54j";

#[tokio::test]
#[ignore]
async fn hotmart_hls_pipeline() {
    let email = std::env::var("HOTMART_EMAIL").ok();
    let password = std::env::var("HOTMART_PASS").ok();
    let url = std::env::var("HOTMART_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());

    // Perfil persistente isolado do teste (mantém o login entre execuções).
    let profile = std::env::temp_dir().join("vidownload-hotmart-e2e-profile");

    // 1. Chrome logado fareja o master m3u8 + o Referer do player.
    let hit = chrome::sniff_hls_authenticated(
        &profile,
        &url,
        email.as_deref(),
        password.as_deref(),
    )
    .await
    .expect("sniff_hls_authenticated falhou (login/DRM?)");

    println!("TITLE: {:?}", hit.title);
    println!("REFERER: {:?}", hit.referer);
    println!("M3U8: {}", &hit.m3u8_url[..hit.m3u8_url.len().min(140)]);
    assert!(hit.m3u8_url.contains("m3u8"), "URL capturada não é m3u8");

    // 2. Analisar carregando o Referer necessário (segmentos/chaves).
    let video = downloader::analyze_with_referer(&hit.m3u8_url, hit.referer.as_deref())
        .await
        .expect("analyze_with_referer falhou");

    println!("STREAMS: {}", video.streams.len());
    assert!(!video.streams.is_empty(), "sem variantes de qualidade");

    let best = &video.streams[0];
    println!(
        "BEST: {} | segs={} | dur={:.0}s | referer={:?}",
        best.quality,
        best.segments.len(),
        best.total_duration,
        best.download_referer,
    );
    assert!(!best.segments.is_empty(), "nenhum segmento parseado");
    assert_eq!(
        best.download_referer, hit.referer,
        "referer não propagado para o stream"
    );
}
