import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import UrlInput from "./components/UrlInput";
import StreamList from "./components/StreamList";
import SpinnerLoader from "./components/SpinnerLoader";
import PosterWall from "./components/PosterWall";
import DownloadProgressBar from "./components/DownloadProgress";
import SettingsModal from "./components/SettingsModal";
import CourseModal from "./components/CourseModal";
import CourseProgress from "./components/CourseProgress";
import type { VideoFound, DownloadProgress, CourseTree, CourseLessonUI, CourseProgressEvt } from "./types";

interface DownloadItem {
  videoId: string;
  title: string;
  quality: string;
  progress: DownloadProgress | null;
}

function App() {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [video, setVideo] = useState<VideoFound | null>(null);
  const [selectedStreams, setSelectedStreams] = useState<Set<number>>(new Set());
  const [downloads, setDownloads] = useState<DownloadItem[]>([]);
  const [showSettings, setShowSettings] = useState(false);
  const [ffmpegOk, setFfmpegOk] = useState(true);
  const pollIntervals = useRef<Map<string, ReturnType<typeof setInterval>>>(new Map());

  // Curso (esteira de aulas)
  const [courseLink, setCourseLink] = useState<string | null>(null);
  const [course, setCourse] = useState<CourseTree | null>(null);
  const [mappingCourse, setMappingCourse] = useState(false);
  const [showCourseModal, setShowCourseModal] = useState(false);
  const [courseStatuses, setCourseStatuses] = useState<Map<string, string>>(new Map());
  const [courseTotals, setCourseTotals] = useState<{ done: number; total: number }>({ done: 0, total: 0 });
  const [courseRunning, setCourseRunning] = useState(false);
  const [courseFinished, setCourseFinished] = useState(false);

  // Limpar intervals ao desmontar
  useEffect(() => {
    return () => {
      pollIntervals.current.forEach((interval) => clearInterval(interval));
    };
  }, []);

  // Checar ffmpeg ao iniciar
  useEffect(() => {
    invoke<boolean>("check_ffmpeg").then(setFfmpegOk).catch(() => setFfmpegOk(true));
  }, []);

  // Escutar eventos de conclusão/erro
  useEffect(() => {
    const unlistenComplete = listen<{ video_id: string; path: string }>("download-complete", (e) => {
      const vid = e.payload.video_id;
      const interval = pollIntervals.current.get(vid);
      if (interval) { clearInterval(interval); pollIntervals.current.delete(vid); }
      setDownloads((prev) => prev.map((d) =>
        d.videoId === vid ? { ...d, progress: { ...d.progress!, state: "Done" } } : d
      ));
    });

    const unlistenError = listen<{ video_id: string; error: string }>("download-error", (e) => {
      const vid = e.payload.video_id;
      const interval = pollIntervals.current.get(vid);
      if (interval) { clearInterval(interval); pollIntervals.current.delete(vid); }
      setDownloads((prev) => prev.map((d) =>
        d.videoId === vid ? { ...d, progress: { ...d.progress!, state: { Error: e.payload.error } } } : d
      ));
    });

    return () => {
      unlistenComplete.then((fn) => fn());
      unlistenError.then((fn) => fn());
    };
  }, []);

  // Eventos do download de curso
  useEffect(() => {
    const unP = listen<CourseProgressEvt>("course-progress", (e) => {
      const { key, status, done, total } = e.payload;
      setCourseStatuses((prev) => new Map(prev).set(key, status));
      setCourseTotals({ done, total });
    });
    const unD = listen<{ ok: number; fail: number; total: number }>("course-done", (e) => {
      setCourseTotals({ done: e.payload.ok + e.payload.fail, total: e.payload.total });
      setCourseFinished(true);
      setCourseRunning(false);
    });
    return () => {
      unP.then((fn) => fn());
      unD.then((fn) => fn());
    };
  }, []);

  const handleOpenCourse = async (url: string) => {
    setMappingCourse(true);
    setError(null);
    try {
      const tree = await invoke<CourseTree>("list_course_tree", { url });
      if (!tree.lessons || tree.lessons.length === 0) {
        setError("Não encontrei aulas neste curso.");
        return;
      }
      setCourse(tree);
      setShowCourseModal(true);
    } catch (err) {
      setError(typeof err === "string" ? err : "Erro ao mapear o curso");
    } finally {
      setMappingCourse(false);
    }
  };

  const handleDownloadCourse = async (lessons: CourseLessonUI[], outputDir: string, includePdfs: boolean) => {
    if (!course) return;
    setShowCourseModal(false);
    setCourseStatuses(new Map());
    setCourseTotals({ done: 0, total: lessons.length });
    setCourseFinished(false);
    setCourseRunning(true);
    try {
      await invoke("download_course", {
        platform: course.platform,
        lessons,
        outputDir,
        includePdfs,
        parallel: 5,
      });
    } catch (err) {
      setError(typeof err === "string" ? err : "Erro ao iniciar o curso");
      setCourseRunning(false);
    }
  };

  const handleCancelCourse = async () => {
    await invoke("cancel_course").catch(() => {});
  };

  const handleAnalyze = async (url: string) => {
    setLoading(true);
    setError(null);
    setVideo(null);
    setSelectedStreams(new Set());
    // É link de curso (MemberKit/Hotmart)? Habilita o botão "Baixar curso".
    setCourseLink(/hotmart\.com|memberkit\.com\.br/i.test(url) ? url : null);

    try {
      const result = await invoke<VideoFound>("analyze_url", { url });
      setVideo(result);
      setSelectedStreams(new Set([result.best_quality_index]));
    } catch (directErr) {
      // Áreas de membros (Hotmart/MemberKit) já usam Chrome logado dentro do
      // analyze_url; o fallback headless (sem login) não ajudaria e só trocaria
      // a mensagem útil por uma genérica. Então mostramos o erro original.
      if (/hotmart\.com|memberkit\.com\.br/i.test(url)) {
        setError(typeof directErr === "string" ? directErr : JSON.stringify(directErr));
        return;
      }
      // Análise direta falhou — Chrome headless via CDP
      try {
        setError(null);
        const streams = await invoke<Array<{ url: string; type: string; contentType: string }>>(
          "chrome_find_streams", { url, timeoutSecs: 30 }
        );
        if (streams.length > 0) {
          const result = await invoke<VideoFound>("analyze_url", { url: streams[0].url });
          setVideo(result);
          setSelectedStreams(new Set([result.best_quality_index]));
        } else {
          setError("Nenhum stream encontrado");
        }
      } catch (chromeErr) {
        setError(typeof chromeErr === "string" ? chromeErr : JSON.stringify(chromeErr));
      }
    } finally {
      setLoading(false);
    }
  };

  const handleDownload = async () => {
    if (!video || selectedStreams.size === 0) return;

    const streamIdx = [...selectedStreams][0];
    const stream = video.streams[streamIdx];
    const title = video.title || "video";

    const filePath = await save({
      defaultPath: `${title}.${stream.init_segment_url ? "mp4" : "ts"}`,
      filters: [{ name: "Video", extensions: ["mp4", "ts", "mkv"] }],
    });

    if (!filePath) return;

    try {
      const videoId = await invoke<string>("start_download", {
        videoId: video.id,
        stream,
        outputPath: filePath,
      });

      // Adicionar à fila
      const newItem: DownloadItem = {
        videoId,
        title,
        quality: stream.quality,
        progress: {
          video_id: videoId,
          state: "Downloading",
          segments_done: 0,
          segments_total: stream.segments.length,
          bytes_downloaded: 0,
          speed_bps: 0,
          eta_seconds: 0,
        },
      };

      setDownloads((prev) => [...prev, newItem]);

      // Polling de progresso para este download
      const interval = setInterval(async () => {
        try {
          const p = await invoke<DownloadProgress>("get_download_progress", { videoId });
          setDownloads((prev) => prev.map((d) => (d.videoId === videoId ? { ...d, progress: p } : d)));
          if (p.state === "Done" || p.state === "Cancelled" || typeof p.state === "object") {
            clearInterval(interval);
            pollIntervals.current.delete(videoId);
          }
        } catch {}
      }, 500);
      pollIntervals.current.set(videoId, interval);

      // Limpar vídeo atual para permitir novo link
      setVideo(null);
      setSelectedStreams(new Set());
    } catch (err) {
      setError(typeof err === "string" ? err : "Erro ao iniciar download");
    }
  };

  const handleCancelDownload = async (videoId: string) => {
    await invoke("cancel_download", { videoId }).catch(() => {});
    const interval = pollIntervals.current.get(videoId);
    if (interval) { clearInterval(interval); pollIntervals.current.delete(videoId); }
    setDownloads((prev) => prev.filter((d) => d.videoId !== videoId));
  };

  const handleRemoveDownload = (videoId: string) => {
    setDownloads((prev) => prev.filter((d) => d.videoId !== videoId));
  };

  const handleToggleStream = (index: number) => {
    setSelectedStreams((prev) => {
      const next = new Set(prev);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  };

  const handleSelectAll = () => {
    if (!video) return;
    if (selectedStreams.size === video.streams.length) setSelectedStreams(new Set());
    else setSelectedStreams(new Set(video.streams.map((_, i) => i)));
  };

  const showEmptyState = !loading && !video && !error && downloads.length === 0;
  const activeDownloads = downloads.filter((d) => d.progress && d.progress.state !== "Done" && d.progress.state !== "Cancelled");

  return (
    <div className="min-h-screen bg-zinc-950 text-zinc-100 flex flex-col">
      {/* Header — sempre visível */}
      <header className="border-b border-zinc-800/60 bg-zinc-950/90 backdrop-blur-sm sticky top-0 z-20">
        <div className="max-w-5xl mx-auto px-6 py-4">
          <div className="flex items-center gap-3 mb-4">
            <img src="/app-icon.png" alt="ViDownload" className="w-9 h-9 rounded-lg shadow-lg shadow-blue-500/20" />
            <h1 className="text-xl font-bold text-white tracking-tight">ViDownload</h1>
            <span className="text-[10px] text-zinc-600 font-mono mt-1">v0.1.0</span>
            {activeDownloads.length > 0 && (
              <span className="ml-auto flex items-center gap-2 text-xs text-blue-400">
                <span className="w-2 h-2 rounded-full bg-blue-500 animate-pulse" />
                {activeDownloads.length} download{activeDownloads.length > 1 ? "s" : ""} ativo{activeDownloads.length > 1 ? "s" : ""}
              </span>
            )}
            <button
              onClick={() => setShowSettings(true)}
              className={`${activeDownloads.length > 0 ? "" : "ml-auto"} p-2 rounded-lg text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800/60 transition-colors relative`}
              title="Configurações"
            >
              {!ffmpegOk && <span className="absolute top-1 right-1 w-2 h-2 rounded-full bg-amber-500" />}
              <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.8}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M10.343 3.94c.09-.542.56-.94 1.11-.94h1.093c.55 0 1.02.398 1.11.94l.149.894c.07.424.384.764.78.93.398.164.855.142 1.205-.108l.737-.527a1.125 1.125 0 011.45.12l.773.774c.39.389.44 1.002.12 1.45l-.527.737c-.25.35-.272.806-.107 1.204.165.397.505.71.93.78l.893.15c.543.09.94.56.94 1.109v1.094c0 .55-.397 1.02-.94 1.11l-.893.149c-.425.07-.765.383-.93.78-.165.398-.143.854.107 1.204l.527.738c.32.447.269 1.06-.12 1.45l-.774.773a1.125 1.125 0 01-1.449.12l-.738-.527c-.35-.25-.806-.272-1.203-.107-.397.165-.71.505-.781.929l-.149.894c-.09.542-.56.94-1.11.94h-1.094c-.55 0-1.019-.398-1.11-.94l-.148-.894c-.071-.424-.384-.764-.781-.93-.398-.164-.854-.142-1.204.108l-.738.527a1.125 1.125 0 01-1.45-.12l-.773-.774a1.125 1.125 0 01-.12-1.45l.527-.737c.25-.35.273-.806.108-1.204-.165-.397-.505-.71-.93-.78l-.894-.15c-.542-.09-.94-.56-.94-1.109v-1.094c0-.55.398-1.02.94-1.11l.894-.149c.424-.07.765-.383.93-.78.165-.398.143-.854-.107-1.204l-.528-.738a1.125 1.125 0 01.12-1.45l.774-.773a1.125 1.125 0 011.45-.12l.737.527c.35.25.807.272 1.204.107.397-.165.71-.505.78-.929l.15-.894z" />
                <path strokeLinecap="round" strokeLinejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
              </svg>
            </button>
          </div>
          <UrlInput onAnalyze={handleAnalyze} loading={loading} />
        </div>
      </header>

      <main className="flex-1 relative">
        {/* Erro */}
        {error && (
          <div className="max-w-5xl mx-auto px-6 py-4">
            <div className="p-4 bg-red-950/40 border border-red-900/50 rounded-lg text-red-400 flex items-center gap-3">
              <svg className="w-5 h-5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m9-.75a9 9 0 11-18 0 9 9 0 0118 0zm-9 3.75h.008v.008H12v-.008z" />
              </svg>
              {error}
            </div>
          </div>
        )}

        {/* Banner: link faz parte de um curso */}
        {courseLink && !courseRunning && !courseFinished && (
          <div className="max-w-5xl mx-auto px-6 pt-4">
            <div className="p-4 bg-blue-950/30 border border-blue-900/40 rounded-lg flex items-center gap-3">
              <span className="text-2xl">📚</span>
              <div className="flex-1">
                <p className="text-sm text-white font-medium">Este link faz parte de um curso</p>
                <p className="text-xs text-zinc-400">Baixe o curso inteiro ou escolha as aulas (com módulos, títulos e materiais).</p>
              </div>
              <button
                onClick={() => handleOpenCourse(courseLink)}
                disabled={mappingCourse}
                className="px-4 py-2 rounded-lg text-sm font-semibold bg-gradient-to-r from-blue-600 to-red-600 text-white hover:opacity-90 disabled:opacity-50 flex items-center gap-2"
              >
                {mappingCourse ? (
                  <>
                    <svg className="w-4 h-4 animate-spin" viewBox="0 0 24 24" fill="none">
                      <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                      <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                    </svg>
                    Mapeando curso…
                  </>
                ) : (
                  "Baixar curso"
                )}
              </button>
            </div>
          </div>
        )}

        {/* Progresso do curso */}
        {course && (courseRunning || courseFinished) && (
          <CourseProgress
            tree={course}
            statuses={courseStatuses}
            done={courseTotals.done}
            total={courseTotals.total}
            finished={courseFinished}
            onCancel={handleCancelCourse}
            onClose={() => {
              setCourseRunning(false);
              setCourseFinished(false);
              setCourse(null);
              setCourseStatuses(new Map());
            }}
          />
        )}

        {/* Loading */}
        {loading && !video && <SpinnerLoader />}

        {/* Empty state — PosterWall */}
        {showEmptyState && (
          <div className="absolute inset-0 top-0">
            <PosterWall />
            <div className="absolute inset-0 bg-gradient-to-t from-zinc-950 via-zinc-950/60 to-zinc-950/30 pointer-events-none" />
            <div className="absolute inset-0 flex flex-col items-center justify-center pointer-events-none z-10">
              <img src="/app-icon.png" alt="ViDownload" className="w-24 h-24 rounded-2xl shadow-2xl shadow-blue-500/30 mb-6" />
              <h2 className="text-4xl font-bold text-white tracking-tight mb-2 drop-shadow-lg">ViDownload</h2>
              <p className="text-zinc-400 text-lg drop-shadow-md">Cole um link para baixar qualquer vídeo</p>
              <div className="flex gap-3 mt-4">
                {["HLS", "m3u8", "MemberKit", "Vimeo", "AES-128"].map((tag) => (
                  <span key={tag} className="px-3 py-1 text-xs bg-zinc-800/80 text-zinc-400 rounded-full border border-zinc-700/50">{tag}</span>
                ))}
              </div>
            </div>
          </div>
        )}

        {/* Seleção de stream (antes de iniciar download) */}
        {video && (
          <div className="max-w-5xl mx-auto px-6 py-6">
            <StreamList
              video={video}
              selectedStreams={selectedStreams}
              onToggleStream={handleToggleStream}
              onSelectAll={handleSelectAll}
              onDownload={handleDownload}
            />
          </div>
        )}

        {/* Fila de downloads */}
        {downloads.length > 0 && (
          <div className="max-w-5xl mx-auto px-6 py-4 space-y-3">
            <h3 className="text-sm font-semibold text-zinc-400 uppercase tracking-wider">
              Downloads ({downloads.length})
            </h3>
            {downloads.map((item) => (
              <div key={item.videoId} className="relative">
                <div className="flex items-center gap-3 mb-1">
                  <span className="text-sm text-white font-medium truncate flex-1">{item.title}</span>
                  <span className="text-xs text-zinc-500">{item.quality}</span>
                  {item.progress?.state === "Done" && (
                    <button
                      onClick={() => handleRemoveDownload(item.videoId)}
                      className="text-xs text-zinc-600 hover:text-zinc-400"
                    >
                      Remover
                    </button>
                  )}
                </div>
                {item.progress && (
                  <DownloadProgressBar
                    progress={item.progress}
                    onCancel={() => handleCancelDownload(item.videoId)}
                  />
                )}
              </div>
            ))}
          </div>
        )}
      </main>

      <SettingsModal open={showSettings} onClose={() => setShowSettings(false)} ffmpegOk={ffmpegOk} />
      {course && (
        <CourseModal
          open={showCourseModal}
          tree={course}
          onClose={() => setShowCourseModal(false)}
          onDownload={handleDownloadCourse}
        />
      )}
    </div>
  );
}

export default App;
