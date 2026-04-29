import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import UrlInput from "./components/UrlInput";
import StreamList from "./components/StreamList";
import TerminalLoader from "./components/TerminalLoader";
import CubeLoader from "./components/CubeLoader";
import PosterWall from "./components/PosterWall";
import type { VideoFound } from "./types";

function App() {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [video, setVideo] = useState<VideoFound | null>(null);
  const [selectedStreams, setSelectedStreams] = useState<Set<number>>(new Set());

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
      setError(typeof err === "string" ? err : "Erro ao analisar URL");
    } finally {
      setLoading(false);
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

  const handleDownload = async () => {
    if (!video) return;
    console.log("Download streams:", [...selectedStreams]);
  };

  const showEmptyState = !loading && !video && !error;

  return (
    <div className="min-h-screen bg-zinc-950 text-zinc-100 flex flex-col">
      {/* Header */}
      <header className="border-b border-zinc-800/60 bg-zinc-950/90 backdrop-blur-sm sticky top-0 z-20">
        <div className="max-w-5xl mx-auto px-6 py-4">
          <div className="flex items-center gap-3 mb-4">
            <img
              src="/app-icon.png"
              alt="ViDownload"
              className="w-9 h-9 rounded-lg shadow-lg shadow-violet-500/20"
            />
            <h1 className="text-xl font-bold text-white tracking-tight">ViDownload</h1>
            <span className="text-[10px] text-zinc-600 font-mono mt-1">v0.1.0</span>
          </div>
          <UrlInput onAnalyze={handleAnalyze} loading={loading} />
        </div>
      </header>

      {/* Content */}
      <main className="flex-1 relative">
        {/* Error */}
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

        {/* Loading — Terminal + Cube */}
        {loading && !video && (
          <div className="flex flex-col items-center justify-center py-16 gap-10">
            <div className="flex items-center gap-12">
              <CubeLoader />
              <div className="ml-8">
                <TerminalLoader text="Scanning..." />
              </div>
            </div>
            <p className="text-zinc-500 text-sm font-mono mt-4">Analisando streams na URL...</p>
          </div>
        )}

        {/* Empty state — 3D Poster Wall + Branding overlay */}
        {showEmptyState && (
          <div className="absolute inset-0 top-0">
            <PosterWall />

            {/* Gradient overlay */}
            <div className="absolute inset-0 bg-gradient-to-t from-zinc-950 via-zinc-950/60 to-zinc-950/30 pointer-events-none" />

            {/* Branding center */}
            <div className="absolute inset-0 flex flex-col items-center justify-center pointer-events-none z-10">
              <img
                src="/app-icon.png"
                alt="ViDownload"
                className="w-24 h-24 rounded-2xl shadow-2xl shadow-violet-500/30 mb-6"
              />
              <h2 className="text-4xl font-bold text-white tracking-tight mb-2 drop-shadow-lg">
                ViDownload
              </h2>
              <p className="text-zinc-400 text-lg drop-shadow-md">
                Cole um link para baixar qualquer vídeo
              </p>
              <div className="flex gap-3 mt-4">
                <span className="px-3 py-1 text-xs bg-zinc-800/80 text-zinc-400 rounded-full border border-zinc-700/50">
                  HLS
                </span>
                <span className="px-3 py-1 text-xs bg-zinc-800/80 text-zinc-400 rounded-full border border-zinc-700/50">
                  m3u8
                </span>
                <span className="px-3 py-1 text-xs bg-zinc-800/80 text-zinc-400 rounded-full border border-zinc-700/50">
                  MP4
                </span>
                <span className="px-3 py-1 text-xs bg-zinc-800/80 text-zinc-400 rounded-full border border-zinc-700/50">
                  AES-128
                </span>
              </div>
            </div>
          </div>
        )}

        {/* Results */}
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
      </main>
    </div>
  );
}

export default App;
