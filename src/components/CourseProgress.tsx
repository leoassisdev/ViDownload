import type { CourseTree } from "../types";

interface Props {
  tree: CourseTree;
  statuses: Map<string, string>; // key -> status ("downloading"|"done"|"skipped"|"error")
  done: number;
  total: number;
  finished: boolean;
  onCancel: () => void;
  onClose: () => void;
}

const ICON: Record<string, string> = {
  downloading: "⬇️",
  done: "✅",
  skipped: "⏭️",
  error: "❌",
  queued: "⏳",
};

export default function CourseProgress({ tree, statuses, done, total, finished, onCancel, onClose }: Props) {
  const pct = total > 0 ? Math.round((done / total) * 100) : 0;
  const failed = [...statuses.values()].filter((s) => s === "error").length;

  return (
    <div className="max-w-5xl mx-auto px-6 py-4">
      <div className="border border-zinc-800 rounded-2xl bg-zinc-900/60 overflow-hidden">
        <div className="p-4 border-b border-zinc-800 flex items-center gap-3">
          <div className="flex-1">
            <div className="flex items-center gap-2">
              <h3 className="text-sm font-bold text-white">{tree.course_name}</h3>
              {!finished && <span className="w-2 h-2 rounded-full bg-blue-500 animate-pulse" />}
            </div>
            <p className="text-xs text-zinc-500 mt-0.5">
              {done}/{total} concluídas{failed > 0 ? ` · ${failed} com erro` : ""}
              {finished ? " · finalizado" : ""}
            </p>
          </div>
          {!finished ? (
            <button
              onClick={onCancel}
              className="px-3 py-1.5 rounded-lg text-xs font-medium bg-red-950/50 border border-red-900/50 text-red-400 hover:bg-red-950"
            >
              Cancelar
            </button>
          ) : (
            <button
              onClick={onClose}
              className="px-3 py-1.5 rounded-lg text-xs font-medium bg-zinc-800 border border-zinc-700 text-zinc-300 hover:bg-zinc-700"
            >
              Fechar
            </button>
          )}
        </div>

        {/* Barra agregada */}
        <div className="px-4 pt-3">
          <div className="h-2 rounded-full bg-zinc-800 overflow-hidden">
            <div
              className="h-full bg-gradient-to-r from-blue-600 to-red-600 transition-all duration-300"
              style={{ width: `${pct}%` }}
            />
          </div>
          <p className="text-right text-xs text-zinc-500 mt-1">{pct}%</p>
        </div>

        {/* Lista por aula */}
        <div className="max-h-72 overflow-auto p-3 space-y-1">
          {tree.lessons.map((l) => {
            const st = statuses.get(l.key) || "queued";
            return (
              <div key={l.key} className="flex items-center gap-2 text-sm px-2 py-1 rounded hover:bg-zinc-800/40">
                <span className="w-5 text-center">{ICON[st] || "⏳"}</span>
                <span
                  className={`flex-1 truncate ${
                    st === "error" ? "text-red-400" : st === "done" || st === "skipped" ? "text-zinc-400" : "text-zinc-200"
                  }`}
                >
                  {String(l.module_index).padStart(2, "0")}.{String(l.lesson_index).padStart(2, "0")} {l.title}
                </span>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
