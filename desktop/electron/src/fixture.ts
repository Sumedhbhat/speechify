import path from "node:path";
import os from "node:os";
import fs from "node:fs";
import { Engine } from "./engine";
import { hash, readWav, Timings } from "./core";
import { catalog } from "./models";
async function main() {
  const [
    id,
    file,
    wav,
    device = "cpu",
    repeats = "1",
    root = path.join(os.tmpdir(), "speechify-electron-candidate-fixture"),
  ] = process.argv.slice(2);
  const model = catalog.find((m) => m.id === id);
  if (!model?.gguf)
    throw Error(
      "Usage: fixture MODEL_ID MODEL.gguf PCM16_16k.wav [cpu|vulkan|metal|cuda] [repeats] [candidate-output-dir]",
    );
  const digest = await hash(file);
  if (digest !== model.gguf.sha256) throw Error("Model checksum mismatch");
  const engine = new Engine(
    path.resolve(__dirname, "../resources/native"),
    new Timings(root),
  );
  const pcm = readWav(wav);
  try {
    const ready = await engine.load({
      file,
      device,
      threads: 4,
      language: "en",
    });
    const runs = [];
    for (let i = 0; i < Number(repeats); i++) runs.push(await engine.run(pcm));
    console.log(
      JSON.stringify(
        {
          model: id,
          sha256: digest,
          fixture: path.resolve(wav),
          fixtureSha256: await hash(wav),
          platform: process.platform,
          arch: process.arch,
          cpu: os.cpus()[0].model,
          loadMs: ready.ms,
          runs,
        },
        null,
        2,
      ),
    );
  } finally {
    engine.stop();
  }
}
main().catch((e) => {
  console.error(String(e));
  process.exitCode = 1;
});
