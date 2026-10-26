import { spawn, ChildProcess, execFile } from "node:child_process";
import path from "node:path";
import { createInterface } from "node:readline";
import { BoundedQueue, Timings } from "./core";
export function killTree(c?: ChildProcess) {
  if (!c?.pid) return;
  if (process.platform === "win32")
    execFile(
      "taskkill",
      ["/PID", String(c.pid), "/T", "/F"],
      { windowsHide: true },
      () => {},
    );
  else
    try {
      process.kill(-c.pid, "SIGKILL");
    } catch {
      c.kill("SIGKILL");
    }
}
export class PythonEngine {
  private child?: ChildProcess;
  private queue = new BoundedQueue();
  private id = 0;
  private generation = 0;
  private pending = new Map<
    number,
    {
      resolve: (r: any) => void;
      reject: (e: any) => void;
      timer: NodeJS.Timeout;
    }
  >();
  private options: any;
  device = "unloaded";
  constructor(
    private resources: string,
    private root: string,
    private logs: Timings,
  ) {}
  private request(msg: any) {
    if (!this.child) throw Error("Python worker unavailable");
    return new Promise<any>((resolve, reject) => {
      const id = ++this.id;
      const timer = setTimeout(() => {
        this.stop();
        reject(Error("Python worker timed out"));
      }, 600000);
      this.pending.set(id, { resolve, reject, timer });
      this.child!.stdin!.write(JSON.stringify({ ...msg, id }) + "\n");
    });
  }
  async load(options: any, preserveQueue = false) {
    this.stop(preserveQueue);
    this.options = options;
    const c = (this.child = spawn(
      options.python,
      [path.join(this.resources, "worker.py")],
      {
        stdio: ["pipe", "pipe", "ignore"],
        detached: process.platform !== "win32",
        windowsHide: true,
        env: {
          ...process.env,
          HF_HOME: path.join(this.root, "hf-cache"),
          HF_HUB_OFFLINE: "1",
          TRANSFORMERS_OFFLINE: "1",
          SPEECHIFY_HOME: this.root,
          PYTHONNOUSERSITE: "1",
        },
      },
    ));
    createInterface({ input: c.stdout! }).on("line", (line) => {
      try {
        const m = JSON.parse(line),
          p = this.pending.get(m.id);
        if (p) {
          clearTimeout(p.timer);
          this.pending.delete(m.id);
          m.error ? p.reject(Error(m.error)) : p.resolve(m);
        }
      } catch {}
    });
    const fail = () => {
      if (this.child === c) {
        this.child = undefined;
        this.device = "unloaded";
        for (const p of this.pending.values()) {
          clearTimeout(p.timer);
          p.reject(Error("Optional Python worker exited"));
        }
        this.pending.clear();
      }
    };
    c.stdin!.on("error", fail);
    c.on("exit", fail);
    c.on("error", fail);
    const r = await this.request({ type: "load", ...options });
    this.device = r.device;
    return r;
  }
  run(pcm: Float32Array, prompt = "") {
    return this.queue.run(async () => {
      if (!this.child) await this.load(this.options, true);
      const generation = this.generation;
      try {
        return await this.request({
          type: "run",
          pcm: Array.from(pcm),
          prompt,
        });
      } catch (e) {
        if (
          generation !== this.generation ||
          (this.child && this.options.opts.device === "cpu")
        )
          throw e;
        await this.load(
          {
            ...this.options,
            opts: { ...this.options.opts, device: "cpu", compute_type: "int8" },
          },
          true,
        );
        return this.request({ type: "run", pcm: Array.from(pcm), prompt });
      }
    });
  }
  stop(preserveQueue = false) {
    if (!preserveQueue) {
      this.generation++;
      this.queue.cancel();
    }
    killTree(this.child);
    this.child = undefined;
    for (const p of this.pending.values()) {
      clearTimeout(p.timer);
      p.reject(Error("Cancelled"));
    }
    this.pending.clear();
    this.device = "unloaded";
  }
}
