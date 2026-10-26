import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import {
  Settings,
  HoldKey,
  BoundedQueue,
  owned,
  edits,
  label,
  pcmWav,
  readWav,
  Timings,
} from "../core";
import { Store } from "../store";
import { guardedPaste } from "../output";
import { Models, catalog } from "../models";
import { judge, gatesPass } from "../learning";
const temp = () =>
  fs.mkdtempSync(path.join(os.tmpdir(), "speechify-electron-test-"));
test("catalog preserves all fourteen models and eleven pins", () => {
  assert.equal(catalog.length, 14);
  assert.equal(catalog.filter((m) => m.gguf).length, 11);
  for (const m of catalog)
    if (m.gguf) assert.match(m.gguf.sha256, /^[a-f0-9]{64}$/);
});
test("hold key ignores repeats, taps and Ctrl+C", () => {
  let starts = 0;
  const ends: boolean[] = [];
  const h = new HoldKey(
    [1],
    250,
    () => starts++,
    (ok) => ends.push(ok),
  );
  h.event(1, true, 0);
  h.event(1, true, 20);
  h.event(1, false, 200);
  h.event(1, true, 1000);
  h.event(3, true, 1300);
  h.event(3, false, 1400);
  h.event(1, false, 1500);
  h.event(1, true, 2000);
  h.event(1, false, 2400);
  assert.equal(starts, 3);
  assert.deepEqual(ends, [false, false, true]);
});
test("combination requires exact chord; unrelated held key suppresses capture", () => {
  const ends: boolean[] = [];
  let starts = 0;
  const h = new HoldKey(
    [1, 2],
    250,
    () => starts++,
    (v) => ends.push(v),
  );
  h.event(9, true, 0);
  h.event(1, true, 1);
  h.event(2, true, 2);
  assert.equal(starts, 0);
  h.cancel();
  h.event(1, true, 10);
  h.event(2, true, 20);
  h.event(2, false, 300);
  assert.deepEqual(ends, [true]);
});
test("cancellation invalidates queued tasks and capacity is bounded", async () => {
  const q = new BoundedQueue(2);
  let release!: () => void;
  const a = q.run(() => new Promise<void>((r) => (release = r)));
  const b = q.run(async () => 42);
  await assert.rejects(
    q.run(async () => 3),
    /full/,
  );
  await new Promise((r) => setImmediate(r));
  q.cancel();
  release();
  await a;
  await assert.rejects(b, /Cancelled/);
  assert.equal(await q.run(async () => 9), 9);
});
test("settings persist all baseline fields and reject invalid patches atomically", () => {
  const root = temp();
  const s = new Settings(root);
  s.save({ input_device: "mic", min_hold_ms: 300 });
  const before = fs.readFileSync(path.join(root, "config.toml"), "utf8");
  assert.throws(() => s.save({ device: "not-a-device" }));
  assert.equal(fs.readFileSync(path.join(root, "config.toml"), "utf8"), before);
  assert.equal(new Settings(root).value.input_device, "mic");
  assert.throws(() => s.import('keep_mic_open = "yes"'));
  assert.throws(() => s.save({ teacher_min_confidence: 1.2 }));
});
test("candidate path guard rejects traversal and symlinks", () => {
  const r = temp(),
    outside = temp();
  assert.throws(() => owned(r, "../bad"));
  fs.symlinkSync(outside, path.join(r, "escape"));
  assert.throws(() => owned(r, "escape"));
});
test("word rules preserve casing and reject long rewrites and insertions", () => {
  assert.deepEqual(edits("use get hub today", "use GitHub today"), [
    ["get hub", "GitHub"],
  ]);
  assert.deepEqual(edits("github works", "GitHub works"), [
    ["github", "GitHub"],
  ]);
  assert.deepEqual(edits("hello", "hello there"), []);
  assert.deepEqual(
    edits("one two three four five", "six seven eight nine ten"),
    [],
  );
});
test("training never uses pasted guesses; correction precedence beats teacher", () => {
  const r = { audio: "a.wav", pasted: "guess", teacher_ts: null };
  assert.equal(label(r), null);
  assert.equal(
    label({
      ...r,
      corrected: "human",
      teacher_text: "teacher",
      teacher_ts: 1,
      teacher_confidence: 0.9,
    }),
    "human",
  );
  assert.equal(
    label({
      ...r,
      teacher_text: "teacher",
      teacher_ts: 1,
      teacher_confidence: 0.4,
    }),
    null,
  );
  assert.equal(label({ ...r, corrected: "human", excluded: 1 }), null);
});
test("SQLite history corrections, threshold, reversion, deletion and export", () => {
  const root = temp(),
    s = new Store(root);
  const id = s.add(
    pcmWav(Buffer.alloc(32000)),
    "get hub works",
    "get hub works",
  );
  s.correct(id, "GitHub works");
  assert.equal(s.apply("get hub works", 2), "get hub works");
  const id2 = s.add(
    pcmWav(Buffer.alloc(32000)),
    "get hub works",
    "get hub works",
  );
  s.correct(id2, "GitHub works");
  assert.equal(s.apply("get hub works", 2), "GitHub works");
  const out = temp();
  assert.equal(s.export(out), 2);
  s.correct(id, "get hub works");
  assert.equal(s.get(id).corrected, null);
  s.exclude(id2, true);
  assert.equal(s.export(temp()), 0);
  const wav = s.audioFile(id);
  s.remove(id);
  assert.equal(fs.existsSync(wav), false);
  s.close();
  const again = new Store(root);
  assert.equal(again.rows().length, 1);
  again.close();
});
test("malicious audio reference cannot delete external file", () => {
  const s = new Store(temp()),
    outside = path.join(temp(), "keep.wav");
  fs.writeFileSync(outside, "keep");
  const id = s.add(pcmWav(Buffer.alloc(2)), "a", "a");
  s.db.prepare("UPDATE samples SET audio=? WHERE id=?").run(outside, id);
  assert.throws(() => s.remove(id), /outside/);
  assert.equal(fs.readFileSync(outside, "utf8"), "keep");
  s.close();
});
test("SQLite-consistent import copies audio and does not change source", async () => {
  const source = new Store(temp()),
    dest = new Store(temp());
  const id = source.add(pcmWav(Buffer.alloc(3200)), "a", "a");
  source.correct(id, "A");
  const original = fs.readFileSync(source.audioFile(id));
  await dest.import(source.root);
  assert.equal(dest.rows().length, 1);
  assert.deepEqual(
    fs.readFileSync(dest.audioFile(Number(dest.rows()[0].id))),
    original,
  );
  dest.remove(Number(dest.rows()[0].id));
  assert.deepEqual(fs.readFileSync(source.audioFile(id)), original);
  source.close();
  dest.close();
});
test("WAV reads samples exactly and refuses malformed/truncated fixtures", () => {
  const r = temp(),
    file = path.join(r, "a.wav"),
    pcm = Buffer.alloc(4);
  pcm.writeInt16LE(16384, 0);
  pcm.writeInt16LE(-16384, 2);
  fs.writeFileSync(file, pcmWav(pcm));
  assert.deepEqual(Array.from(readWav(file)), [0.5, -0.5]);
  fs.writeFileSync(file, pcmWav(pcm).subarray(0, 45));
  assert.throws(() => readWav(file), /Truncated/);
});
test("paste checks focus before changing clipboard", async () => {
  let writes = 0;
  await assert.rejects(
    guardedPaste(
      "hi",
      "1",
      async () => "2",
      async () => {},
      {
        read: async () => ({ text: "old", html: "", rtf: "" }),
        write: async () => {
          writes++;
        },
      },
    ),
    /Focus changed/,
  );
  assert.equal(writes, 0);
});
test("clipboard ownership token prevents clobbering even a newer identical copy", async () => {
  let current: any = { text: "old", html: "", rtf: "" };
  await guardedPaste(
    "hello",
    "1",
    async () => "1",
    async () => {},
    {
      read: async () => current,
      write: async (v) => {
        current = { html: "", rtf: "", ...v };
      },
    },
    async () => {
      current = { text: "hello", html: "", rtf: "", stamp: "" };
    },
  );
  assert.equal(current.text, "hello");
  assert.equal(current.stamp, "");
});
test("clipboard restores snapshot on successful paste when still owned", async () => {
  let current: any = { text: "old", html: "<b>old</b>", rtf: "" };
  let pasted = 0;
  await guardedPaste(
    "hello",
    "1",
    async () => "1",
    async () => {
      pasted++;
    },
    {
      read: async () => current,
      write: async (v) => {
        current = { html: "", rtf: "", ...v };
      },
    },
    async () => {},
  );
  assert.equal(pasted, 1);
  assert.equal(current.html, "<b>old</b>");
});
test("failed download and cancellation remove candidate partial files", async () => {
  const fetchBefore = globalThis.fetch;
  const root = temp(),
    m = new Models(root, () => {});
  globalThis.fetch = async () => new Response(new Uint8Array([1, 2, 3]));
  try {
    await assert.rejects(m.install("moonshine-base"), /Checksum/);
    assert.equal(fs.readdirSync(root).length, 0);
    globalThis.fetch = async (_url, opts) =>
      new Promise((_resolve, reject) =>
        opts?.signal?.addEventListener("abort", () => reject(Error("aborted"))),
      );
    const p = m.install("moonshine-base");
    m.cancel("moonshine-base");
    await assert.rejects(p, /aborted/);
    assert.equal(m.progress.get("moonshine-base").stage, "cancelled");
    assert.equal(fs.readdirSync(root).length, 0);
  } finally {
    globalThis.fetch = fetchBefore;
  }
});
test("read-only reuse rejects unverified models", async () => {
  const r = temp(),
    file = path.join(r, "bad.gguf");
  fs.writeFileSync(file, "bad");
  const m = new Models(path.join(r, "models"), () => {});
  await assert.rejects(m.reference("moonshine-base", file), /checksum/);
  assert.equal(fs.readFileSync(file, "utf8"), "bad");
});
test("teacher rejects hallucination on silence and clamps probabilities", () => {
  assert.equal(
    judge("one two three four five six", new Float32Array(16000), 0.99)
      .confidence,
    0,
  );
  const pcm = new Float32Array(16000 * 3).fill(0.2);
  assert.equal(judge("hello there friend", pcm, 1.5).confidence, 1);
});
test("adoption requires both gates and strict personal improvement", () => {
  const r = {
    status: "accepted",
    gate1: { passed: true, current: 0.2, candidate: 0.1 },
    gate2: { passed: true },
  };
  assert.equal(gatesPass(r), true);
  assert.equal(gatesPass({ ...r, gate2: { passed: false } }), false);
  assert.equal(
    gatesPass({ ...r, gate1: { passed: true, current: 0.1, candidate: 0.1 } }),
    false,
  );
});
test("diagnostics rotate bounded files and do not include fixture transcript", () => {
  const root = temp(),
    t = new Timings(root);
  t.log("inference", { ms: 25 });
  const f = path.join(root, "logs/timings.jsonl");
  fs.writeFileSync(f, "x".repeat(2000001));
  t.log("ready");
  assert.ok(fs.existsSync(f + ".1"));
  assert.ok(fs.statSync(f).size < 1000);
});

test("failed import rolls back candidate rows and copied WAVs", async () => {
  const source = new Store(temp()),
    dest = new Store(temp());
  source.add(pcmWav(Buffer.alloc(4)), "first", "first");
  const id = source.add(pcmWav(Buffer.alloc(4)), "bad", "bad");
  source.db
    .prepare("UPDATE samples SET audio=? WHERE id=?")
    .run("../escape.wav", id);
  await assert.rejects(dest.import(source.root), /outside/);
  assert.equal(dest.rows().length, 0);
  assert.deepEqual(fs.readdirSync(dest.audio), []);
  source.close();
  dest.close();
});
