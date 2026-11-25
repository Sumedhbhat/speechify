// The contract between the UI and the Rust shell. Every type here mirrors a
// serde type in `core/` or `src-tauri/`; every function is one Tauri command.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Style = "formal" | "casual" | "very_casual" | "excited";
export type Category =
  "personal" | "work" | "email" | "code" | "terminal" | "assistant" | "document" | "other";
export type Outcome = "pasted" | "copied" | "cancelled" | "failed";

export interface Settings {
  hotkeys: {
    push_to_talk: string;
    hands_free: string;
    command: string;
    paste_last: string;
    copy_last: string;
    cancel: string;
  };
  styles: { personal: Style; work: Style; email: Style; other: Style };
  speech: {
    model: string;
    device: string; // "auto" | "cpu" | "vulkan" | "metal" | "cuda"
    language: string;
    remote_url: string;
    remote_key: string;
    remote_model: string;
    streaming: boolean; // dictate with streaming_model instead of model
    streaming_model: string;
  };
  rewrite: {
    level: "none" | "light" | "medium";
    enabled: boolean;
    url: string;
    key: string;
    model: string;
    timeout_ms: number;
    min_words: number;
  };
  teacher: {
    enabled: boolean;
    hour: number;
    speech_model: string;
    rewrite_model: string;
    learn_after: number;
    batch: number;
  };
  spelling: "indian" | "american";
  numbers: "indian" | "international";
  microphone: string;
  sounds: boolean;
  mute_while_dictating: boolean;
  show_bar_always: boolean;
  launch_at_login: boolean;
  retention: "keep" | "day" | "never";
  keep_audio: boolean;
  auto_learn: boolean;
  press_enter: boolean;
  theme: string; // "system" | "light" | "dark"
}

export interface Dictation {
  id: number;
  created_at: number; // Unix seconds
  offset_min: number;
  app: string;
  process: string;
  title: string;
  category: Category;
  command: boolean;
  duration_ms: number;
  speech_ms: number;
  raw: string; // what the recogniser heard
  text: string; // what was pasted
  words: number;
  rewritten: boolean;
  speech_model: string;
  rewrite_model: string;
  asr_ms: number;
  rewrite_ms: number;
  total_ms: number;
  outcome: Outcome;
  fixes: number;
  audio: string; // empty when no recording was kept
  edited: string | null;
  teacher_raw: string | null;
  teacher_text: string | null;
  teacher_wer: number | null;
  reviewed_at: number | null;
  flagged: boolean;
  trace: Span[];
}

export type Stage =
  | "stop"
  | "load"
  | "speech"
  | "settle"
  | "cleanup"
  | "paste"
  | "detect"
  | "piece"
  | "feed"
  | "model";

export interface ModelTime {
  load_ms?: number;
  prompt_ms?: number;
  answer_ms?: number;
  prompt_tokens?: number;
  answer_tokens?: number;
}

export interface Span {
  stage: Stage;
  at_ms: number;
  ms: number;
  chunk?: number;
  audio_ms?: number;
  words?: number;
  model?: ModelTime;
}

export interface Word {
  id: number;
  heard: string; // empty for a plain vocabulary word
  term: string;
  source: string; // "manual" | "edit" | "teacher"
  hits: number;
}
export interface Replacement {
  from: string;
  to: string;
}
export interface Suggestion {
  id: number;
  heard: string;
  term: string;
  count: number;
}
export interface Snippet {
  id: number;
  trigger: string;
  expansion: string;
  hits: number;
}
export interface Note {
  id: number;
  title: string;
  body: string;
  updated_at: number;
}
export interface Rule {
  pattern: string;
  category: Category;
}

export interface Day {
  date: string; // YYYY-MM-DD
  words: number;
  band: number; // 0..4
}
export interface Share {
  name: string;
  words: number;
  dictations: number;
  share: number; // 0..1
}
export interface Insights {
  words: number;
  dictations: number;
  minutes_spoken: number;
  wpm: number;
  minutes_saved: number;
  fixes: number;
  today_words: number;
  week_words: number;
  streak: number;
  longest_streak: number;
  active_days: number;
  days: Day[]; // oldest first, starts on a Monday
  apps: Share[];
  categories: Share[];
  hours: number[]; // 24 entries
  weekdays: number[]; // 7 entries, Monday first
  median_ms: number;
  p95_ms: number;
  rewritten_share: number;
  accuracy: number | null; // reserved for validated accuracy; always null for now
  teacher_agreement: number | null; // 0..1: how often the everyday model agreed with the nightly teacher
  reviewed: number;
}

export interface SpeechModel {
  id: string;
  name: string;
  bytes: number;
  notes: string;
  wants_gpu: boolean;
  streaming: boolean; // recognises speech as it is spoken
  installed: boolean;
  active: boolean; // chosen for live dictation
  teacher: boolean; // chosen for the nightly review
}

export interface Status {
  version: string;
  platform: "windows" | "macos" | "linux";
  wayland: boolean; // a Wayland session: apps cannot watch global keys
  shortcuts: boolean; // global shortcuts work; false on Wayland, or on a Mac without Accessibility
  updates: boolean; // this build can update itself (macOS)
  speech_ready: boolean;
  speech_device: string; // e.g. "vulkan NVIDIA GeForce RTX 4060"; empty until loaded
  speech_error: string; // empty when fine
  rewrite_reachable: boolean; // the language-model server answers
  rewrite_models: string[]; // models that server has
  microphones: string[];
}

/** A release newer than the running one. */
export interface Update {
  version: string;
  notes: string;
}

export interface RewriteStatus {
  reachable: boolean; // the language-model server answers
  models: string[]; // models that server has
}

