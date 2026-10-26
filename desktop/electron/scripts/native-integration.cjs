// Run with Node or packaged Electron in ELECTRON_RUN_AS_NODE mode; no UI/hooks/mic.
const path = require("node:path"),
  os = require("node:os"),
  fs = require("node:fs"),
  assert = require("node:assert/strict");
const root = path.resolve(process.argv[2] || ".");
const model = process.argv[3],
  wav = process.argv[4],
  expectedPhrase = process.argv[5] || "Quilter";
const { Engine } = require(path.join(root, "dist/engine.js"));
const { readWav, hash, Timings } = require(path.join(root, "dist/core.js"));
const { catalog } = require(path.join(root, "dist/models.js"));
(async () => {
  assert.equal(
    await hash(model),
    catalog.find((m) => m.id === "parakeet-tdt-0.6b-v3").gguf.sha256,
  );
  const e = new Engine(
    path.join(root, "resources/native"),
    new Timings(
      fs.mkdtempSync(path.join(os.tmpdir(), "speechify-electron-integration-")),
    ),
  );
  const pcm = readWav(wav);
  try {
    await e.load({ file: model, device: "cuda", threads: 4, language: "en" });
    assert.equal(e.device.toLowerCase(), "cpu");
    const a = await e.run(pcm);
    assert.ok(
      a.text.toLowerCase().includes(expectedPhrase.toLowerCase()),
      "Fixture must contain expected phrase: " + expectedPhrase,
    );
    const b = await e.run(pcm);
    assert.equal(a.text, b.text);
    const run = e.run(pcm);
    const queued = e.run(pcm);
    await new Promise((r) => setTimeout(r, 25));
    e.child.kill("SIGKILL");
    const recovered = await run;
    assert.equal(recovered.text, a.text);
    assert.equal((await queued).text, a.text);
    const cancelled = e.run(pcm);
    await new Promise((r) => setTimeout(r, 25));
    e.stop();
    await assert.rejects(cancelled, /Cancelled/);
    console.log(
      JSON.stringify({
        modelSha256: await hash(model),
        fixtureSha256: await hash(wav),
        expectedPhrase,
        checks: [
          "packaged native module load",
          "CUDA-unavailable CPU fallback",
          "persistent two-run transcript",
          "in-flight crash recovery retries on CPU",
          "queued audio survives crash recovery",
          "in-flight explicit cancellation",
        ],
        text: a.text,
        firstMs: a.ms,
        warmMs: b.ms,
        recoveryMs: recovered.ms,
      }),
    );
  } finally {
    e.stop();
  }
})().catch((e) => {
  console.error(e);
  process.exitCode = 1;
});
