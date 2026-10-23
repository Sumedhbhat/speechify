"""Compare how fast each engine runs a model on this machine.

    speechify bench                                  Parakeet v3 and Whisper turbo
    speechify bench --model parakeet-tdt-0.6b-v3 --runs 7 --cpu
    speechify bench --clips ~/bench/clips            a folder with clips.json + WAVs

For every model it runs the backend Speechify uses here (on an NVIDIA GPU:
ONNX Runtime or CTranslate2 on CUDA) and transcribe.cpp on this machine's
GPU (CUDA, Vulkan or Metal), through the real worker process, and prints a
table of load time, median transcription time per clip and word error rate.
With --cpu, both run on the CPU as well. Missing environments and models are
installed first, like `speechify models install`.

Medians, because other programs (and a GPU's clocks) make single timings
noisy. Results go to --out as JSON too.
"""

from __future__ import annotations

import json
import statistics
import time
from dataclasses import dataclass
from pathlib import Path

import numpy as np

DEFAULT_MODELS = ("parakeet-tdt-0.6b-v3", "whisper-large-v3-turbo")


@dataclass
class Clip:
    name: str
    audio: np.ndarray
    text: str

    @property
    def seconds(self) -> float:
        return self.audio.size / 16_000


def builtin_clips() -> list[Clip]:
    """The self-test's LibriSpeech clip (10 s), and three of it in a row (31 s)."""
    from .audio import load_wav
    from .selftest import REFERENCE, SAMPLE

    audio = load_wav(SAMPLE)
    gap = np.zeros(8_000, dtype=np.float32)
    long = np.concatenate([audio, gap, audio, gap, audio])
    return [Clip("10s", audio, REFERENCE), Clip("31s", long, " ".join([REFERENCE] * 3))]


def folder_clips(folder: Path) -> list[Clip]:
    """clips.json ([{"name", "text"}, ...]) and <name>.wav, as in the Handy benchmark."""
    from .audio import load_wav

    entries = json.loads((folder / "clips.json").read_text(encoding="utf-8"))
    return [Clip(e["name"], load_wav(folder / f"{e['name']}.wav"), e["text"]) for e in entries]


def candidates(spec, device: str, cpu: bool):
    """(label, spec, device) for every engine worth timing for `spec` here."""
    from .engines.catalog import as_ggml, placed

    out = [(f"{placed(spec, device).backend} on {device}", placed(spec, device), device)]
    if spec.gguf is not None and device != "cpu":
        out.append((f"ggml on {device}", as_ggml(spec), device))
    if cpu and device != "cpu":
        out.append((f"{spec.backend} on cpu", spec, "cpu"))
        if spec.gguf is not None:
            out.append(("ggml on cpu", as_ggml(spec), "cpu"))
    seen, unique = set(), []
    for label, s, d in out:
        if (s.backend, d) not in seen:
            seen.add((s.backend, d))
            unique.append((label, s, d))
    return unique


def run_one(spec, opts: dict, clips: list[Clip], runs: int, log=print) -> dict:
    from .engines import host
    from .metrics import wer

    host.install(spec, opts, log)
    engine = host.EngineHost()
    t0 = time.perf_counter()
    info = engine.start(spec, opts)
    result = {"backend": engine.spec.backend, "device": info.get("device"),
              "load_s": round(time.perf_counter() - t0, 2), "clips": []}
    try:
        for clip in clips:
            walls, text = [], ""
            for _ in range(runs):
                t = time.perf_counter()
                text, _ = engine.transcribe(clip.audio, None)
                walls.append(time.perf_counter() - t)
            result["clips"].append({
                "name": clip.name, "seconds": round(clip.seconds, 2),
                "median_s": round(statistics.median(walls), 3), "runs_s": [round(w, 3) for w in walls],
                "wer": round(wer([clip.text], [text]), 4), "text": text,
            })
            log(f"  {clip.name}: median {statistics.median(walls):.2f}s over {runs} runs, "
                f"WER {wer([clip.text], [text]):.1%}")
        result["device"] = engine.info.get("device")  # in case it fell back to the CPU
    finally:
        engine.stop()
    return result


def main(models: list[str] | None, runs: int, cpu: bool, clips_dir: str | None, out: str | None) -> int:
    from .config import Config, data_dir
    from .engines.catalog import resolve

    cfg = Config.load()
    device = cfg.resolved_device()
    clips = folder_clips(Path(clips_dir).expanduser()) if clips_dir else builtin_clips()
    results = []
    for name in models or DEFAULT_MODELS:
        spec = resolve(name, data_dir())
        for label, s, d in candidates(spec, device, cpu):
            print(f"--- {spec.name}: {label}", flush=True)
            opts = {**cfg.engine_opts(), "device": d,
                    "compute_type": cfg.compute_type if d == "cuda" else "int8"}
            try:
                r = run_one(s, opts, clips, runs)
            except Exception as exc:  # one engine failing shouldn't hide the others' numbers
                print(f"  failed: {exc}")
                r = {"backend": s.backend, "device": d, "error": str(exc), "clips": []}
            results.append({"model": spec.id, "engine": label, **r})

    lines = ["| Model | Engine | Device in use | Load | " + " | ".join(
        f"{c.name} ({c.seconds:.0f}s audio)" for c in clips) + " | WER |",
        "|" + " --- |" * (5 + len(clips))]
    for r in results:
        if "error" in r:
            lines.append(f"| {r['model']} | {r['engine']} | failed: {r['error'][:80]} |"
                         + " |" * (2 + len(clips)))
            continue
        times = " | ".join(f"{c['median_s']:.2f} s" for c in r["clips"])
        mean_wer = statistics.mean(c["wer"] for c in r["clips"]) if r["clips"] else 0
        lines.append(f"| {r['model']} | {r['engine']} | {r['device']} | {r['load_s']:.1f} s | {times} "
                     f"| {mean_wer:.1%} |")
    table = "\n".join(lines)
    print(f"\nMedian of {runs} runs per clip; load includes the warm-up run.\n\n{table}")
    path = Path(out) if out else data_dir() / "logs" / f"bench-{time.strftime('%Y%m%d-%H%M%S')}.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({"device_setting": cfg.device, "device": device, "runs": runs,
                                "results": results, "table": table}, indent=1), encoding="utf-8")
    print(f"\nSaved to {path}")
    return 0 if all("error" not in r for r in results) else 1
