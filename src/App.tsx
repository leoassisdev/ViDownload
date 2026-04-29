import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import UrlInput from "./components/UrlInput";
import StreamList from "./components/StreamList";
import TerminalLoader from "./components/TerminalLoader";
import CubeLoader from "./components/CubeLoader";
import PosterWall from "./components/PosterWall";
import BrowserBar from "./components/BrowserBar";
import type { VideoFound, DownloadProgress } from "./types";

function App() {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [video, setVideo] = useState<VideoFound | null>(null);
  const [selectedStreams, setSelectedStreams] = useState<Set<number>>(new Set());
  const [browserOpen, setBrowserOpen] = useState(false);
  const [browserUrl, setBrowserUrl] = useState("");
  const [downloading, setDownloading] = useState(false);
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const progressInterval = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    const unlistenNav = listen<string>("browser-navigated", (e) => setBrowserUrl(e.payload));
    const unlistenClosed = listen("browser-closed", () => { setBrowserOpen(false); setBrowserUrl(""); });
    const unlistenComplete = listen<{ video_id: string; path: string }>("download-complete", () => {
      setDownloading(false);
      setProgress(null);
      if (progressInterval.current) clearInterval(progressInterval.current);
    });
    const unlistenError = listen<{ video_id: string; error: string }>("download-error", (event) => {
      setDownloading(false);
      setError(`Erro no download: ${event.payload.error}`);
      if (progressInterval.current) clearInterval(progressInterval.current);
    });

    return () => {
      unlistenNav.then((fn) => fn());
      unlistenClosed.then((fn) => fn());
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
    } catch (err) {
      const msg = typeof err === "string" ? err : "Erro ao analisar URL";
      setError(msg);
      if (msg.includes("Nenhum stream") || msg.includes("No HLS")) {
        try {
          await invoke("open_browser", { url });
          setBrowserOpen(true);
          setBrowserUrl(url);
          setError(null);
        } catch (_) {}
      }
    } finally {
      setLoading(false);
    }
  };

  const handleDownload = async () => {
    if (!video || selectedStreams.size === 0) return;

    const streamIdx = [...selectedStreams][0];
    const stream = video.streams[streamIdx];

    const filePath = await save({
      defaultPath: `${video.title || "video"}.${stream.init_segment_url ? "mp4" : "ts"}`,
      filters: [{ name: "Video", extensions: ["mp4", "ts", "mkv"] }],
    });

    if (!filePath) return;

    setDownloading(true);
    setError(null);

    try {
      const videoId = await invoke<string>("start_download", {
        videoId: video.id,
        stream,
        outputPath: filePath,
      });

      // Polling de progresso
      progressInterval.current = setInterval(async () => {
        try {
          const p = await invoke<DownloadProgress>("get_download_progress", { videoId });
          setProgress(p);
          if (p.state === "Done" || p.state === "Cancelled" || typeof p.state === "object") {
            setDownloading(false);
            if (progressInterval.current) clearInterval(progressInterval.current);
          }
        } catch (_) {}
      }, 500);
    } catch (err) {
      setError(typeof err === "string" ? err : "Erro ao iniciar download");
      setDownloading(false);
    }
  };

  const handleCancel = async () => {
    if (video) {
      await invoke("cancel_download", { videoId: video.id }).catch(() => {});
    }
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

  const handleOpenBrowser = async (url: string) => {
    try {
      await invoke("open_browser", { url });
      setBrowserOpen(true);
      setBrowserUrl(url);
    } catch (_) {}
  };

  const handleCloseBrowser = () => { setBrowserOpen(false); setBrowserUrl(""); };

  const showEmptyState = !loading && !video && !error && !browserOpen;

  const formatBytes = (bytes: number) => {
    if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
    if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(1)} MB`;
    if (bytes >= 1e3) return `${(bytes / 1e3).toFixed(0)} KB`;
    return `${bytes} B`;
  };

  const formatSpeed = (bps: number) => `${formatBytes(bps)}/s`;

  return (
    <div className="min-h-screen bg-zinc-950 text-zinc-100 flex flex-col">
      <header className="border-b border-zinc-800/60 bg-zinc-950/90 backdrop-blur-sm sticky top-0 z-20">
        <div className="max-w-5xl mx-auto px-6 py-4">
          <div className="flex items-center gap-3 mb-4">
            <img src="/app-icon.png" alt="ViDownload" className="w-9 h-9 rounded-lg shadow-lg shadow-violet-500/20" />
            <h1 className="text-xl font-bold text-white tracking-tight">ViDownload</h1>
            <span className="text-[10px] text-zinc-600 font-mono mt-1">v0.1.0</span>
            {!browserOpen && (
              <button
                onClick={() => {
                  const url = (document.querySelector<HTMLInputElement>("#url-input"))?.value?.trim();
                  if (url) handleOpenBrowser(url.startsWith("http") ? url : `https://${url}`);
                }}
                className="ml-auto px-3 py-1.5 text-xs bg-zinc-800 hover:bg-zinc-700 text-zinc-400 hover:text-white border border-zinc-700 rounded-lg transition-colors"
              >
                Abrir Navegador
              </button>
            )}
            {browserOpen && (
              <span className="ml-auto flex items-center gap-2 text-xs text-green-400">
                <span className="w-2 h-2 rounded-full bg-green-500 animate-pulse" />
                Navegador ativo
              </span>
            )}
          </div>
          <UrlInput onAnalyze={handleAnalyze} loading={loading} />
        </div>
      </header>

      {browserOpen && <BrowserBar currentUrl={browserUrl} onClose={handleCloseBrowser} />}

      <main className="flex-1 relative">
        {error && (
          <div className="max-w-5xl mx-auto px-6 py-6">
            <div className="p-4 bg-red-950/40 border border-red-900/50 rounded-lg text-red-400 flex items-center gap-3">
              <svg className="w-5 h-5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m9-.75a9 9 0 11-18 0 9 9 0 0118 0zm-9 3.75h.008v.008H12v-.008z" />
              </svg>
              {error}
            </div>
          </div>
        )}

        {loading && !video && (
          <div className="flex flex-col items-center justify-center py-16 gap-10">
            <div className="flex items-center gap-12">
              <CubeLoader />
              <div className="ml-8"><TerminalLoader text="Scanning..." /></div>
            </div>
            <p className="text-zinc-500 text-sm font-mono mt-4">Analisando streams na URL...</p>
          </div>
        )}

        {showEmptyState && (
          <div className="absolute inset-0 top-0">
            <PosterWall />
            <div className="absolute inset-0 bg-gradient-to-t from-zinc-950 via-zinc-950/60 to-zinc-950/30 pointer-events-none" />
            <div className="absolute inset-0 flex flex-col items-center justify-center pointer-events-none z-10">
              <img src="/app-icon.png" alt="ViDownload" className="w-24 h-24 rounded-2xl shadow-2xl shadow-violet-500/30 mb-6" />
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

        {browserOpen && !video && !loading && !error && (
          <div className="max-w-5xl mx-auto px-6 py-8 text-center text-zinc-500">
            <p className="text-lg mb-2">Navegador aberto em outra janela</p>
            <p className="text-sm">Navegue até o vídeo que deseja baixar.</p>
          </div>
        )}

        {video && (
          <div className="max-w-5xl mx-auto px-6 py-6 space-y-4">
            <StreamList
              video={video}
              selectedStreams={selectedStreams}
              onToggleStream={handleToggleStream}
              onSelectAll={handleSelectAll}
              onDownload={handleDownload}
              downloading={downloading}
            />

            {/* Barra de progresso */}
            {downloading && progress && (
              <div className="p-4 bg-zinc-900 border border-zinc-800 rounded-lg space-y-3">
                <div className="flex justify-between text-sm">
                  <span className="text-zinc-400">
                    {progress.state === "Downloading" && "Baixando..."}
                    {progress.state === "Muxing" && "Montando arquivo..."}
                    {progress.state === "Done" && "Concluído!"}
                  </span>
                  <span className="text-zinc-500">
                    {progress.segments_done}/{progress.segments_total} segs | {formatBytes(progress.bytes_downloaded)} | {formatSpeed(progress.speed_bps)}
                  </span>
                </div>

                <div className="w-full bg-zinc-800 rounded-full h-2.5">
                  <div
                    className="bg-violet-600 h-2.5 rounded-full transition-all duration-300"
                    style={{
                      width: `${progress.segments_total > 0 ? (progress.segments_done / progress.segments_total) * 100 : 0}%`,
                    }}
                  />
                </div>

                <div className="flex justify-between">
                  <span className="text-xs text-zinc-600">
                    {progress.segments_total > 0 ? Math.round((progress.segments_done / progress.segments_total) * 100) : 0}%
                  </span>
                  <button
                    onClick={handleCancel}
                    className="text-xs text-red-400 hover:text-red-300 transition-colors"
                  >
                    Cancelar
                  </button>
                </div>
              </div>
            )}
          </div>
        )}
      </main>
    </div>
  );
}

export default App;
