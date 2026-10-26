import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import vm from "node:vm";
import { createHash } from "node:crypto";
import { judge } from "../learning";
import { Store } from "../store";
import { Models, optional } from "../models";
import { pcmWav, splitAudio } from "../core";
const temp = () =>
  fs.mkdtempSync(path.join(os.tmpdir(), "speechify-continuation-"));

test("latest private audio is memory-only until correction, then exportable", () => {
  const store = new Store(temp()),
    wav = pcmWav(Buffer.alloc(3200));
  const id = store.add(wav, "wrong", "wrong", false);
  assert.equal(store.get(id).audio, null);
  assert.deepEqual(fs.readdirSync(store.audio), []);
  store.correct(id, "right");
  assert.deepEqual(fs.readFileSync(store.audioFile(id)), wav);
  assert.equal(store.export(temp()), 1);
  store.close();
});
test("private retention holds only latest clip and clears on delete, close and TTL", () => {
  const store = new Store(temp()),
    wav = pcmWav(Buffer.alloc(32));
  const first = store.add(wav, "one", "one", false);
  const second = store.add(wav, "two", "two", false);
  store.correct(first, "earlier");
  assert.equal(store.get(first).audio, null);
  (store as any).latest.expires = Date.now() - 1;
  store.correct(second, "expired");
  assert.equal(store.get(second).audio, null);
  const third = store.add(wav, "three", "three", false);
  const retained = (store as any).latest.wav as Buffer;
  store.remove(third);
  assert.ok(retained.every((n) => n === 0));
  store.add(wav, "four", "four", false);
  store.close();
  assert.equal((store as any).latest, undefined);
});
test("saving unchanged private transcript does not persist its audio", () => {
  const store = new Store(temp());
  const id = store.add(pcmWav(Buffer.alloc(20)), "same", "same", false);
  store.correct(id, "same");
  assert.equal(store.get(id).audio, null);
  store.close();
});
test("native long-audio splitting stays within limit and preserves every sample", () => {
  const audio = Float32Array.from({ length: 17003 }, (_, i) => Math.sin(i));
  const pieces = splitAudio(audio, 4000);
  assert.ok(pieces.length > 4);
  assert.ok(pieces.every((p) => p.length > 0 && p.length <= 4000));
  assert.deepEqual(
    pieces.flatMap((p) => Array.from(p)),
    Array.from(audio),
  );
  assert.equal(splitAudio(audio, 0)[0], audio);
  assert.equal(splitAudio(audio, 20000)[0], audio);
});
test("optional install pins URL, validates SHA and Git blobs, writes atomic receipt, detects tampering", async () => {
  const before = globalThis.fetch,
    id = "fixture-optional",
    root = temp();
  const parts = [Buffer.from("weights"), Buffer.from('{"fixture":true}')];
  optional[id] = {
    repo: "fixture/model",
    revision: "a".repeat(40),
    files: [
      {
        file: "model.bin",
        size: parts[0].length,
        sha256: createHash("sha256").update(parts[0]).digest("hex"),
      },
      {
        file: "config.json",
        size: parts[1].length,
        gitBlob: createHash("sha1")
          .update(`blob ${parts[1].length}\0`)
          .update(parts[1])
          .digest("hex"),
      },
    ],
  };
  let index = 0;
  globalThis.fetch = async (url) => {
    assert.ok(String(url).includes("/resolve/" + "a".repeat(40) + "/"));
    return new Response(parts[index++]);
  };
  const stages: string[] = [];
  const models = new Models(root, () => {
    const p = models.progress.get(id);
    if (p) stages.push(p.stage);
  });
  try {
    await models.install(id);
    assert.equal(models.optionalInstalled(id), true);
    const dir = await models.optionalFile(id);
    assert.equal(
      JSON.parse(fs.readFileSync(path.join(dir, "receipt.json"), "utf8"))
        .revision,
      "a".repeat(40),
    );
    assert.ok(stages.includes("downloading") && stages.includes("verifying"));
    assert.ok(fs.readdirSync(root).every((n) => !n.includes("partial")));
    fs.writeFileSync(path.join(dir, "model.bin"), "changed");
    await assert.rejects(models.optionalFile(id), /checksum/);
  } finally {
    globalThis.fetch = before;
    delete optional[id];
    fs.rmSync(root, { recursive: true, force: true });
  }
});
test("optional install failure/cancellation cleans staging and never advertises installed", async () => {
  const before = globalThis.fetch,
    id = "fixture-cancel",
    root = temp();
  optional[id] = {
    repo: "fixture/model",
    revision: "b".repeat(40),
    files: [{ file: "model.bin", size: 3, sha256: "0".repeat(64) }],
  };
  const models = new Models(root, () => {});
  try {
    globalThis.fetch = async () => new Response(new Uint8Array([1, 2, 3]));
    await assert.rejects(models.install(id), /checksum/);
    assert.equal(models.optionalInstalled(id), false);
    assert.deepEqual(fs.readdirSync(root), []);
    globalThis.fetch = async (_url, opts) =>
      new Promise((_resolve, reject) =>
        opts?.signal?.addEventListener("abort", () => reject(Error("aborted"))),
      );
    const task = models.install(id);
    models.cancel(id);
    await assert.rejects(task, /aborted/);
    assert.equal(models.progress.get(id).stage, "cancelled");
    assert.deepEqual(fs.readdirSync(root), []);
  } finally {
    globalThis.fetch = before;
    delete optional[id];
    fs.rmSync(root, { recursive: true, force: true });
  }
});

