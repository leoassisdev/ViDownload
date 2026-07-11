import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { SiteCredential } from "../types";

interface Props {
  open: boolean;
  onClose: () => void;
  ffmpegOk: boolean;
}

export default function SettingsModal({ open, onClose, ffmpegOk }: Props) {
  const [creds, setCreds] = useState<SiteCredential[]>([]);
  const [host, setHost] = useState("potenza.memberkit.com.br");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const load = async () => {
    try {
      setCreds(await invoke<SiteCredential[]>("list_site_credentials"));
    } catch {
      setCreds([]);
    }
  };

  useEffect(() => {
    if (open) load();
  }, [open]);

  const save = async () => {
    if (!host.trim() || !email.trim() || !password) {
      setMsg("Preencha site, email e senha.");
      return;
    }
    setSaving(true);
    setMsg(null);
    try {
      await invoke("save_site_credentials", { host: host.trim(), email: email.trim(), password });
      setPassword("");
      setMsg("Salvo!");
      await load();
    } catch (e) {
      setMsg(typeof e === "string" ? e : "Erro ao salvar");
    } finally {
      setSaving(false);
    }
  };

  const remove = async (h: string) => {
    try {
      await invoke("remove_site_credentials", { host: h });
      await load();
    } catch {}
  };

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm p-4" onClick={onClose}>
      <div
        className="w-full max-w-lg bg-zinc-900 border border-zinc-800 rounded-2xl shadow-2xl p-6 space-y-5"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between">
          <h2 className="text-lg font-bold text-white">Configurações</h2>
          <button onClick={onClose} className="text-zinc-500 hover:text-zinc-300">
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* ffmpeg status */}
        <div className={`flex items-center gap-2 text-sm px-3 py-2 rounded-lg border ${
          ffmpegOk
            ? "bg-green-950/30 border-green-900/50 text-green-400"
            : "bg-amber-950/30 border-amber-900/50 text-amber-400"
        }`}>
          <span className={`w-2 h-2 rounded-full ${ffmpegOk ? "bg-green-500" : "bg-amber-500"}`} />
          {ffmpegOk
            ? "ffmpeg detectado — downloads com áudio+vídeo (Vimeo) funcionam."
            : "ffmpeg não encontrado. Instale com: brew install ffmpeg"}
        </div>

        {/* Credenciais de sites */}
        <div className="space-y-3">
          <div>
            <h3 className="text-sm font-semibold text-zinc-300">Login de áreas de membros</h3>
            <p className="text-xs text-zinc-500">
              Para sites com login (ex. MemberKit), o app entra automaticamente com estas credenciais.
              Ficam salvas só neste computador.
            </p>
          </div>

          {creds.length > 0 && (
            <div className="space-y-2">
              {creds.map((c) => (
                <div key={c.host} className="flex items-center gap-3 text-sm bg-zinc-800/60 rounded-lg px-3 py-2">
                  <div className="flex-1 min-w-0">
                    <div className="text-white truncate">{c.host}</div>
                    <div className="text-zinc-500 text-xs truncate">{c.email}</div>
                  </div>
                  <button
                    onClick={() => remove(c.host)}
                    className="text-xs text-zinc-500 hover:text-red-400"
                  >
                    Remover
                  </button>
                </div>
              ))}
            </div>
          )}

          <div className="space-y-2">
            <input
              value={host}
              onChange={(e) => setHost(e.target.value)}
              placeholder="site (ex. potenza.memberkit.com.br)"
              className="w-full px-3 py-2 bg-zinc-800 border border-zinc-700 rounded-lg text-sm text-white placeholder-zinc-600 focus:border-blue-500 outline-none"
            />
            <input
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="email de login"
              autoComplete="off"
              className="w-full px-3 py-2 bg-zinc-800 border border-zinc-700 rounded-lg text-sm text-white placeholder-zinc-600 focus:border-blue-500 outline-none"
            />
            <input
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              type="password"
              placeholder="senha"
              autoComplete="new-password"
              className="w-full px-3 py-2 bg-zinc-800 border border-zinc-700 rounded-lg text-sm text-white placeholder-zinc-600 focus:border-blue-500 outline-none"
            />
            <div className="flex items-center gap-3">
              <button
                onClick={save}
                disabled={saving}
                className="px-4 py-2 bg-blue-600 hover:bg-blue-500 disabled:bg-zinc-700 text-white text-sm font-semibold rounded-lg transition-colors"
              >
                {saving ? "Salvando..." : "Salvar login"}
              </button>
              {msg && <span className="text-xs text-zinc-400">{msg}</span>}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
