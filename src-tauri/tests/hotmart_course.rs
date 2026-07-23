//! Baixa o curso Hotmart inteiro, organizado por módulo, com títulos corretos.
//!   HOTMART_URL=... HOTMART_OUT=... [HOTMART_LIMIT=N] \
//!     cargo test --test hotmart_course -- --ignored --nocapture
//!
//! Usa o perfil já logado. Fareja+baixa cada aula na hora (link assinado ~8min).

use std::sync::atomic::AtomicBool;
use vidownload_lib::chrome::{self, CourseLesson};
use vidownload_lib::engine::{downloader, ffmpeg};

const DEFAULT_URL: &str =
    "https://hotmart.com/pt-BR/club/linkedin-em-acao/products/4662418/content/0OvoWRZ54j";

fn sanitize(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if (c as u32) < 0x20 => ' ',
            c => c,
        })
        .collect();
    cleaned.trim().trim_end_matches('.').trim().to_string()
}

#[tokio::test]
#[ignore]
async fn baixar_curso() {
    let url = std::env::var("HOTMART_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());
    let profile = std::env::temp_dir().join("vidownload-hotmart-e2e-profile");
    let limit: usize = std::env::var("HOTMART_LIMIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);

    assert!(ffmpeg::is_available(), "ffmpeg necessário: brew install ffmpeg");

    // 1) Listar a árvore do curso (uma sessão Chrome, rápido).
    println!("Mapeando curso...");
    let lessons: Vec<CourseLesson> = chrome::list_course(&profile, &url)
        .await
        .expect("list_course falhou");

    let course_name = "LinkedIn em Ação";
    let out_root = std::env::var("HOTMART_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| dirs_download().join(sanitize(course_name)));
    std::fs::create_dir_all(&out_root).expect("criar pasta raiz");
    println!("Saída: {}", out_root.display());
    println!("{} aulas com vídeo.\n", lessons.len());

    let mut ok = 0usize;
    let mut fail: Vec<String> = Vec::new();

    for (i, lesson) in lessons.iter().enumerate() {
        if i >= limit {
            println!("(limite {} atingido, parando)", limit);
            break;
        }
        let module_dir = out_root.join(format!(
            "{:02}. {}",
            lesson.module_index,
            sanitize(&lesson.module_name)
        ));
        let _ = std::fs::create_dir_all(&module_dir);
        let file = module_dir.join(format!(
            "{:02}. {}.mp4",
            lesson.lesson_index,
            sanitize(&lesson.title)
        ));
        let tag = format!(
            "M{:02} A{:02} {}",
            lesson.module_index, lesson.lesson_index, lesson.title
        );

        if file.exists() && std::fs::metadata(&file).map(|m| m.len() > 100_000).unwrap_or(false) {
            println!("• já existe, pulando: {}", tag);
            ok += 1;
            continue;
        }

        // 2) Farejar o m3u8 FRESCO desta aula.
        let (m3u8, referer) = match chrome::sniff_lesson(&profile, &lesson.content_url).await {
            Ok(v) => v,
            Err(e) => {
                println!("✗ {} — sniff: {}", tag, e);
                fail.push(format!("{} (sniff: {})", tag, e));
                continue;
            }
        };

        // 3) Analisar e pegar a melhor qualidade.
        let video = match downloader::analyze_with_referer(&m3u8, referer.as_deref()).await {
            Ok(v) => v,
            Err(e) => {
                println!("✗ {} — analyze: {}", tag, e);
                fail.push(format!("{} (analyze: {})", tag, e));
                continue;
            }
        };
        let best = &video.streams[video.best_quality_index.min(video.streams.len() - 1)];

        // 4) Baixar direto pra .mp4 via ffmpeg.
        print!("↓ {} [{}] ... ", tag, best.quality);
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let cancel = AtomicBool::new(false);
        let tmp = file.with_extension("part.mp4");
        let _ = std::fs::remove_file(&tmp);
        let res = ffmpeg::download_mux(
            &best.url,
            best.audio_url.as_deref(),
            best.download_referer.as_deref(),
            &tmp,
            &cancel,
            |_secs| {},
        )
        .await;

        match res {
            Ok(()) => {
                let _ = std::fs::rename(&tmp, &file);
                let sz = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
                println!("OK ({:.1} MB)", sz as f64 / 1_048_576.0);
                ok += 1;
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                println!("FALHOU: {}", e);
                fail.push(format!("{} (ffmpeg: {})", tag, e));
            }
        }
    }

    println!("\n=== FIM: {} ok, {} falhas ===", ok, fail.len());
    for f in &fail {
        println!("  ✗ {}", f);
    }
    assert!(ok > 0, "nenhuma aula baixada");
}

fn dirs_download() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join("Downloads"))
        .unwrap_or_else(|_| std::env::temp_dir())
}
