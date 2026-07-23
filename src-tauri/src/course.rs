//! Orquestra "baixar curso inteiro" para o app: lista a árvore (módulos/aulas +
//! anexos) e baixa a seleção em paralelo, organizada em pastas por módulo.
//! Reusa todo o motor já testado (chrome::sniff_memberkit_course/list_course/
//! sniff_lesson, extractors::extract_vimeo, downloader::analyze*, ffmpeg::download_mux).

use crate::config;
use crate::engine::{downloader, extractors, ffmpeg};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseAttachment {
    pub name: String,
    pub url: String,
    pub kind: String, // "pdf" | "file"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseLessonUI {
    pub key: String,
    pub module_index: usize,
    pub module_name: String,
    pub lesson_index: usize,
    pub title: String,
    pub has_video: bool,
    #[serde(default)]
    pub attachments: Vec<CourseAttachment>,
    // internos p/ resolver o vídeo no download
    #[serde(default)]
    pub vimeo_id: Option<String>,
    #[serde(default)]
    pub content_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseTree {
    pub platform: String, // "memberkit" | "hotmart"
    pub course_name: String,
    pub lessons: Vec<CourseLessonUI>,
}

/// Flag de cancelamento do download de curso em andamento.
#[derive(Default)]
pub struct CourseState {
    pub cancel: Arc<AtomicBool>,
}

fn app_config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|e| format!("Erro ao resolver dir de config: {}", e))
}

/// Remove caracteres inválidos de nome de arquivo/pasta, preservando acentos.
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

/// Raiz do curso MemberKit a partir de uma URL de aula: scheme://host/{id}-slug
fn memberkit_root(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let seg = parsed.path_segments()?.next()?; // "210366-projeto-caminhoes-s-a"
    Some(format!("{}://{}/{}", parsed.scheme(), host, seg))
}

