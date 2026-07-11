export interface StreamInfo {
  url: string;
  quality: string;
  bandwidth: number;
  resolution: string | null;
  codecs: string | null;
  segments: Segment[];
  is_encrypted: boolean;
  is_live: boolean;
  init_segment_url: string | null;
  total_duration: number;
  target_duration: number;
  media_sequence: number;
  /** Playlist de áudio separada (HLS demuxado, ex. Vimeo); baixa via ffmpeg. */
  audio_url?: string | null;
  /** Referer para as requisições de mídia, quando o CDN exige. */
  download_referer?: string | null;
}

export interface SiteCredential {
  host: string;
  email: string;
  password: string;
}

export interface Segment {
  url: string;
  duration: number;
  sequence: number;
  byte_range: ByteRange | null;
  key: EncryptionKey | null;
}

export interface ByteRange {
  length: number;
  offset: number;
}

export interface EncryptionKey {
  method: string;
  uri: string;
  iv: string | null;
}

export interface VideoFound {
  id: string;
  page_url: string;
  streams: StreamInfo[];
  title: string | null;
  best_quality_index: number;
}

export interface DownloadProgress {
  video_id: string;
  state: DownloadState;
  segments_done: number;
  segments_total: number;
  bytes_downloaded: number;
  speed_bps: number;
  eta_seconds: number;
}

export type DownloadState =
  | "Analyzing"
  | "Downloading"
  | "Muxing"
  | "Done"
  | "Cancelled"
  | { Error: string };
