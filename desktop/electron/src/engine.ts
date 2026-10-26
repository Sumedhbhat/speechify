import { fork, ChildProcess } from "node:child_process";
import path from "node:path";
import fs from "node:fs";
import { BoundedQueue, Timings } from "./core";
export class Engine {
  private child?: ChildProcess;
  private pending = new Map<
    number,
    {
      resolve: (x: any) => void;
      reject: (x: any) => void;
      timer: NodeJS.Timeout;
    }
  >();
  private seq = 0;
  private generation = 0;
  private queue = new BoundedQueue(4);
  private loading?: Promise<any>;
  private options: any;
  device = "unloaded";
  constructor(
    private native: string,
    private logs: Timings,
  ) {}
  private request(msg: any) {
    return new Promise<any>((resolve, reject) => {
      const id = ++this.seq;
      const timer = setTimeout(() => {
        this.stop();
        reject(Error("Worker timed out"));
      }, 300000);
      this.pending.set(id, { resolve, reject, timer });
      this.child!.send({ ...msg, id });
    });
  }
  async load(options: any, preserveQueue = false) {
    this.stop(preserveQueue);
    this.options = options;
    this.child = fork(path.join(__dirname, "native-worker.js"), [], {
      env: {
        ...process.env,
        ELECTRON_RUN_AS_NODE: "1",
        SPEECHIFY_NATIVE: this.native,
      },
      stdio: ["ignore", "ignore", "pipe", "ipc"],
      serialization: "advanced",
      detached: process.platform !== "win32",
    });
    let logBytes = 0;
    this.child.stderr?.on("data", (b: Buffer) => {
      // Native libraries can print decoded tokens: keep only byte counts in diagnostics.
      logBytes += b.length;
      if (logBytes > 1e6) logBytes = 0;
    });
    this.child.on("message", (m: any) => {
      const p = this.pending.get(m.id);
      if (!p) return;
      clearTimeout(p.timer);
      this.pending.delete(m.id);
      m.error ? p.reject(Error(m.error)) : p.resolve(m);
    });
    const current = this.child;
    this.child.on("error", (e) => {
      if (this.child === current) this.fail(e);
    });
    this.child.on("exit", (code, signal) => {
      this.logs.log("worker_exit", { code, signal });
      if (this.child === current)
        this.fail(Error("Inference worker exited; next dictation will reload"));
    });
    this.loading = this.request({ type: "load", ...options }).then((r) => {
      this.device = r.device;
      this.logs.log("model_ready", { ms: r.ms, device: r.device });
      return r;
    });
    return this.loading;
  }
  private fail(e: Error) {
    for (const p of this.pending.values()) {
      clearTimeout(p.timer);
      p.reject(e);
    }
    this.pending.clear();
    this.child = undefined;
    this.device = "unloaded";
  }
  run(pcm: Float32Array, prompt = "") {
    if (pcm.length > 16000 * 120)
      return Promise.reject(Error("Clip exceeds 120 seconds"));
    return this.queue.run(async () => {
      if (!this.child) {
        if (!this.options) throw Error("Choose an installed model");
        await this.load(this.options, true);
      }
      await this.loading;
      const generation = this.generation;
      let r;
      try {
        r = await this.request({ type: "run", pcm: Array.from(pcm), prompt });
      } catch (e) {
        // A process crash can bypass the native library's own GPU fallback.
        // Retry this audio once on CPU; explicit cancellation never retries.
        if (generation !== this.generation || this.child) throw e;
        await this.load({ ...this.options, device: "cpu" }, true);
        r = await this.request({ type: "run", pcm: Array.from(pcm), prompt });
      }
      this.device = r.device;
      this.logs.log("inference", {
        ms: r.ms,
        device: r.device,
        samples: pcm.length,
      });
      return r;
    });
  }
  stop(preserveQueue = false) {
    if (!preserveQueue) {
      this.generation++;
      this.queue.cancel();
    }
    const c = this.child;
    this.child = undefined;
    if (c?.pid) {
      if (process.platform === "win32") {
        require("node:child_process").spawn(
          "taskkill",
          ["/pid", String(c.pid), "/T", "/F"],
          { windowsHide: true },
        );
      } else {
        try {
          process.kill(-c.pid, "SIGKILL");
        } catch {
          c.kill("SIGKILL");
        }
      }
    }
    this.fail(Error("Cancelled"));
    this.loading = undefined;
  }
}