/// Nome legível do curso a partir do slug da raiz (fallback de sugestão).
fn derive_name(root: &str) -> String {
    let slug = root.trim_end_matches('/').rsplit('/').next().unwrap_or("Curso");
    let no_id = slug.splitn(2, '-').nth(1).unwrap_or(slug);
    no_id
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn kind_of(url: &str) -> String {
    if url.split('?').next().unwrap_or(url).to_lowercase().ends_with(".pdf") {
        "pdf".to_string()
    } else {
        "file".to_string()
    }
}

/// Lista a árvore do curso (módulos → aulas + anexos) para a UI escolher.
#[tauri::command]
pub async fn list_course_tree(app: AppHandle, url: String) -> Result<CourseTree, String> {
    let cfg = app_config_dir(&app)?;
    let profile = cfg.join("chrome-profile");
    let host = config::host_of(&url).unwrap_or_default();

    let _ = app.emit("course-map-progress", "Mapeando curso…");

    if host.contains("memberkit.com.br") {
        let root = memberkit_root(&url).ok_or("URL MemberKit inválida")?;
        let (email, password) = config::find_for_host(&cfg, &host)
            .map(|c| (c.email, c.password))
            .unzip();
        let lessons =
            crate::chrome::sniff_memberkit_course(&profile, &root, email.as_deref(), password.as_deref())
                .await?;
        let ui: Vec<CourseLessonUI> = lessons
            .into_iter()
            .map(|l| CourseLessonUI {
                key: l.lesson_url.clone(),
                module_index: l.module_index,
                module_name: l.module_name,
                lesson_index: l.lesson_index,
                title: l.title,
                has_video: l.vimeo_id.is_some(),
                attachments: l
                    .attachments
                    .into_iter()
                    .map(|(name, u)| CourseAttachment {
                        kind: kind_of(&u),
                        name,
                        url: u,
                    })
                    .collect(),
                vimeo_id: l.vimeo_id,
                content_url: Some(l.lesson_url),
            })
            .collect();
        return Ok(CourseTree {
            platform: "memberkit".to_string(),
            course_name: derive_name(&root),
            lessons: ui,
        });
    }

    if host.contains("hotmart.com") {
        let lessons = crate::chrome::list_course(&profile, &url).await?;
        let ui: Vec<CourseLessonUI> = lessons
            .into_iter()
            .map(|l| CourseLessonUI {
                key: l.content_url.clone(),
                module_index: l.module_index,
                module_name: l.module_name,
                lesson_index: l.lesson_index,
                title: l.title,
                has_video: true,
                attachments: Vec::new(), // Hotmart: anexos baixados best-effort no download
                vimeo_id: None,
                content_url: Some(l.content_url),
            })
            .collect();
        return Ok(CourseTree {
            platform: "hotmart".to_string(),
            course_name: "Curso Hotmart".to_string(),
            lessons: ui,
        });
    }

    Err("Este link não é de um curso suportado (MemberKit/Hotmart).".to_string())
}

/// Cancela o download de curso em andamento.
#[tauri::command]
pub fn cancel_course(state: State<'_, CourseState>) -> Result<(), String> {
    state.cancel.store(true, Ordering::Relaxed);
    Ok(())
}

/// Baixa a seleção de aulas (vídeo + anexos opcionais), em paralelo, organizada
/// por módulo. Retorna imediatamente; o progresso vem por eventos.
#[tauri::command]
pub async fn download_course(
    app: AppHandle,
    state: State<'_, CourseState>,
    platform: String,
    lessons: Vec<CourseLessonUI>,
    output_dir: String,
    include_pdfs: bool,
    parallel: Option<usize>,
) -> Result<(), String> {
    let cancel = state.cancel.clone();
    cancel.store(false, Ordering::Relaxed);

    let cfg = app_config_dir(&app)?;
    let profile = cfg.join("chrome-profile");
    let out_root = PathBuf::from(&output_dir);
    std::fs::create_dir_all(&out_root).map_err(|e| format!("Erro ao criar pasta: {}", e))?;

    // Hotmart resolve via browser (perfil = lock único) → serial. MemberKit é paralelo.
    let par = if platform == "hotmart" {
        1
    } else {
        parallel.unwrap_or(5).clamp(1, 8)
    };

    let total = lessons.len();
    let handle = app.clone();

    tauri::async_runtime::spawn(async move {
        use futures_util::stream::{self, StreamExt};
        let done = Arc::new(AtomicUsize::new(0));
        let fail = Arc::new(AtomicUsize::new(0));

        stream::iter(lessons.into_iter().map(|lesson| {
            let app = handle.clone();
            let out_root = out_root.clone();
            let profile = profile.clone();
            let cancel = cancel.clone();
            let done = done.clone();
            let fail = fail.clone();
            let platform = platform.clone();
            async move {
                let key = lesson.key.clone();
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let emit = |status: &str, message: String| {
                    let _ = app.emit(
                        "course-progress",
                        serde_json::json!({
                            "key": key, "status": status, "message": message,
                            "done": done.load(Ordering::Relaxed), "total": total,
                        }),
                    );
                };

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

                // Anexos (PDF/materiais).
                if include_pdfs {
                    for att in &lesson.attachments {
                        let ext_name = sanitize(&att.name);
                        let apath = module_dir.join(format!("{:02}. {}", lesson.lesson_index, ext_name));
                        if !apath.exists() {
                            let _ = download_file(&att.url, &apath).await;
                        }
                    }
                }

                // Vídeo: pula se já existe.
                let exists =
                    file.exists() && std::fs::metadata(&file).map(|m| m.len() > 100_000).unwrap_or(false);
                if !lesson.has_video {
                    done.fetch_add(1, Ordering::Relaxed);
                    emit("done", "sem vídeo".to_string());
                    return;
                }
                if exists {
                    done.fetch_add(1, Ordering::Relaxed);
                    emit("skipped", "já existe".to_string());
                    return;
                }

                emit("downloading", format!("baixando {}", lesson.title));

                // Resolver o stream FRESCO (link assinado expira).
                let resolved: Result<(String, Option<String>, Option<String>), String> = async {
                    if platform == "memberkit" {
                        let vid = lesson.vimeo_id.clone().ok_or("sem vimeo")?;
                        let referer = lesson.content_url.clone();
                        let ex = extractors::extract_vimeo(&vid, referer.as_deref())
                            .await
                            .ok_or("extract_vimeo falhou (DRM?)")?;
                        Ok((ex.m3u8_url, None, None))
                    } else {
                        let content_url = lesson.content_url.clone().ok_or("sem content_url")?;
                        let (m3u8, referer) = crate::chrome::sniff_lesson(&profile, &content_url).await?;
                        Ok((m3u8, referer, None))
                    }
                }
                .await;

                let (m3u8, referer, _) = match resolved {
                    Ok(v) => v,
                    Err(e) => {
                        fail.fetch_add(1, Ordering::Relaxed);
                        emit("error", e);
                        return;
                    }
                };

                let video = if referer.is_some() {
                    downloader::analyze_with_referer(&m3u8, referer.as_deref()).await
                } else {
                    downloader::analyze(&m3u8).await
                };
                let video = match video {
                    Ok(v) => v,
                    Err(e) => {
                        fail.fetch_add(1, Ordering::Relaxed);
                        emit("error", format!("analyze: {}", e));
                        return;
                    }
                };
                let best = &video.streams[video.best_quality_index.min(video.streams.len().saturating_sub(1))];

                let tmp = file.with_extension("part.mp4");
                let _ = std::fs::remove_file(&tmp);
                let res = ffmpeg::download_mux(
                    &best.url,
                    best.audio_url.as_deref(),
                    best.download_referer.as_deref(),
                    &tmp,
                    &cancel,
                    |_s| {},
                )
                .await;
                match res {
                    Ok(()) => {
                        let _ = std::fs::rename(&tmp, &file);
                        done.fetch_add(1, Ordering::Relaxed);
                        emit("done", "ok".to_string());
                    }
                    Err(e) => {
                        let _ = std::fs::remove_file(&tmp);
                        fail.fetch_add(1, Ordering::Relaxed);
                        emit("error", format!("ffmpeg: {}", e));
                    }
                }
            }
        }))
        .buffer_unordered(par)
        .collect::<Vec<()>>()
        .await;

        let _ = handle.emit(
            "course-done",
            serde_json::json!({
                "ok": done.load(Ordering::Relaxed),
                "fail": fail.load(Ordering::Relaxed),
                "total": total,
            }),
        );
    });

    Ok(())
}

/// Baixa um arquivo simples (anexo) via reqwest.
async fn download_file(url: &str, path: &std::path::Path) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    tokio::fs::write(path, &bytes).await.map_err(|e| e.to_string())?;
    Ok(())
}
