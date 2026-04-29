import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import UrlInput from "./components/UrlInput";
import StreamList from "./components/StreamList";
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
      // Auto-select best quality
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
      if (next.has(index)) {
        next.delete(index);
      } else {
        next.add(index);
      }
      return next;
    });
  };

  const handleSelectAll = () => {
    if (!video) return;
    if (selectedStreams.size === video.streams.length) {
      setSelectedStreams(new Set());
    } else {
      setSelectedStreams(new Set(video.streams.map((_, i) => i)));
    }
  };

  const handleDownload = async () => {
    if (!video) return;
    // TODO: Phase 5 — open save dialog, start downloads
    console.log("Download streams:", [...selectedStreams]);
  };

  return (
    <div className="min-h-screen bg-zinc-900 text-zinc-100">
      {/* Header */}
      <header className="border-b border-zinc-800 bg-zinc-900/80 backdrop-blur-sm sticky top-0 z-10">
        <div className="max-w-5xl mx-auto px-6 py-4">
          <div className="flex items-center gap-3 mb-4">
            <div className="w-8 h-8 rounded-lg bg-violet-600 flex items-center justify-center">
              <svg className="w-5 h-5 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
              </svg>
            </div>
            <h1 className="text-xl font-bold text-white">ViDownload</h1>
            <span className="text-xs text-zinc-500 mt-1">v0.1.0</span>
          </div>
          <UrlInput onAnalyze={handleAnalyze} loading={loading} />
        </div>
      </header>

      {/* Content */}
      <main className="max-w-5xl mx-auto px-6 py-6">
        {error && (
          <div className="p-4 bg-red-900/30 border border-red-800 rounded-lg text-red-300">
            {error}
          </div>
        )}

        {loading && !video && (
          <div className="flex flex-col items-center justify-center py-20 text-zinc-500">
            <svg className="animate-spin h-10 w-10 mb-4" viewBox="0 0 24 24">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
            </svg>
            <p>Analisando URL...</p>
          </div>
        )}

        {!loading && !video && !error && (
          <div className="flex flex-col items-center justify-center py-20 text-zinc-600">
            <svg className="w-16 h-16 mb-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M13.19 8.688a4.5 4.5 0 011.242 7.244l-4.5 4.5a4.5 4.5 0 01-6.364-6.364l1.757-1.757m9.86-2.04a4.5 4.5 0 00-6.364-6.364L6.188 6.07a4.5 4.5 0 001.242 7.244" />
            </svg>
            <p className="text-lg">Cole um link para começar</p>
            <p className="text-sm mt-1">Suporta HLS, m3u8 e páginas com vídeo</p>
          </div>
        )}

        {video && (
          <StreamList
            video={video}
            selectedStreams={selectedStreams}
            onToggleStream={handleToggleStream}
            onSelectAll={handleSelectAll}
            onDownload={handleDownload}
          />
        )}
      </main>
    </div>
  );
}

export default App;