// Execute the actual compiled capture script against fake browser APIs, never a device.
function captureFixture() {
  let listener: (m: any) => void = () => {},
    stopped = 0;
  let openResolve: (s: any) => void = () => {},
    renderResolve: (s: any) => void = () => {};
  const events: any[] = [],
    contexts: any[] = [];
  const track = {
    stop() {
      stopped++;
    },
    onended: null,
  };
  const stream = { getTracks: () => [track], getAudioTracks: () => [track] };
  class Context {
    sampleRate = 48000;
    state = "running";
    destination = {};
    processor: any;
    constructor() {
      contexts.push(this);
    }
    createMediaStreamSource() {
      return { connect() {}, disconnect() {} };
    }
    createScriptProcessor() {
      return (this.processor = {
        connect() {},
        disconnect() {},
        onaudioprocess: null,
      });
    }
    async resume() {}
    async close() {
      this.state = "closed";
    }
  }
  class Offline {
    createBuffer() {
      return { copyToChannel() {} };
    }
    createBufferSource() {
      return { connect() {}, start() {} };
    }
    startRendering() {
      return new Promise((r) => {
        renderResolve = r;
      });
    }
  }
  const sandbox = vm.createContext({
    capture: {
      on: (f: any) => {
        listener = f;
      },
      send: (e: string, d: any) => events.push([e, d]),
    },
    navigator: {
      mediaDevices: {
        enumerateDevices: async () => [],
        addEventListener() {},
        getUserMedia: () =>
          new Promise((r) => {
            openResolve = r;
          }),
      },
    },
    AudioContext: Context,
    OfflineAudioContext: Offline,
    Float32Array,
    console,
  });
  vm.runInContext(
    fs.readFileSync(path.join(__dirname, "../capture.js"), "utf8"),
    sandbox,
  );
  return {
    send: (m: any) => listener(m),
    opened: () => openResolve(stream),
    rendered: () =>
      renderResolve({ getChannelData: () => new Float32Array(160) }),
    events,
    contexts,
    stopped: () => stopped,
  };
}
const flush = () => new Promise((r) => setImmediate(r));
test("capture cancellation while opening closes late microphone without submitting audio", async () => {
  const f = captureFixture();
  f.send({ type: "start", id: 1 });
  await flush();
  f.send({ type: "stop", accept: false });
  f.opened();
  await flush();
  assert.equal(f.stopped(), 1);
  assert.equal(f.events.filter((e) => e[0] === "audio").length, 0);
});
test("capture cancellation during resampling suppresses stale audio and closes idle mic first", async () => {
  const f = captureFixture();
  f.send({ type: "start", id: 1 });
  await flush();
  f.opened();
  await flush();
  f.contexts[0].processor.onaudioprocess({
    inputBuffer: { getChannelData: () => new Float32Array(4096) },
  });
  f.send({ type: "stop", accept: true });
  await flush();
  assert.equal(f.stopped(), 1);
  f.send({ type: "stop", accept: false });
  f.rendered();
  await flush();
  assert.equal(f.events.filter((e) => e[0] === "audio").length, 0);
});
test("capture normal release submits matching recording ID after resampling", async () => {
  const f = captureFixture();
  f.send({ type: "start", id: 7 });
  await flush();
  f.opened();
  await flush();
  f.contexts[0].processor.onaudioprocess({
    inputBuffer: { getChannelData: () => new Float32Array(4096) },
  });
  f.send({ type: "stop", accept: true });
  await flush();
  f.rendered();
  await flush();
  const audio = f.events.filter((e) => e[0] === "audio");
  assert.equal(audio.length, 1);
  assert.equal(audio[0][1].id, 7);
  assert.equal(f.stopped(), 1);
});

test("non-finite teacher confidence is rejected rather than stored as a label", () => {
  const pcm = new Float32Array(48000).fill(0.2);
  for (const confidence of [NaN, Infinity, -Infinity])
    assert.deepEqual(judge("hello there friend", pcm, confidence), {
      confidence: 0,
      note: "invalid confidence",
    });
});
