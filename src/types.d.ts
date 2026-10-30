import type defaults from '../src-tauri/resources/defaults.json';

// The persisted defaults own the configuration keys and value types.
export type Config = typeof defaults;
export type Page = 'Dictation' | 'History' | 'Models' | 'Learning' | 'Settings';

export interface Model {
  id: string;
  name: string;
  languages: string;
  installed: boolean;
  runtime: string;
  notes?: string | null;
  vram_gb?: number;
  download_gb?: number;
  gguf?: { size: number } | null;
  gated?: boolean;
  punctuates?: boolean;
}

export interface Download {
  id: string;
  stage: string;
  bytes?: number;
  total?: number;
  error?: string;
}

export interface Snapshot {
  config: Config;
  data_dir: string;
  version: string;
  manual_recording: boolean;
  models: Model[];
  diagnostics: { guidance: string };
  state: {
    stage: string;
    device: string;
    level: number;
    recent: string;
    error: string | null;
    download: Download | null;
    actual_input?: string;
  };
}

export interface Recording {
  id: number;
  ts: number;
  pasted: string;
  corrected: string | null;
  audio: string | null;
  excluded: number;
  teacher_text?: string | null;
  teacher_confidence?: number | null;
  teacher_note?: string | null;
}

export interface Learning {
  teacher: unknown;
  training: unknown;
  rules: { wrong: string; right: string; count: number }[];
  vocab: { term: string; count: number }[];
  versions: { name: string; gate1: unknown; gate2: unknown }[];
}
