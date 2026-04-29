import type { VideoFound } from "../types";

interface Props {
  video: VideoFound;
  selectedStreams: Set<number>;
  onToggleStream: (index: number) => void;
  onSelectAll: () => void;
  onDownload: () => void;
}

export default function StreamList({
  video,
  selectedStreams,
  onToggleStream,
  onSelectAll,
  onDownload,
}: Props) {
  const formatDuration = (seconds: number): string => {
    if (seconds <= 0) return "--:--";
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = Math.floor(seconds % 60);
    if (h > 0) return `${h}:${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
    return `${m}:${s.toString().padStart(2, "0")}`;
  };

  const formatBandwidth = (bps: number): string => {
    if (bps >= 1_000_000) return `${(bps / 1_000_000).toFixed(1)} Mbps`;
    if (bps >= 1_000) return `${Math.round(bps / 1_000)} kbps`;
    return `${bps} bps`;
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-lg font-semibold text-white">
            {video.title || "Vídeo encontrado"}
          </h2>
          <p className="text-sm text-zinc-400 truncate max-w-xl">{video.page_url}</p>
        </div>
        <div className="flex gap-2">
          {video.streams.length > 1 && (
            <button
              onClick={onSelectAll}
              className="px-4 py-2 text-sm bg-zinc-700 hover:bg-zinc-600 text-zinc-300 rounded-lg transition-colors"
            >
              {selectedStreams.size === video.streams.length ? "Desmarcar todos" : "Selecionar todos"}
            </button>
          )}
          <button
            onClick={onDownload}
            disabled={selectedStreams.size === 0}
            className="px-5 py-2 bg-green-600 hover:bg-green-500 disabled:bg-zinc-700 disabled:text-zinc-500 text-white font-semibold rounded-lg transition-colors"
          >
            Baixar {selectedStreams.size > 1 ? `(${selectedStreams.size})` : ""}
          </button>
        </div>
      </div>

      <div className="space-y-2">
        {video.streams.map((stream, idx) => {
          const isBest = idx === video.best_quality_index;
          const isSelected = selectedStreams.has(idx);

          return (
            <div
              key={idx}
              onClick={() => onToggleStream(idx)}
              className={`flex items-center gap-4 p-4 rounded-lg border cursor-pointer transition-all ${
                isSelected
                  ? "bg-violet-900/30 border-violet-500"
                  : "bg-zinc-800/50 border-zinc-700 hover:border-zinc-600"
              }`}
            >
              <div className={`w-5 h-5 rounded border-2 flex items-center justify-center transition-colors ${
                isSelected ? "bg-violet-600 border-violet-600" : "border-zinc-600"
              }`}>
                {isSelected && (
                  <svg className="w-3 h-3 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
                  </svg>
                )}
              </div>

              <div className="flex-1 flex items-center gap-4">
                <span className="text-white font-mono font-bold text-lg min-w-[80px]">
                  {stream.quality}
                </span>

                {isBest && (
                  <span className="px-2 py-0.5 text-xs font-bold bg-amber-500/20 text-amber-400 rounded">
                    MELHOR
                  </span>
                )}

                {stream.is_encrypted && (
                  <span className="px-2 py-0.5 text-xs bg-red-500/20 text-red-400 rounded">
                    AES-128
                  </span>
                )}

                <span className="text-sm text-zinc-400">
                  {stream.codecs || "—"}
                </span>
              </div>

              <div className="flex items-center gap-6 text-sm text-zinc-400">
                <span>{formatDuration(stream.total_duration)}</span>
                {stream.bandwidth > 0 && <span>{formatBandwidth(stream.bandwidth)}</span>}
                <span>{stream.segments.length} segs</span>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
