import type { DownloadProgress as DP } from "../types";

interface Props {
  progress: DP;
  onCancel: () => void;
  onPause?: () => void;
  onResume?: () => void;
}

export default function DownloadProgressBar({ progress, onCancel }: Props) {
  const pct = progress.segments_total > 0
    ? (progress.segments_done / progress.segments_total) * 100
    : 0;

  const strokeLen = 310;
  const strokeVal = (pct / 100) * strokeLen;
  const dashVal = strokeLen - strokeVal;

  const isDone = progress.state === "Done";
  const isMuxing = progress.state === "Muxing";

  const formatBytes = (bytes: number) => {
    if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
    if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(1)} MB`;
    if (bytes >= 1e3) return `${(bytes / 1e3).toFixed(0)} KB`;
    return `${bytes} B`;
  };

  return (
    <div className="flex items-center gap-6 p-5 bg-zinc-900/80 border border-zinc-800 rounded-xl backdrop-blur-sm">
      {/* SVG Circular Progress */}
      <div className="relative w-24 h-24 shrink-0">
        <svg viewBox="0 0 120 120" className="w-24 h-24 -rotate-90">
          {/* Track */}
          <rect
            x="10" y="10" width="100" height="100" rx="16"
            fill="none"
            stroke="rgba(255,255,255,0.05)"
            strokeWidth="6"
          />
          {/* Progress */}
          <rect
            x="10" y="10" width="100" height="100" rx="16"
            fill="none"
            stroke="url(#progressGradient)"
            strokeWidth="6"
            strokeDasharray={`${strokeVal} ${dashVal}`}
            strokeLinecap="round"
            style={{ transition: "stroke-dasharray 0.3s ease" }}
            opacity={pct > 0 ? 1 : 0}
          />
          <defs>
            <linearGradient id="progressGradient" x1="0%" y1="0%" x2="100%" y2="100%">
              <stop offset="0%" stopColor="#3b82f6" />
              <stop offset="100%" stopColor="#ef4444" />
            </linearGradient>
          </defs>
        </svg>
        {/* Center text */}
        <div className="absolute inset-0 flex items-center justify-center">
          {isDone ? (
            <svg className="w-8 h-8 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={3}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
            </svg>
          ) : (
            <span className="text-lg font-bold bg-gradient-to-r from-blue-400 to-red-400 bg-clip-text text-transparent">
              {Math.round(pct)}%
            </span>
          )}
        </div>
      </div>

      {/* Info */}
      <div className="flex-1 min-w-0 space-y-2">
        <div className="flex items-center justify-between">
          <span className="text-sm font-semibold text-white">
            {isDone && "Download concluído!"}
            {isMuxing && "Montando arquivo..."}
            {progress.state === "Downloading" && "Baixando..."}
            {progress.state === "Analyzing" && "Analisando..."}
          </span>
          <span className="text-xs text-zinc-500 font-mono">
            {progress.segments_done}/{progress.segments_total}
          </span>
        </div>

        {/* Linear progress bar */}
        <div className="w-full bg-zinc-800 rounded-full h-1.5 overflow-hidden">
          <div
            className="h-full rounded-full bg-gradient-to-r from-blue-500 to-red-500 transition-all duration-300"
            style={{ width: `${pct}%` }}
          />
        </div>

        <div className="flex items-center justify-between text-xs text-zinc-500">
          <span>{formatBytes(progress.bytes_downloaded)}</span>
          <span>{formatBytes(progress.speed_bps)}/s</span>
        </div>
      </div>

      {/* Controls */}
      <div className="flex flex-col gap-2 shrink-0">
        {!isDone && (
          <button
            onClick={onCancel}
            className="p-2 rounded-lg hover:bg-red-900/30 text-zinc-500 hover:text-red-400 transition-colors"
            title="Cancelar"
          >
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        )}
      </div>
    </div>
  );
}
