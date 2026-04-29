import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface Props {
  currentUrl: string;
  onClose: () => void;
}

export default function BrowserBar({ currentUrl, onClose }: Props) {
  const [urlInput, setUrlInput] = useState(currentUrl);
  const [navigating, setNavigating] = useState(false);

  const handleNavigate = async (e: React.FormEvent) => {
    e.preventDefault();
    let url = urlInput.trim();
    if (!url) return;

    // Adicionar protocolo se não tiver
    if (!url.startsWith("http://") && !url.startsWith("https://")) {
      url = "https://" + url;
      setUrlInput(url);
    }

    setNavigating(true);
    try {
      await invoke("browser_navigate", { url });
    } catch (err) {
      console.error("Erro ao navegar:", err);
    } finally {
      setNavigating(false);
    }
  };

  const handleBack = () => invoke("browser_back").catch(console.error);
  const handleForward = () => invoke("browser_forward").catch(console.error);
  const handleReload = () => invoke("browser_reload").catch(console.error);

  const handleClose = async () => {
    await invoke("close_browser").catch(console.error);
    onClose();
  };

  return (
    <div className="flex items-center gap-2 px-4 py-2 bg-zinc-900 border-b border-zinc-800/60">
      {/* Controles de navegação */}
      <div className="flex items-center gap-1">
        <button
          onClick={handleBack}
          className="p-1.5 rounded hover:bg-zinc-800 text-zinc-400 hover:text-white transition-colors"
          title="Voltar"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M15 19l-7-7 7-7" />
          </svg>
        </button>

        <button
          onClick={handleForward}
          className="p-1.5 rounded hover:bg-zinc-800 text-zinc-400 hover:text-white transition-colors"
          title="Avançar"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M9 5l7 7-7 7" />
          </svg>
        </button>

        <button
          onClick={handleReload}
          className="p-1.5 rounded hover:bg-zinc-800 text-zinc-400 hover:text-white transition-colors"
          title="Recarregar"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
          </svg>
        </button>
      </div>

      {/* Barra de URL */}
      <form onSubmit={handleNavigate} className="flex-1 flex">
        <input
          type="text"
          value={urlInput}
          onChange={(e) => setUrlInput(e.target.value)}
          className="flex-1 px-3 py-1.5 bg-zinc-800 border border-zinc-700 rounded-lg text-sm text-white placeholder-zinc-500 focus:outline-none focus:border-violet-500 transition-colors font-mono"
          placeholder="Digite uma URL..."
          disabled={navigating}
        />
      </form>

      {/* Indicador de status */}
      <div className="flex items-center gap-2">
        <span className="w-2 h-2 rounded-full bg-green-500 animate-pulse" title="Navegador ativo" />

        {/* Botão fechar browser */}
        <button
          onClick={handleClose}
          className="p-1.5 rounded hover:bg-red-900/50 text-zinc-400 hover:text-red-400 transition-colors"
          title="Fechar navegador"
        >
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>
    </div>
  );
}
