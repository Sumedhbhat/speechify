import fs from "node:fs";
import path from "node:path";
import { createHash, randomUUID } from "node:crypto";
import { parse, stringify } from "smol-toml";
import defaults from "./defaults.json";
export type Config = typeof defaults & {
  theme: string;
  startup: boolean;
  inference_python: string;
  training_python: string;
};
export function atomic(file: string, data: string | Buffer) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = file + "." + randomUUID() + ".tmp";
  try {
    fs.writeFileSync(tmp, data, { mode: 0o600 });
    fs.renameSync(tmp, file);
  } finally {
    fs.rmSync(tmp, { force: true });
  }
}
export function owned(root: string, name: string) {
  const base = fs.realpathSync(root);
  const dest = path.resolve(base, name);
  if (!dest.startsWith(base + path.sep))
    throw Error("Path is outside candidate storage");
  if (fs.existsSync(dest) && !fs.realpathSync(dest).startsWith(base + path.sep))
    throw Error("Symlink escapes candidate storage");
  return dest;
}
export async function hash(file: string) {
  const h = createHash("sha256");
  for await (const chunk of fs.createReadStream(file)) h.update(chunk);
  return h.digest("hex");
}
export class Settings {
  value: Config;
  constructor(public root: string) {
    fs.mkdirSync(root, { recursive: true });
    this.value = {
      ...defaults,
      model: fs.existsSync("/proc/driver/nvidia/version")
        ? "whisper-large-v3-turbo"
        : "parakeet-tdt-0.6b-v3",
      compute_type: fs.existsSync("/proc/driver/nvidia/version")
        ? "float16"
        : "int8",
      ptt_key: process.platform === "darwin" ? "alt_r" : "ctrl_r",
      theme: "system",
      startup: false,
      inference_python: "",
      training_python: "",
    };
    const p = path.join(root, "config.toml");
    if (fs.existsSync(p))
      this.value = this.validate(parse(fs.readFileSync(p, "utf8")));
    else this.save({});
  }
  validate(patch: Record<string, unknown>): Config {
    const next = { ...this.value };
    for (const [k, v] of Object.entries(patch)) {
      if (!(k in next)) throw Error("Unknown setting: " + k);
      if (typeof v !== typeof next[k as keyof Config])
        throw Error("Invalid type: " + k);
      (next as any)[k] = v;
    }
    for (const k of [
      "min_hold_ms",
      "autocorrect_after",
      "beam_size",
      "onnx_threads",
      "idle_minutes",
      "retrain_after_labels",
      "retrain_after_days",
      "max_regression_wer",
      "keep_versions",
    ] as const) {
      if (!Number.isFinite(next[k]) || next[k] < 0 || next[k] > 100000)
        throw Error("Out of range: " + k);
    }
    if (
      next.min_hold_ms > 5000 ||
      next.keep_versions < 1 ||
      next.autocorrect_after < 1 ||
      next.beam_size < 1 ||
      !Number.isFinite(next.teacher_min_confidence) ||
      next.teacher_min_confidence < 0 ||
      next.teacher_min_confidence > 1
    )
      throw Error("Invalid setting range");
    if (!["auto", "cpu", "cuda", "vulkan", "metal"].includes(next.device))
      throw Error("Unknown device");
    if (!["system", "dark", "light"].includes(next.theme))
      throw Error("Unknown theme");
    if (next.llm_url && !/^https?:\/\//.test(next.llm_url))
      throw Error("Cleanup URL requires http or https");
    return next;
  }
  save(patch: Record<string, unknown>) {
    const next = this.validate(patch);
    atomic(path.join(this.root, "config.toml"), stringify(next));
    this.value = next;
    return next;
  }
  export() {
    return stringify(this.value);
  }
  import(text: string) {
    return this.save(parse(text));
  }
}
export class HoldKey {
  private down = new Set<number>();
  private started: number | null = null;
  private cancelled = false;
  constructor(
    public keys: number[],
    public minMs: number,
    private start: () => void,
    private end: (accept: boolean) => void,
  ) {}
  event(code: number, pressed: boolean, now = performance.now()) {
    if (pressed) {
      if (this.down.has(code)) return;
      this.down.add(code);
      if (this.started !== null && !this.keys.includes(code)) {
        this.cancelled = true;
        this.end(false);
        this.started = null;
      } else if (
        this.started === null &&
        !this.cancelled &&
        this.down.size === this.keys.length &&
        this.keys.every((k) => this.down.has(k))
      ) {
        this.started = now;
        this.start();
      }
    } else {
      this.down.delete(code);
      if (this.started !== null && this.keys.includes(code)) {
        const ok = !this.cancelled && now - this.started >= this.minMs;
        this.started = null;
        this.end(ok);
      }
      if (!this.down.size) this.cancelled = false;
    }
  }
  cancel() {
    if (this.started !== null) this.end(false);
    this.started = null;
    this.down.clear();
    this.cancelled = false;
  }
}
export class BoundedQueue {
  private tail = Promise.resolve();
  private size = 0;
  private generation = 0;
  constructor(public limit = 4) {}
  cancel() {
    this.generation++;
  }
  run<T>(job: () => Promise<T>): Promise<T> {
    if (this.size >= this.limit)
      return Promise.reject(Error("Dictation queue full (4 clips)"));
    this.size++;
    const generation = this.generation;
    const p = this.tail.then(() => {
      if (generation !== this.generation) throw Error("Cancelled");
      return job();
    });
    this.tail = p.then(
      () => {},
      () => {},
    );
    return p.finally(() => this.size--);
  }
}
export function label(row: any, min = 0.7): string | null {
  if (row.excluded || !row.audio) return null;
  if (row.corrected?.trim()) return row.corrected.trim();
  if (row.teacher_note || !row.teacher_ts || row.teacher_confidence < min)
    return null;
  return row.teacher_text?.trim() || null;
}
export function edits(before: string, after: string): [string, string][] {
  const a = before.trim().split(/\s+/),
    b = after.trim().split(/\s+/);
  if (a.length > 1000 || b.length > 1000) return [];
  const strip = (s: string) =>
    s.replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "");
  const norm = (s: string) => strip(s).toLowerCase();
  const dp = Array.from(
    { length: a.length + 1 },
    () => new Uint16Array(b.length + 1),
  );
  for (let i = a.length - 1; i >= 0; i--)
    for (let j = b.length - 1; j >= 0; j--)
      dp[i][j] =
        norm(a[i]) === norm(b[j])
          ? 1 + dp[i + 1][j + 1]
          : Math.max(dp[i + 1][j], dp[i][j + 1]);
  const out: [string, string][] = [];
  let i = 0,
    j = 0;
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && norm(a[i]) === norm(b[j])) {
      if (strip(a[i]) !== strip(b[j])) out.push([norm(a[i]), strip(b[j])]);
      i++;
      j++;
      continue;
    }
    const ai = i,
      bj = j;
    while (
      (i < a.length || j < b.length) &&
      !(i < a.length && j < b.length && norm(a[i]) === norm(b[j]))
    ) {
      if (j === b.length || (i < a.length && dp[i + 1][j] >= dp[i][j + 1])) i++;
      else j++;
    }
    if (i > ai && j > bj && i - ai <= 4 && j - bj <= 4)
      out.push([
        a.slice(ai, i).map(norm).join(" "),
        b.slice(bj, j).map(strip).join(" "),
      ]);
  }
  return out.filter(([a, b]) => a && b);
}
export function pcmWav(pcm: Buffer) {
  const h = Buffer.alloc(44);
  h.write("RIFF");
  h.writeUInt32LE(36 + pcm.length, 4);
  h.write("WAVEfmt ", 8);
  h.writeUInt32LE(16, 16);
  h.writeUInt16LE(1, 20);
  h.writeUInt16LE(1, 22);
  h.writeUInt32LE(16000, 24);
  h.writeUInt32LE(32000, 28);
  h.writeUInt16LE(2, 32);
  h.writeUInt16LE(16, 34);
  h.write("data", 36);
  h.writeUInt32LE(pcm.length, 40);
  return Buffer.concat([h, pcm]);
}
export function readWav(file: string): Float32Array {
  const b = fs.readFileSync(file);
  if (
    b.toString("ascii", 0, 4) !== "RIFF" ||
    b.toString("ascii", 8, 12) !== "WAVE"
  )
    throw Error("Expected WAV");
  let rate = 0,
    channels = 0,
    bits = 0,
    format = 0,
    data: Buffer | undefined;
  for (let p = 12; p + 8 <= b.length;) {
    const n = b.readUInt32LE(p + 4);
    if (p + 8 + n > b.length) throw Error("Truncated WAV");
    const tag = b.toString("ascii", p, p + 4);
    if (tag === "fmt ") {
      format = b.readUInt16LE(p + 8);
      channels = b.readUInt16LE(p + 10);
      rate = b.readUInt32LE(p + 12);
      bits = b.readUInt16LE(p + 22);
    }
    if (tag === "data") data = b.subarray(p + 8, p + 8 + n);
    p += 8 + n + (n % 2);
  }
  if (!data || format !== 1 || bits !== 16 || channels < 1 || rate !== 16000)
    throw Error(
      "Fixture requires 16 kHz PCM16 WAV; service capture resamples before submission",
    );
  const out = new Float32Array(data.length / 2 / channels);
  for (let i = 0; i < out.length; i++)
    for (let c = 0; c < channels; c++)
      out[i] += data.readInt16LE((i * channels + c) * 2) / 32768 / channels;
  return out;
}
export class Timings {
  constructor(private root: string) {}
  log(event: string, fields: Record<string, unknown> = {}) {
    const dir = path.join(this.root, "logs");
    fs.mkdirSync(dir, { recursive: true });
    const file = path.join(dir, "timings.jsonl");
    if (fs.existsSync(file) && fs.statSync(file).size > 2e6) {
      fs.rmSync(file + ".1", { force: true });
      fs.renameSync(file, file + ".1");
    }
    fs.appendFileSync(
      file,
      JSON.stringify({ ts: new Date().toISOString(), event, ...fields }) + "\n",
      { mode: 0o600 },
    );
  }
}

// Respect native family/session limits, cutting near silence without dropping samples.
export function splitAudio(pcm: Float32Array, limit: number): Float32Array[] {
  if (!Number.isSafeInteger(limit) || limit <= 0 || pcm.length <= limit)
    return [pcm];
  const pieces: Float32Array[] = [];
  let start = 0;
  while (pcm.length - start > limit) {
    const lo = Math.floor(limit * 0.8),
      window = Math.min(1600, limit - lo);
    let cut = limit,
      best = Infinity;
    for (let i = lo; i + window <= limit; i += window) {
      let energy = 0;
      for (let j = 0; j < window; j++) energy += pcm[start + i + j] ** 2;
      if (energy < best) {
        best = energy;
        cut = Math.max(1, i + Math.floor(window / 2));
      }
    }
    pieces.push(pcm.subarray(start, start + cut));
    start += cut;
  }
  pieces.push(pcm.subarray(start));
  return pieces;
}
