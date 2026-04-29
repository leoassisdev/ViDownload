# ViDownload — Plano de Desenvolvimento

> App desktop (macOS .dmg) que captura e baixa vídeos HLS/m3u8 de qualquer site.
> Stack: **Tauri 2 + React + TypeScript + Rust**

---

## Regras de Produto

1. **Entrada = URL** — O usuário cola um link de uma página com vídeo(s)
2. **Múltiplos vídeos** — Se a página tiver mais de um vídeo, o app lista todos e pergunta qual baixar (ou todos)
3. **Qualidade máxima por padrão** — Sempre seleciona a maior resolução/bitrate disponível automaticamente
4. **Opções de qualidade** — O usuário pode escolher qualidade menor antes de iniciar o download
5. **Saída = arquivo .mp4** salvo na pasta escolhida pelo usuário

---

## Arquitetura

```
┌─────────────────────────────────────────────────┐
│                   Tauri Shell                    │
│               (Rust backend + WebView)           │
├────────────┬────────────────────────┬────────────┤
│  Webview   │     React Frontend     │   Rust     │
│  Injector  │  (UI de controle)      │  Backend   │
├────────────┼────────────────────────┼────────────┤
│ Intercepta │ - Lista de streams     │ - Proxy    │
│ MediaSource│ - Qualidade/codec      │   HTTP     │
│ + network  │ - Progresso download   │ - Download │
│ requests   │ - Preview video        │   paralelo │
│            │ - Histórico            │ - M3U8     │
│            │                        │   parser   │
│            │                        │ - TS demux │
│            │                        │ - MP4 mux  │
│            │                        │ - AES-128  │
│            │                        │ - File I/O │
└────────────┴────────────────────────┴────────────┘
```

---

## Fases

### FASE 1 — Scaffold & Infra
- [x] 1.1 Criar repo git + .gitignore
- [ ] 1.2 Init Tauri 2 + React + TypeScript (pnpm)
- [ ] 1.3 Estrutura de pastas (src-tauri, src/components, src/hooks, src/lib)
- [ ] 1.4 Configurar Tauri: permissões, webview, window config
- [ ] 1.5 Build dev rodando (tauri dev)
- [ ] 1.6 Commit: "feat: scaffold Tauri 2 + React + TS"

### FASE 2 — Browser Embutido (Webview de Navegação)
- [ ] 2.1 Criar componente BrowserView com barra de URL
- [ ] 2.2 Webview secundária para navegação do usuário (onde o user abre sites)
- [ ] 2.3 Controles: voltar, avançar, reload, URL bar
- [ ] 2.4 Injetar content script na webview de navegação
- [ ] 2.5 Commit: "feat: embedded browser with navigation"

### FASE 3 — Detecção de Streams (Network Sniffing)
- [ ] 3.1 Rust: proxy HTTP interceptador (usa `hyper` ou `reqwest` + `tokio`)
- [ ] 3.2 Detectar URLs por Content-Type: `application/x-mpegurl`, `video/mp2t`, `application/dash+xml`
- [ ] 3.3 Detectar URLs por extensão: `.m3u8`, `.m3u`, `.ts`, `.m4s`
- [ ] 3.4 Capturar headers originais (Referer, Origin, Cookie, Auth) de cada request
- [ ] 3.5 Emitir evento Tauri `stream-detected` → frontend
- [ ] 3.6 Frontend: mostrar badge/notificação quando stream detectado
- [ ] 3.7 Commit: "feat: network stream detection engine"

