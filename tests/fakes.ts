// Plain fakes for the api/events boundary, plus small test helpers.

import { vi, type Mock } from "vitest";
import type { Dictation, Insights, Settings, SpeechModel, Status, TeacherStatus } from "../src/api";
import type { Api, Deps, Events } from "../src/deps";
import { createSettingsStore } from "../src/settings";

export type FakeApi = { [K in keyof Api]: Mock<Api[K]> };

export function settings(): Settings {
  return {
    hotkeys: {
      push_to_talk: "Ctrl+Meta+Space",
      hands_free: "Ctrl+Meta+H",
      command: "Ctrl+Meta+C",
      paste_last: "Ctrl+Meta+V",
      copy_last: "Ctrl+Meta+X",
      cancel: "Escape",
    },
    styles: { personal: "casual", work: "formal", email: "formal", other: "casual" },
    speech: {
      model: "turbo",
      device: "auto",
      language: "en",
      remote_url: "",
      remote_key: "",
      remote_model: "",
    },
    rewrite: {
      level: "light",
      enabled: true,
      url: "http://127.0.0.1:11434",
      key: "",
      model: "qwen2.5:1.5b",
      timeout_ms: 3000,
      min_words: 4,
    },
    teacher: {
      enabled: false,
      hour: 3,
      speech_model: "large",
      rewrite_model: "qwen2.5:7b",
      learn_after: 3,
      batch: 50,
    },
    spelling: "indian",
    numbers: "indian",
    microphone: "",
    sounds: true,
    mute_while_dictating: false,
    show_bar_always: true,
    launch_at_login: false,
    retention: "keep",
    keep_audio: true,
    auto_learn: true,
    press_enter: false,
    theme: "system",
  };
}

export function status(over: Partial<Status> = {}): Status {
  return {
    version: "1.2.3",
    platform: "linux",
    wayland: false,
    shortcuts: true,
    speech_ready: true,
    speech_device: "vulkan NVIDIA GeForce RTX 4060",
    speech_error: "",
    rewrite_reachable: true,
    rewrite_models: ["qwen2.5:1.5b", "llama3.2:3b"],
    microphones: ["USB Mic", "Built-in"],
    ...over,
  };
}

export function insights(over: Partial<Insights> = {}): Insights {
  return {
    words: 12_345,
    dictations: 321,
    minutes_spoken: 95,
    wpm: 142,
    minutes_saved: 210,
    fixes: 17,
    today_words: 250,
    week_words: 1800,
    streak: 4,
    longest_streak: 12,
    active_days: 40,
    days: Array.from({ length: 14 }, (_, i) => ({
      date: new Date(Date.UTC(2026, 9, 26 + i)).toISOString().slice(0, 10),
      words: i * 10,
      band: i % 5,
    })),
    apps: [{ name: "Slack", words: 800, dictations: 20, share: 0.6 }],
    categories: [
      { name: "work", words: 800, dictations: 20, share: 0.6 },
      { name: "mystery", words: 10, dictations: 1, share: 0.01 },
    ],
    hours: Array.from({ length: 24 }, (_, i) => i),
    weekdays: [1, 2, 3, 4, 5, 6, 7],
    median_ms: 820,
    p95_ms: 1900,
    rewritten_share: 0.42,
    accuracy: 0.93,
    reviewed: 12,
    ...over,
  };
}

export function dictation(over: Partial<Dictation> = {}): Dictation {
  return {
    id: 1,
    created_at: Math.floor(Date.now() / 1000),
    offset_min: -new Date().getTimezoneOffset(),
    app: "Slack",
    process: "slack",
    title: "general",
    category: "work",
    command: false,
    duration_ms: 3000,
    speech_ms: 2500,
    raw: "um hello there",
    text: "Hello there.",
    words: 2,
    rewritten: true,
    speech_model: "Whisper turbo",
    rewrite_model: "qwen2.5:1.5b",
    asr_ms: 320,
    rewrite_ms: 210,
    total_ms: 600,
    outcome: "pasted",
    fixes: 0,
    audio: "",
    edited: null,
    teacher_raw: null,
    teacher_text: null,
    teacher_wer: null,
    reviewed_at: null,
    flagged: false,
    ...over,
  };
}

export function model(over: Partial<SpeechModel> = {}): SpeechModel {
  return {
    id: "turbo",
    name: "Whisper turbo",
    bytes: 1_600_000_000,
    notes: "Fast and accurate on a graphics card.",
    wants_gpu: true,
    installed: true,
    active: true,
    teacher: false,
    ...over,
  };
}

