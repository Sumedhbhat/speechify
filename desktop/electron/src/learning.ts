import fs from "node:fs";
import path from "node:path";
import { spawn, ChildProcess } from "node:child_process";
import { deflateSync } from "node:zlib";
import { Settings, readWav, label, owned, Timings } from "./core";
import { Store } from "./store";
import { Models } from "./models";
import { Engine } from "./engine";
import { killTree, PythonEngine } from "./python-engine";
export function judge(
  text: string,
  pcm: Float32Array,
  confidence: number | null,
) {
  const words = text.match(/[\p{L}\p{N}]+/gu) || [];
  let voiced = 0;
  for (let i = 0; i + 480 <= pcm.length; i += 480) {
    let power = 0;
    for (let j = i; j < i + 480; j++) power += pcm[j] ** 2;
    if (Math.sqrt(power / 480) > 0.004) voiced += 0.03;
  }
  let note: string | null = null;
  if (confidence !== null && !Number.isFinite(confidence))
    return { confidence: 0, note: "invalid confidence" };
  if (!words.length) note = "empty";
  else if (words.length > voiced * 7 + 3) note = "too long for clip";
  else if (voiced >= 3 && words.length < voiced * 0.3)
    note = "too short for clip";
  else if (
    text.length >= 60 &&
    Buffer.byteLength(text) / deflateSync(text).length > 2.4
  )
    note = "repeated phrase";
  const c =
    confidence ??
    Math.max(
      0,
      0.8 - Math.max(0, words.length / Math.max(1, voiced) - 4.5) * 0.15,
    );
  return { confidence: note ? 0 : Math.max(0, Math.min(1, c)), note };
}
export function gatesPass(report: any) {
  return (
    report?.status === "accepted" &&
    report.gate1?.passed === true &&
    report.gate2?.passed === true &&
    report.gate1.candidate < report.gate1.current
  );
}
export class Learning {
  private teacher?: Engine | PythonEngine;
  private training?: ChildProcess;
  private timer?: NodeJS.Timeout;
  private cancelled = false;
  private running = false;
  constructor(
    private settings: Settings,
    private store: Store,
    private models: Models,
    private resources: string,
    private logs: Timings,
    private update: () => void,
    private idle: () => boolean,
    private release: () => void,
    private reload: (id: string) => Promise<void>,
  ) {}
  start() {
    this.timer = setInterval(() => {
      if (this.settings.value.learn_auto && this.idle() && !this.running)
        void this.tick().catch((e) => {
          this.store.setState("teacher_status", String(e));
          this.update();
        });
    }, 30000);
  }
  private async tick() {
    const cfg = this.settings.value;
    if (cfg.teacher !== "off") await this.runTeacher();
    if (!this.idle()) return;
    const rows = this.store.db.prepare("SELECT * FROM samples").all() as any[];
    const count = rows.filter((r) =>
      label(r, cfg.teacher_min_confidence),
    ).length;
    const last = this.store.state("last_training", {
      count: 0,
      ts: Math.min(
        ...rows
          .filter((r) => label(r, cfg.teacher_min_confidence))
          .map((r) => (r.teacher_ts || r.ts) * 1000),
      ),
    });
    if (
      count >= 50 &&
      (count - last.count >= cfg.retrain_after_labels ||
        Date.now() - last.ts > cfg.retrain_after_days * 86400000)
    )
      await this.train();
  }
  async runTeacher() {
    if (this.running) throw Error("Learning is already running");
    const cfg = this.settings.value;
    if (cfg.teacher === "off") throw Error("Teacher is off");
    if (!this.idle())
      throw Error("Teacher waits for idle time and mains power");
    if (!cfg.teacher_on_cpu && cfg.device === "cpu")
      throw Error("CPU teacher requires explicit opt-in");
    const id = cfg.teacher === "auto" ? "granite-speech-4.1-2b" : cfg.teacher;
    const spec = this.models.list().find((m) => m.id === id);
    if (!spec) throw Error("Unknown teacher");
    const file = spec.gguf ? await this.models.file(id) : undefined;
    if (!file && !cfg.inference_python)
      throw Error("Optional teacher requires inference Python executable");
    if (this.running || !this.idle())
      throw Error("Learning preempted before model load");
    this.running = true;
    this.cancelled = false;
    this.release();
    this.teacher = file
      ? new Engine(path.join(this.resources, "native"), this.logs)
      : new PythonEngine(
          path.join(this.resources, "python"),
          this.settings.root,
          this.logs,
        );
    this.store.setState("teacher_status", "Loading " + id);
    this.update();
    try {
      const optionalRepo = file
        ? undefined
        : id.startsWith("ft:")
          ? spec.repo
          : await this.models.optionalFile(id);
      if (this.cancelled || !this.idle()) throw Error("Teacher preempted");
      await this.teacher.load(
        file
          ? {
              file,
              device: cfg.device,
              threads: cfg.onnx_threads,
              language: cfg.language,
            }
          : {
              python: cfg.inference_python,
              model: id,
              repo: optionalRepo,
              install: false,
              opts: {
                device: ["auto", "vulkan", "metal"].includes(cfg.device)
                  ? "cpu"
                  : cfg.device,
                compute_type: cfg.compute_type,
                language: cfg.language,
                beam_size: cfg.beam_size,
                onnx_threads: cfg.onnx_threads,
              },
            },
      );
      if (
        !cfg.teacher_on_cpu &&
        this.teacher.device.toLowerCase().includes("cpu")
      )
        throw Error("CPU teacher requires explicit opt-in");
      const rows = this.store.db
        .prepare(
          "SELECT * FROM samples WHERE teacher_ts IS NULL AND excluded=0 AND audio IS NOT NULL ORDER BY ts DESC LIMIT 50",
        )
        .all() as any[];
      for (const r of rows) {
        if (this.cancelled || !this.idle()) break;
        const pcm = readWav(this.store.audioFile(r.id));
        const result = await this.teacher.run(pcm);
        const judged = judge(result.text, pcm, result.confidence);
        this.store.teacher(
          r.id,
          result.text,
          id,
          judged.confidence,
          judged.note,
        );
        this.store.setState("teacher_status", "Labeled recording " + r.id);
        this.update();
      }
      this.store.setState(
        "teacher_status",
        this.cancelled
          ? "Paused for dictation / power"
          : "Teacher batch complete",
      );
    } finally {
      this.teacher.stop();
      this.teacher = undefined;
      this.running = false;
      if (!this.cancelled) await this.reload(cfg.model);
      this.update();
    }
  }
  async train() {
    if (this.running) throw Error("Learning is already running");
    if (!this.idle())
      throw Error("Training waits for idle time and mains power");
    const cfg = this.settings.value;
    const versions = this.store.state("versions", []) as any[];
    const current = versions.find(
      (v) =>
        v.path === cfg.model || "ft:" + path.basename(v.path) === cfg.model,
    );
    const base = current?.base || cfg.model;
    if (!["whisper-large-v3-turbo", "whisper-large-v3"].includes(base))
      throw Error("Fine-tuning supports Whisper large-v3 and turbo only");
    const python = cfg.training_python;
    if (!python)
      throw Error(
        "Set training Python executable in Learning settings; requires isolated CUDA training dependencies",
      );
    const count = (
      this.store.db.prepare("SELECT * FROM samples").all() as any[]
    ).filter((r) => label(r, cfg.teacher_min_confidence)).length;
    if (count < 50)
      throw Error(
        "Training needs at least 50 human or confident teacher labels",
      );
    this.release();
    this.running = true;
    this.cancelled = false;
    const reportFile = path.join(this.settings.root, "training-report.json");
    fs.rmSync(reportFile, { force: true });
    this.store.setState(
      "training_status",
      "Training: held-out and LibriSpeech gates required",
    );
    this.update();
    const c = (this.training = spawn(
      python,
      [
        path.join(this.resources, "python/scripts/finetune.py"),
        "--auto",
        "--report",
        reportFile,
        "--base-id",
        base,
        "--base",
        base === "whisper-large-v3"
          ? "openai/whisper-large-v3"
          : "openai/whisper-large-v3-turbo",
        "--current",
        current?.hf ||
          (base === "whisper-large-v3"
            ? "openai/whisper-large-v3"
            : "openai/whisper-large-v3-turbo"),
        "--min-confidence",
        String(cfg.teacher_min_confidence),
        "--max-regression",
        String(cfg.max_regression_wer),
      ],
      {
        detached: process.platform !== "win32",
        windowsHide: true,
        stdio: "ignore",
        env: {
          ...process.env,
          SPEECHIFY_HOME: this.settings.root,
          HF_HOME: path.join(this.settings.root, "hf-cache"),
          PYTHONNOUSERSITE: "1",
        },
      },
    ));
    try {
      await new Promise<void>((resolve, reject) => {
        c.on("error", reject);
        c.on("exit", (code) =>
          code === 0
            ? resolve()
            : reject(
                Error(
                  this.cancelled
                    ? "Training cancelled"
                    : "Training failed; inspect training report",
                ),
              ),
        );
      });
      const report = JSON.parse(fs.readFileSync(reportFile, "utf8"));
      this.store.setState("training_report", report);
      if (!gatesPass(report))
        throw Error("Training rejected by evaluation gates");
      const model = owned(
        path.join(this.settings.root, "models"),
        path.basename(report.model),
      );
      if (
        path.resolve(report.model) !== model ||
        !fs.existsSync(path.join(model, "model.bin"))
      )
        throw Error("Trainer returned an unsafe model path");
      const entry = {
        name: path.basename(model),
        path: model,
        hf: report.hf,
        base,
        ts: Date.now() / 1000,
        gate1: report.gate1,
        gate2: report.gate2,
      };
      versions.push(entry);
      this.store.setState("versions", versions);
      await this.reload("ft:" + entry.name);
      this.prune();
      this.store.setState("last_training", { count, ts: Date.now() });
      this.store.setState("training_status", "Adopted " + entry.name);
    } catch (e) {
      this.store.setState("training_status", String(e));
      throw e;
    } finally {
      this.training = undefined;
      this.running = false;
      this.update();
    }
  }
  prune() {
    const list = this.store.state("versions", []) as any[],
      keep = this.settings.value.keep_versions;
    while (list.length > keep) {
      const i = list.findIndex(
        (v) =>
          this.settings.value.model !== v.path &&
          this.settings.value.model !== "ft:" + v.name,
      );
      if (i < 0) break;
      const v = list.splice(i, 1)[0];
      for (const p of [v.path, v.hf])
        if (p) {
          const safe = owned(
            path.join(this.settings.root, "models"),
            path.basename(p),
          );
          if (safe === path.resolve(p))
            fs.rmSync(safe, { recursive: true, force: true });
        }
    }
    this.store.setState("versions", list);
  }
  async rollback() {
    const list = this.store.state("versions", []) as any[];
    const i = list.findIndex(
      (v) =>
        v.path === this.settings.value.model ||
        "ft:" + v.name === this.settings.value.model,
    );
    if (i < 0) throw Error("Active model is not an adopted training version");
    await this.reload(i ? "ft:" + list[i - 1].name : list[i].base);
  }
  preempt() {
    this.cancelled = true;
    this.teacher?.stop();
    killTree(this.training);
    this.store.setState("teacher_status", "Paused for dictation / power");
  }
  stop() {
    if (this.timer) clearInterval(this.timer);
    this.preempt();
  }
}
