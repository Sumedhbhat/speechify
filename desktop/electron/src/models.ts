import fs from "node:fs";
import path from "node:path";
import catalog from "./catalog.json";
import { hash, atomic, owned } from "./core";
export { catalog };
export class Models {
  private jobs = new Map<string, AbortController>();
  progress = new Map<string, any>();
  private reuse: Record<string, string> = {};
  constructor(
    public root: string,
    private update: () => void,
  ) {
    fs.mkdirSync(root, { recursive: true });
    const f = path.join(root, "references.json");
    if (fs.existsSync(f)) this.reuse = JSON.parse(fs.readFileSync(f, "utf8"));
  }
  list() {
    const trained = fs
      .readdirSync(this.root)
      .filter(
        (n) =>
          n.startsWith("whisper-ft-") &&
          fs.existsSync(path.join(this.root, n, "model.bin")),
      )
      .map((n) => ({
        id: "ft:" + n,
        name: "Your fine-tuned Whisper (" + n.slice(11) + ")",
        backend: "whisper",
        repo: path.join(this.root, n),
        languages: "as trained",
        accuracy: 0,
        speed: 4,
        vram_gb: 2,
        download_gb: 0,
        gguf: null,
        prompt: true,
        finetune: true,
        gated: false,
        notes: "Candidate-owned training output",
        installed: true,
        support: "Optional Python CTranslate2 worker",
        progress: undefined,
      }));
    return [
      ...trained,
      ...catalog.map((m) => ({
        ...m,
        installed:
          fs.existsSync(path.join(this.root, m.id + ".gguf")) ||
          !!this.reuse[m.id],
        progress: this.progress.get(m.id),
        support: m.gguf
          ? "Native transcribe.cpp"
          : "Optional Python inference worker",
      })),
    ];
  }
  async file(id: string) {
    const m = catalog.find((m) => m.id === id);
    if (!m?.gguf)
      throw Error("This model requires the optional Python backend");
    const file = this.reuse[id] || owned(this.root, id + ".gguf");
    if ((await hash(file)) !== m.gguf.sha256)
      throw Error("Model checksum mismatch");
    return file;
  }
  async reference(id: string, file: string) {
    const m = catalog.find((m) => m.id === id);
    if (!m?.gguf || (await hash(file)) !== m.gguf.sha256)
      throw Error("Read-only model checksum mismatch");
    this.reuse[id] = path.resolve(file);
    atomic(path.join(this.root, "references.json"), JSON.stringify(this.reuse));
    this.update();
  }
  cancel(id: string) {
    this.jobs.get(id)?.abort();
  }
  stop() {
    for (const c of this.jobs.values()) c.abort();
  }
  async install(id: string) {
    if (this.jobs.has(id)) throw Error("Download already running");
    const m = catalog.find((m) => m.id === id);
    if (!m?.gguf) throw Error("No pinned native artifact for this model");
    const c = new AbortController();
    this.jobs.set(id, c);
    const dest = owned(this.root, id + ".gguf"),
      tmp = dest + ".partial";
    let bytes = 0;
    try {
      this.progress.set(id, {
        stage: "downloading",
        bytes,
        total: m.gguf.size,
      });
      this.update();
      const res = await fetch(
        `https://huggingface.co/${m.gguf.repo}/resolve/${m.gguf.revision}/${m.gguf.file}`,
        { signal: c.signal },
      );
      if (!res.ok || !res.body) throw Error("Download HTTP " + res.status);
      const fd = fs.openSync(tmp, "w", 0o600);
      try {
        let last = 0;
        for await (const chunk of res.body as any) {
          c.signal.throwIfAborted();
          fs.writeSync(fd, chunk);
          bytes += chunk.length;
          if (bytes > m.gguf.size)
            throw Error("Artifact larger than pinned size");
          if (Date.now() - last > 200) {
            this.progress.set(id, {
              stage: "downloading",
              bytes,
              total: m.gguf.size,
            });
            this.update();
            last = Date.now();
          }
        }
      } finally {
        fs.closeSync(fd);
      }
      this.progress.set(id, { stage: "verifying", bytes, total: m.gguf.size });
      this.update();
      if (bytes !== m.gguf.size || (await hash(tmp)) !== m.gguf.sha256)
        throw Error("Checksum/size mismatch");
      c.signal.throwIfAborted();
      fs.renameSync(tmp, dest);
      this.progress.delete(id);
    } catch (e) {
      this.progress.set(id, {
        stage: c.signal.aborted ? "cancelled" : "error",
        error: String(e),
        bytes,
        total: m.gguf.size,
      });
      throw e;
    } finally {
      fs.rmSync(tmp, { force: true });
      this.jobs.delete(id);
      this.update();
    }
  }
}
