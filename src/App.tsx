import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import UrlInput from "./components/UrlInput";
import StreamList from "./components/StreamList";
import SpinnerLoader from "./components/SpinnerLoader";
import PosterWall from "./components/PosterWall";
import DownloadProgressBar from "./components/DownloadProgress";
import type { VideoFound, DownloadProgress } from "./types";

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
  const pollIntervals = useRef<Map<string, ReturnType<typeof setInterval>>>(new Map());

  // Limpar intervals ao desmontar
  useEffect(() => {
    return () => {
      pollIntervals.current.forEach((interval) => clearInterval(interval));
    };
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

  const handleAnalyze = async (url: string) => {
    setLoading(true);
    setError(null);
    setVideo(null);
    setSelectedStreams(new Set());

    try {
      const result = await invoke<VideoFound>("analyze_url", { url });
      setVideo(result);
      setSelectedStreams(new Set([result.best_quality_index]));
    } catch {
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
                {["HLS", "m3u8", "MP4", "AES-128"].map((tag) => (
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
    </div>
  );
}

export default App;