export function teacher(over: Partial<TeacherStatus> = {}): TeacherStatus {
  return { running: false, last_run: null, report: null, waiting: 5, ...over };
}

export function fakeApi(): FakeApi {
  return {
    getSettings: vi.fn(async () => settings()),
    saveSettings: vi.fn(async () => undefined),
    getStatus: vi.fn(async () => status()),
    history: vi.fn(async () => []),
    editDictation: vi.fn(async () => []),
    deleteDictations: vi.fn(async () => undefined),
    flagDictation: vi.fn(async () => undefined),
    dictationAudio: vi.fn(async () => ""),
    copyText: vi.fn(async () => undefined),
    getInsights: vi.fn(async () => insights()),
    dictionary: vi.fn(async () => []),
    addWord: vi.fn(async () => undefined),
    removeWord: vi.fn(async () => undefined),
    suggestions: vi.fn(async () => []),
    resolveSuggestion: vi.fn(async () => undefined),
    snippets: vi.fn(async () => []),
    saveSnippet: vi.fn(async () => 1),
    deleteSnippet: vi.fn(async () => undefined),
    notes: vi.fn(async () => []),
    saveNote: vi.fn(async () => 1),
    deleteNote: vi.fn(async () => undefined),
    rules: vi.fn(async () => []),
    saveRules: vi.fn(async () => undefined),
    models: vi.fn(async () => [model()]),
    downloadModel: vi.fn(async () => undefined),
    cancelDownload: vi.fn(async () => undefined),
    deleteModel: vi.fn(async () => undefined),
    pullRewriteModel: vi.fn(async () => undefined),
    teacherStatus: vi.fn(async () => teacher()),
    runTeacher: vi.fn(async () => undefined),
    exportTraining: vi.fn(async () => "/home/me/speechify-training.jsonl"),
    tryFormat: vi.fn(async (input: string) => `formatted: ${input}`),
    barAction: vi.fn(async () => undefined),
    openMain: vi.fn(async () => undefined),
  };
}

type Handler = (payload: never) => void;

/** Events whose handlers tests can fire with `emit`. */
export function fakeEvents() {
  const handlers = new Map<string, Handler[]>();
  const on = (name: string) => (handler: Handler) => {
    handlers.set(name, [...(handlers.get(name) ?? []), handler]);
    return Promise.resolve(() => undefined);
  };
  const events = {
    bar: on("bar"),
    level: on("level"),
    historyChanged: on("historyChanged"),
    modelProgress: on("modelProgress"),
    rewriteProgress: on("rewriteProgress"),
    teacherFinished: on("teacherFinished"),
    navigate: on("navigate"),
  } as unknown as Events;
  const emit = <K extends keyof Events>(
    name: K,
    payload?: Parameters<Parameters<Events[K]>[0]>[0],
  ) => {
    for (const handler of handlers.get(name) ?? []) (handler as (p: unknown) => void)(payload);
  };
  return { events, emit, handlers };
}

export function makeDeps() {
  const api = fakeApi();
  const { events, emit } = fakeEvents();
  const onSaved = vi.fn();
  const onError = vi.fn();
  const navigate = vi.fn();
  const store = createSettingsStore(api, onSaved, onError);
  const deps: Deps = { api, events, settings: store, navigate };
  return { deps, api, events, emit, onSaved, onError, navigate };
}

/** Lets pending promise callbacks run (works with fake timers too). */
export async function settle(): Promise<void> {
  for (let i = 0; i < 200; i++) await Promise.resolve();
}

export function byText<T extends HTMLElement = HTMLElement>(
  root: ParentNode,
  selector: string,
  text: string,
): T {
  const found = [...root.querySelectorAll<T>(selector)].find(
    (el) => el.textContent?.trim() === text,
  );
  if (!found) throw new Error(`No ${selector} with text "${text}"`);
  return found;
}

export function byLabel<T extends HTMLElement = HTMLElement>(root: ParentNode, label: string): T {
  const found = root.querySelector<T>(`[aria-label="${label}"]`);
  if (found) return found;
  const forLabel = byText<HTMLLabelElement>(root, "label", label);
  return document.getElementById(forLabel.htmlFor) as T;
}

export function change(
  el: HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement,
  value: string | boolean,
) {
  if (typeof value === "boolean") (el as HTMLInputElement).checked = value;
  else el.value = value;
  el.dispatchEvent(new Event("change"));
}

export function type(el: HTMLInputElement | HTMLTextAreaElement, value: string) {
  el.value = value;
  el.dispatchEvent(new Event("input"));
}

export function toasts(): string[] {
  return [...document.querySelectorAll(".toast")].map((el) => el.textContent ?? "");
}