### FASE 4 — Parser M3U8 (Rust)
- [ ] 4.1 Parser de Master Playlist (#EXT-X-STREAM-INF, BANDWIDTH, RESOLUTION, CODECS)
- [ ] 4.2 Parser de Media Playlist (#EXTINF, #EXT-X-KEY, #EXT-X-MAP, #EXT-X-BYTERANGE)
- [ ] 4.3 Resolver URLs relativas contra base URL
- [ ] 4.4 Suporte a live playlists (polling, EXT-X-MEDIA-SEQUENCE tracking)
- [ ] 4.5 Struct StreamInfo { url, quality, codec, segments[], encryption, duration }
- [ ] 4.6 Testes unitários para cada tipo de playlist
- [ ] 4.7 Commit: "feat: M3U8 parser with master + media playlist support"

### FASE 5 — Download Engine (Rust)
- [ ] 5.1 Download paralelo de segmentos (tokio tasks, configurável 1-6 threads)
- [ ] 5.2 Replay de headers originais capturados na Fase 3
- [ ] 5.3 Retry com backoff exponencial (max 6 tentativas)
- [ ] 5.4 Suporte a byte-range requests (#EXT-X-BYTERANGE)
- [ ] 5.5 Progress tracking por segmento → evento Tauri `download-progress`
- [ ] 5.6 Pause / Resume / Cancel
- [ ] 5.7 Fila de downloads
- [ ] 5.8 Commit: "feat: parallel segment download engine"

### FASE 6 — Decriptação AES-128
- [ ] 6.1 Fetch da key URL do #EXT-X-KEY
- [ ] 6.2 Decriptação AES-128-CBC com IV (do tag ou sequence number)
- [ ] 6.3 Usar crate `aes` + `cbc` (nativo Rust, sem overhead)
- [ ] 6.4 Pipeline: download → decrypt → buffer
- [ ] 6.5 Testes com streams AES-128 reais
- [ ] 6.6 Commit: "feat: AES-128-CBC segment decryption"

### FASE 7 — Demuxer TS → Raw Tracks (Rust)
- [ ] 7.1 Parser de MPEG-2 TS packets (sync 0x47, 188 bytes)
- [ ] 7.2 Parse PAT → PMT → identificar PIDs de video/audio
- [ ] 7.3 Extrair PES packets → H.264 NAL units (video) + AAC frames (audio)
- [ ] 7.4 Gerar lista de samples com timestamps (PTS/DTS)
- [ ] 7.5 Testes com .ts files reais
- [ ] 7.6 Commit: "feat: MPEG-TS demuxer"

### FASE 8 — Muxer MP4 (Rust)
- [ ] 8.1 ISO BMFF box writer (ftyp, moov, mvhd, trak, tkhd, mdia, mdhd, hdlr, minf, stbl, mdat)
- [ ] 8.2 Gerar stts, stss, stsc, stsz, stco/co64 a partir dos samples
- [ ] 8.3 Suporte a arquivos > 4GB (co64 automático)
- [ ] 8.4 Mux de tracks separados (video + audio) em MP4 único
- [ ] 8.5 Metadata: duração, codec info, timescale
- [ ] 8.6 Testes: gerar MP4 e validar com ffprobe
- [ ] 8.7 Commit: "feat: MP4 muxer (ISO BMFF writer)"

### FASE 9 — Parser fMP4 (Segmentos já em MP4)
- [ ] 9.1 Parser de init segment (ftyp + moov)
- [ ] 9.2 Parser de media segments (moof + mdat)
- [ ] 9.3 Concatenação de fMP4 segments → MP4 final
- [ ] 9.4 Parse SIDX para byte-range requests
- [ ] 9.5 Commit: "feat: fMP4 segment parser and concatenator"

### FASE 10 — Intercept MediaSource (Capture Mode)
- [ ] 10.1 Script JS injetado: monkey-patch `MediaSource.addSourceBuffer` + `SourceBuffer.appendBuffer`
- [ ] 10.2 Capturar blobs de fMP4 injetados pelo player do site
- [ ] 10.3 Enviar dados interceptados via IPC (webview → Rust)
- [ ] 10.4 Rust: receber, parsear e acumular tracks de fMP4
- [ ] 10.5 Botão "Force Capture" no frontend
- [ ] 10.6 Commit: "feat: MediaSource intercept capture mode"

### FASE 11 — Frontend UI
- [ ] 11.1 Layout: browser (topo) + painel de downloads (lateral/bottom)
- [ ] 11.2 Lista de streams detectados com: URL, qualidade, codec, duração estimada
- [ ] 11.3 Seletor de qualidade quando master playlist tem múltiplos levels
- [ ] 11.4 Barra de progresso por download (%, velocidade, ETA)
- [ ] 11.5 Botões: Download, Pause, Cancel, Force Capture
- [ ] 11.6 Preview thumbnail do vídeo
- [ ] 11.7 Histórico de downloads
- [ ] 11.8 Settings: pasta de destino, threads paralelas, tema dark/light
- [ ] 11.9 Commit: "feat: download manager UI"

### FASE 12 — Live Stream Support
- [ ] 12.1 Polling de playlist live (intervalo = targetduration)
- [ ] 12.2 Tracking de novos segmentos por media sequence
- [ ] 12.3 Gravação contínua com flush periódico
- [ ] 12.4 Botão Start/Stop recording
- [ ] 12.5 Handling de #EXT-X-DISCONTINUITY (troca de capítulo)
- [ ] 12.6 Commit: "feat: live stream recording"

### FASE 13 — Testes & QA
- [ ] 13.1 Testes unitários Rust: parser m3u8, demuxer TS, muxer MP4, AES
- [ ] 13.2 Testes de integração: pipeline completo (m3u8 → MP4)
- [ ] 13.3 Testar em sites reais: Twitch VODs, Twitter video, sites de streaming
- [ ] 13.4 Testar streams encriptados (AES-128)
- [ ] 13.5 Testar live streams
- [ ] 13.6 Testar capture mode (MediaSource intercept)
- [ ] 13.7 Validar MP4 output com ffprobe / VLC
- [ ] 13.8 Commit: "test: full test suite"

### FASE 14 — Build & Distribuição (.dmg)
- [ ] 14.1 Configurar Tauri bundler para macOS (.dmg + .app)
- [ ] 14.2 Ícone do app (icon.icns)
- [ ] 14.3 Info.plist: nome, versão, bundle identifier
- [ ] 14.4 Code signing (se tiver Apple Developer ID)
- [ ] 14.5 Notarização Apple (se tiver conta)
- [ ] 14.6 Testar instalação .dmg em Mac limpo
- [ ] 14.7 Commit: "build: macOS .dmg distribution"

---

## Stack Técnica

| Camada | Tecnologia | Motivo |
|--------|-----------|--------|
| Shell | **Tauri 2** | Nativo, leve (~5MB), Rust backend, webview nativo |
| Frontend | **React 19 + TypeScript** | UI reativa, componentes |
| Styling | **Tailwind CSS 4** | Rápido, dark mode built-in |
| Backend | **Rust** | Performance para demux/mux/crypto, zero overhead |
| HTTP | **reqwest + tokio** | Async HTTP client, download paralelo |
| M3U8 | **Custom parser (Rust)** | Controle total, zero dependência externa |
| TS Demux | **Custom (Rust)** | MPEG-TS → raw H.264/AAC |
| MP4 Mux | **Custom (Rust)** | ISO BMFF writer, controle total dos boxes |
| Crypto | **aes + cbc crates** | AES-128-CBC nativo, rápido |
| IPC | **Tauri Commands + Events** | Comunicação webview ↔ Rust |
| Build | **Tauri bundler** | Gera .dmg nativo |

## Crates Rust Principais

```toml
[dependencies]
tauri = { version = "2", features = ["all"] }
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["cookies", "stream"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
aes = "0.8"
cbc = "0.1"
url = "2"
byteorder = "1"
bytes = "1"
thiserror = "2"
tracing = "0.1"
```

## Vantagens sobre a Extensão Chrome

| Extensão Chrome | ViDownload |
|----------------|------------|
| Limitado ao browser | App standalone |
| JS puro (lento para mux) | Rust (10-50x mais rápido) |
| Limites de memória do browser | Memória nativa sem limites |
| Sem suporte a arquivos grandes | Stream direto para disco |
| Precisa do site hlsloader.com | Tudo local, zero dependência externa |
| Não gera .dmg | Distribuição nativa macOS |
