import fs from "node:fs";
import path from "node:path";
import catalog from "./catalog.json";
import optionalPins from "./optional-pins.json";
import { createHash, randomUUID } from "node:crypto";
export const optional: Record<
  string,
  {
    repo: string;
    revision: string;
    files: { file: string; size: number; sha256?: string; gitBlob?: string }[];
  }
> = optionalPins;
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
          !!this.reuse[m.id] ||
          this.optionalInstalled(m.id),
        progress: this.progress.get(m.id),
        support: m.gguf
          ? "Native transcribe.cpp"
          : "Optional Python inference worker",
      })),
    ];
  }
  private optionalRoot(id: string) {
    if (!optional[id])
      throw Error("No pinned optional artifact for this model");
    return owned(this.root, id + "-" + optional[id].revision);
  }
  optionalInstalled(id: string) {
    if (!optional[id]) return false;
    try {
      const dir = this.optionalRoot(id);
      const receipt = JSON.parse(
        fs.readFileSync(path.join(dir, "receipt.json"), "utf8"),
      );
      return (
        receipt.revision === optional[id].revision &&
        receipt.repo === optional[id].repo &&
        optional[id].files.every(
          (f) => fs.statSync(owned(dir, f.file)).size === f.size,
        )
      );
    } catch {
      return false;
    }
  }
  private async verifyOptional(dir: string, id: string, signal?: AbortSignal) {
    for (const f of optional[id].files) {
      signal?.throwIfAborted();
      const file = owned(dir, f.file);
      if (fs.statSync(file).size !== f.size)
        throw Error("Optional model size mismatch: " + f.file);
      const h = createHash(f.sha256 ? "sha256" : "sha1");
      if (!f.sha256) h.update("blob " + f.size + "\0");
      for await (const b of fs.createReadStream(file)) {
        signal?.throwIfAborted();
        h.update(b);
      }
      if (h.digest("hex") !== (f.sha256 || f.gitBlob))
        throw Error("Optional model checksum mismatch: " + f.file);
    }
  }
  async optionalFile(id: string) {
    if (!this.optionalInstalled(id))
      throw Error("Install the pinned optional model first");
    const dir = this.optionalRoot(id);
    await this.verifyOptional(dir, id);
    return dir;
  }
  private async installOptional(id: string) {
    const dest = this.optionalRoot(id),
      pin = optional[id];
    if (this.optionalInstalled(id)) {
      await this.optionalFile(id);
      return;
    }
    // Unique candidate-owned staging prevents cancelled retries from racing old writers.
    const tmp = dest + ".partial-" + randomUUID(),
      c = new AbortController();
    this.jobs.set(id, c);
    let bytes = 0,
      last = 0;
    const total = pin.files.reduce((n, f) => n + f.size, 0);
    const progress = (stage: string, error?: string) => {
      this.progress.set(id, { stage, bytes, total, error });
      this.update();
    };
    try {
      fs.mkdirSync(tmp, { recursive: true });
      progress("downloading");
      for (const f of pin.files) {
        c.signal.throwIfAborted();
        const res = await fetch(
          `https://huggingface.co/${pin.repo}/resolve/${pin.revision}/${f.file}`,
          { signal: c.signal },
        );
        if (!res.ok || !res.body) throw Error("Download HTTP " + res.status);
        const file = owned(tmp, f.file);
        fs.mkdirSync(path.dirname(file), { recursive: true });
        const fd = fs.openSync(file, "wx", 0o600);
        let count = 0;
        try {
          for await (const chunk of res.body as any) {
            c.signal.throwIfAborted();
            count += chunk.length;
            if (count > f.size)
              throw Error("Optional artifact exceeds pinned size");
            let offset = 0;
            while (offset < chunk.length)
              offset += fs.writeSync(fd, chunk, offset, chunk.length - offset);
            bytes += chunk.length;
            if (Date.now() - last >= 200) {
              progress("downloading");
              last = Date.now();
            }
          }
        } finally {
          fs.closeSync(fd);
        }
        if (count !== f.size) throw Error("Optional artifact truncated");
      }
      progress("verifying");
      await this.verifyOptional(tmp, id, c.signal);
      c.signal.throwIfAborted();
      atomic(
        path.join(tmp, "receipt.json"),
        JSON.stringify({ ...pin, installedAt: new Date().toISOString() }),
      );
      // A failed/incomplete destination is not accepted as an install.
      if (fs.existsSync(dest))
        throw Error(
          "Incomplete optional installation exists; remove candidate folder " +
            dest +
            " then retry",
        );
      fs.renameSync(tmp, dest);
      this.progress.delete(id);
    } catch (e) {
      progress(c.signal.aborted ? "cancelled" : "error", String(e));
      throw e;
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true });
      this.jobs.delete(id);
      this.update();
    }
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
    if (!m?.gguf) return this.installOptional(id);
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
          let offset = 0;
          while (offset < chunk.length)
            offset += fs.writeSync(fd, chunk, offset, chunk.length - offset);
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
