//! Probe: descobrir o endpoint de navegação (lista de módulos/aulas).
//!   cargo test --test hotmart_probe -- --ignored --nocapture

use vidownload_lib::chrome;

const URL: &str =
    "https://hotmart.com/pt-BR/club/linkedin-em-acao/products/4662418/content/0OvoWRZ54j";

#[tokio::test]
#[ignore]
async fn probe() {
    let profile = std::env::temp_dir().join("vidownload-hotmart-e2e-profile");
    let needles = ["cb.hotmart.com", "gateway", "navigation", "modules"];
    let bodies = chrome::capture_api_json(&profile, URL, &needles, 22)
        .await
        .expect("capture falhou");

    println!("=== {} respostas ===", bodies.len());
    for (u, body) in &bodies {
        // procurar bodies que listam módulos/páginas
        let looks_nav = body.contains("\"pages\"")
            || body.contains("\"modules\"")
            || (body.contains("\"module") && body.contains("\"hash\""))
            || u.contains("navigation")
            || u.contains("modules");
        println!(
            "URL: {}\n  LEN={} NAV?={}\n  {}",
            u,
            body.len(),
            looks_nav,
            &body[..body.len().min(if looks_nav { 2500 } else { 120 })]
        );
    }
}
