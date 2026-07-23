import { useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type { CourseTree, CourseLessonUI } from "../types";

interface Props {
  open: boolean;
  tree: CourseTree;
  onClose: () => void;
  onDownload: (lessons: CourseLessonUI[], outputDir: string, includePdfs: boolean) => void;
}

export default function CourseModal({ open: isOpen, tree, onClose, onDownload }: Props) {
  const [selected, setSelected] = useState<Set<string>>(
    () => new Set(tree.lessons.map((l) => l.key)) // tudo marcado por padrão
  );
  const [includePdfs, setIncludePdfs] = useState(true);
  const [folderName, setFolderName] = useState(tree.course_name || "Curso");
  const [parentDir, setParentDir] = useState<string | null>(null);

  // Agrupar por módulo mantendo a ordem
  const modules = useMemo(() => {
    const map = new Map<number, { name: string; lessons: CourseLessonUI[] }>();
    for (const l of tree.lessons) {
      if (!map.has(l.module_index)) map.set(l.module_index, { name: l.module_name, lessons: [] });
      map.get(l.module_index)!.lessons.push(l);
    }
    return [...map.entries()].sort((a, b) => a[0] - b[0]);
  }, [tree]);

  const withVideo = tree.lessons.filter((l) => l.has_video).length;
  const withPdf = tree.lessons.filter((l) => l.attachments.length > 0).length;

  const toggle = (key: string) =>
    setSelected((prev) => {
      const n = new Set(prev);
      n.has(key) ? n.delete(key) : n.add(key);
      return n;
    });

  const toggleModule = (lessons: CourseLessonUI[]) =>
    setSelected((prev) => {
      const n = new Set(prev);
      const allOn = lessons.every((l) => n.has(l.key));
      lessons.forEach((l) => (allOn ? n.delete(l.key) : n.add(l.key)));
      return n;
    });

  const pickFolder = async () => {
    const dir = await open({ directory: true, multiple: false, title: "Escolha onde salvar o curso" });
    if (typeof dir === "string") setParentDir(dir);
  };

  const finalPath = parentDir ? `${parentDir}/${(folderName || "Curso").replace(/[\/\\:*?"<>|]/g, "-").trim()}` : null;

  const start = (all: boolean) => {
    if (!finalPath) {
      pickFolder();
      return;
    }
    const chosen = all ? tree.lessons : tree.lessons.filter((l) => selected.has(l.key));
    if (chosen.length === 0) return;
    onDownload(chosen, finalPath, includePdfs);
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-4">
      <div className="w-full max-w-2xl max-h-[88vh] bg-zinc-900 border border-zinc-800 rounded-2xl flex flex-col overflow-hidden">
        {/* Cabeçalho */}
        <div className="p-5 border-b border-zinc-800 flex items-start justify-between">
          <div>
            <h2 className="text-lg font-bold text-white">Baixar curso</h2>
            <p className="text-sm text-zinc-400 mt-0.5 truncate max-w-md">{tree.course_name}</p>
            <p className="text-xs text-zinc-500 mt-1">
              {tree.lessons.length} aulas · {withVideo} com vídeo · {withPdf} com material
            </p>
          </div>
          <button onClick={onClose} className="p-1 text-zinc-500 hover:text-zinc-200" title="Fechar">
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Destino */}
        <div className="p-4 border-b border-zinc-800 space-y-2 bg-zinc-900/60">
          <div className="flex gap-2">
            <input
              value={folderName}
              onChange={(e) => setFolderName(e.target.value)}
              placeholder="Nome da pasta do curso"
              className="flex-1 px-3 py-2 bg-zinc-800 border border-zinc-700 rounded-lg text-sm text-white outline-none focus:border-blue-500"
            />
            <button
              onClick={pickFolder}
              className="px-3 py-2 bg-zinc-800 border border-zinc-700 rounded-lg text-sm text-zinc-200 hover:bg-zinc-700"
            >
              Escolher pasta
            </button>
          </div>
          <p className="text-xs text-zinc-500 truncate">
            {finalPath ? `Salvar em: ${finalPath}` : "Nenhuma pasta escolhida ainda"}
          </p>
          <label className="flex items-center gap-2 text-sm text-zinc-300 cursor-pointer select-none">
            <input type="checkbox" checked={includePdfs} onChange={(e) => setIncludePdfs(e.target.checked)} />
            Incluir PDFs / materiais das aulas
          </label>
        </div>

        {/* Árvore módulos → aulas */}
        <div className="flex-1 overflow-auto p-4 space-y-3">
          {modules.map(([mi, mod]) => {
            const allOn = mod.lessons.every((l) => selected.has(l.key));
            return (
              <div key={mi} className="border border-zinc-800 rounded-lg overflow-hidden">
                <button
                  onClick={() => toggleModule(mod.lessons)}
                  className="w-full flex items-center gap-2 px-3 py-2 bg-zinc-800/60 hover:bg-zinc-800 text-left"
                >
                  <span
                    className={`w-4 h-4 rounded border flex items-center justify-center ${
                      allOn ? "bg-blue-600 border-blue-600" : "border-zinc-600"
                    }`}
                  >
                    {allOn && <span className="text-white text-[10px]">✓</span>}
                  </span>
                  <span className="text-sm font-semibold text-white flex-1 truncate">
                    {String(mi).padStart(2, "0")}. {mod.name}
                  </span>
                  <span className="text-xs text-zinc-500">{mod.lessons.length}</span>
                </button>
                <div className="divide-y divide-zinc-800/60">
                  {mod.lessons.map((l) => {
                    const on = selected.has(l.key);
                    return (
                      <label
                        key={l.key}
                        className="flex items-center gap-2 px-3 py-2 pl-8 cursor-pointer hover:bg-zinc-800/40"
                      >
                        <span
                          className={`w-4 h-4 rounded border flex items-center justify-center shrink-0 ${
                            on ? "bg-blue-600 border-blue-600" : "border-zinc-600"
                          }`}
                        >
                          {on && <span className="text-white text-[10px]">✓</span>}
                        </span>
                        <input type="checkbox" className="hidden" checked={on} onChange={() => toggle(l.key)} />
                        <span className="text-sm text-zinc-300 flex-1 truncate">
                          {String(l.lesson_index).padStart(2, "0")}. {l.title}
                        </span>
                        {!l.has_video && <span className="text-[10px] text-zinc-600">sem vídeo</span>}
                        {l.attachments.length > 0 && <span title="tem material">📄</span>}
                      </label>
                    );
                  })}
                </div>
              </div>
            );
          })}
        </div>

        {/* Ações */}
        <div className="p-4 border-t border-zinc-800 flex items-center gap-2">
          <span className="text-xs text-zinc-500 flex-1">{selected.size} selecionadas</span>
          <button
            onClick={() => start(false)}
            disabled={selected.size === 0}
            className="px-4 py-2 rounded-lg text-sm font-medium bg-zinc-800 border border-zinc-700 text-white hover:bg-zinc-700 disabled:opacity-40"
          >
            Baixar selecionadas ({selected.size})
          </button>
          <button
            onClick={() => start(true)}
            className="px-4 py-2 rounded-lg text-sm font-semibold bg-gradient-to-r from-blue-600 to-red-600 text-white hover:opacity-90"
          >
            Baixar curso inteiro
          </button>
        </div>
      </div>
    </div>
  );
}