export interface TeacherReport {
  reviewed: number;
  skipped: number;
  average_wer: number | null;
  learned: Replacement[];
  pending: number;
  examples: number;
}
export interface TeacherStatus {
  running: boolean;
  last_run: number | null; // Unix seconds
  report: TeacherReport | null;
  waiting: number; // dictations not reviewed yet
}

/** What the floating bar shows; sent on the "bar" event. */
export type Bar =
  | { state: "idle" }
  | { state: "listening"; command: boolean; locked: boolean }
  | { state: "processing" }
  | { state: "done"; words: number }
  | { state: "card"; text: string; message: string }
  | { state: "notice"; message: string; error: boolean; undo: boolean };

export interface Progress {
  id: string; // model id or name
  done: number;
  total: number;
  finished: boolean;
  error: string; // empty unless it failed
}

export type BarAction = "click" | "cancel" | "undo" | "dismiss";

// Commands reject with a plain sentence (string) meant to be shown as is.
export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  // Settings is recording a shortcut; the shell stops reading them meanwhile.
  pauseShortcuts: (paused: boolean) => invoke<void>("pause_shortcuts", { paused }),
  getStatus: () => invoke<Status>("get_status"),
  /** `null` when this is the newest release. */
  checkUpdate: () => invoke<Update | null>("check_update"),
  /** Replaces the app with the newest release and restarts; resolves only on failure. */
  installUpdate: () => invoke<void>("install_update"),

  history: (search: string, limit: number, offset: number) =>
    invoke<Dictation[]>("history", { search, limit, offset }),
  /** Saves a correction; returns dictionary entries learned from it. */
  editDictation: (id: number, text: string) =>
    invoke<Replacement[]>("edit_dictation", { id, text }),
  /** `null` clears all history. */
  deleteDictations: (ids: number[] | null) => invoke<void>("delete_dictations", { ids }),
  flagDictation: (id: number, flagged: boolean) => invoke<void>("flag_dictation", { id, flagged }),
  /** A data: URL of the recording, or "" if none was kept. */
  dictationAudio: (id: number) => invoke<string>("dictation_audio", { id }),
  copyText: (text: string) => invoke<void>("copy_text", { text }),

  getInsights: () => invoke<Insights>("get_insights"),

  dictionary: () => invoke<Word[]>("dictionary"),
  addWord: (heard: string, term: string) => invoke<void>("add_word", { heard, term }),
  removeWord: (id: number) => invoke<void>("remove_word", { id }),
  suggestions: () => invoke<Suggestion[]>("suggestions"),
  resolveSuggestion: (id: number, accept: boolean) =>
    invoke<void>("resolve_suggestion", { id, accept }),

  snippets: () => invoke<Snippet[]>("snippets"),
  saveSnippet: (id: number | null, trigger: string, expansion: string) =>
    invoke<number>("save_snippet", { id, trigger, expansion }),
  deleteSnippet: (id: number) => invoke<void>("delete_snippet", { id }),

  notes: () => invoke<Note[]>("notes"),
  saveNote: (id: number | null, title: string, body: string) =>
    invoke<number>("save_note", { id, title, body }),
  deleteNote: (id: number) => invoke<void>("delete_note", { id }),

  rules: () => invoke<Rule[]>("rules"),
  saveRules: (rules: Rule[]) => invoke<void>("save_rules", { rules }),

  models: () => invoke<SpeechModel[]>("models"),
  /** Progress arrives on the "model-progress" event. */
  downloadModel: (id: string) => invoke<void>("download_model", { id }),
  cancelDownload: (id: string) => invoke<void>("cancel_download", { id }),
  deleteModel: (id: string) => invoke<void>("delete_model", { id }),
  /** Asks Ollama to download a model; progress on "rewrite-progress". */
  pullRewriteModel: (model: string) => invoke<void>("pull_rewrite_model", { model }),
  /** Asks only the language-model server; cheaper than getStatus. */
  rewriteStatus: () => invoke<RewriteStatus>("rewrite_status"),

  teacherStatus: () => invoke<TeacherStatus>("teacher_status"),
  /** Starts a review now; "teacher-finished" fires when it ends. */
  runTeacher: () => invoke<void>("run_teacher"),
  /** Writes the training export and returns the file path. */
  exportTraining: () => invoke<string>("export_training"),
  exportDiagnostics: () => invoke<string>("export_diagnostics"),

  /** Formats typed text as if it had been dictated into `app`. */
  tryFormat: (text: string, app: string) => invoke<string>("try_format", { text, app }),

  barAction: (action: BarAction) => invoke<void>("bar_action", { action }),
  barHit: (rects: number[][]) => invoke<void>("bar_hit", { rects }),
  openMain: (page: string) => invoke<void>("open_main", { page }),
};

export const events = {
  bar: (handler: (bar: Bar) => void) => on<Bar>("bar", handler),
  /** Microphone loudness 0..1, about 20 times a second while listening. */
  level: (handler: (level: number) => void) => on<number>("level", handler),
  barHover: (handler: (over: boolean) => void) => on<boolean>("bar-hover", handler),
  historyChanged: (handler: () => void) => on<null>("history-changed", () => handler()),
  modelProgress: (handler: (progress: Progress) => void) => on<Progress>("model-progress", handler),
  rewriteProgress: (handler: (progress: Progress) => void) =>
    on<Progress>("rewrite-progress", handler),
  teacherFinished: (handler: (result: { report: TeacherReport | null; error: string }) => void) =>
    on<{ report: TeacherReport | null; error: string }>("teacher-finished", handler),
  /** The shell asks the main window to show a page (e.g. from the tray). */
  navigate: (handler: (page: string) => void) => on<string>("navigate", handler),
};

function on<T>(name: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(name, (event) => handler(event.payload));
}
