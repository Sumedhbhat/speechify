#!/usr/bin/env python3
"""Fixture-only verification of an actual packaged binary. No desktop startup."""

import argparse
import pathlib
import os
import subprocess
import json
import hashlib
import statistics
import time
import shutil
import math
import tomllib

p = argparse.ArgumentParser()
p.add_argument("binary", type=pathlib.Path)
p.add_argument("wav", type=pathlib.Path)
p.add_argument("model", type=pathlib.Path)
p.add_argument("--output", type=pathlib.Path, required=True)
p.add_argument("--parent", type=pathlib.Path, required=True)
args = p.parse_args()
root = args.parent.resolve() / "speechify-tauri-candidate"
(root / "models").mkdir(parents=True, exist_ok=True)
dest = root / "models/moonshine-base.gguf"
if not dest.exists():
    shutil.copy2(args.model, dest)


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        while b := f.read(1024 * 1024):
            h.update(b)
    return h.hexdigest()


assert digest(dest) == "7f0027dfd857d310b63a85ef57cadf183da712cc374f85a648f8bc18aaa2efc8"
log = root / "logs/timings.jsonl"
if log.exists():
    log.unlink()
started = time.perf_counter()
run = subprocess.run(
    [str(args.binary.resolve()), "fixture", "moonshine-base", str(args.wav.resolve()), "20", "cpu"],
    env={**os.environ, "SPEECHIFY_CANDIDATE_PARENT": str(args.parent.resolve())},
    capture_output=True,
    text=True,
    timeout=300,
    check=True,
)
elapsed = (time.perf_counter() - started) * 1000
rows = [json.loads(line) for line in run.stdout.splitlines()]
assert len(rows) == 20
assert all(
    r["result"]["device"] == "cpu"
    and "country" in r["result"]["text"].lower()
    and "americans" in r["result"]["text"].lower()
    for r in rows
)
events = [json.loads(line) for line in log.read_text().splitlines()]
ready = [e for e in events if e["event"] == "model_ready"]
assert len(ready) == 1, "Worker must remain loaded"
times = [r["result"]["inference_ms"] for r in rows]
effective = tomllib.loads((root / "config.toml").read_text())
report = {
    "platform": os.uname().sysname if hasattr(os, "uname") else "Windows",
    "samples": len(rows),
    "model_id": "moonshine-base",
    "model_sha256": digest(dest),
    "model_bytes": dest.stat().st_size,
    "wav_sha256": digest(args.wav),
    "wav_bytes": args.wav.stat().st_size,
    "binary_sha256": digest(args.binary),
    "device": "cpu",
    "threads": effective["onnx_threads"],
    "language": effective["language"],
    "persistent_model_loads": len(ready),
    "native_load_ms": ready[0]["values"]["load_ms"],
    "process_wall_ms": elapsed,
    "inference_median_ms": statistics.median(times),
    "inference_p95_ms": sorted(times)[math.ceil(len(times) * 0.95) - 1],
    "inference_ms": times,
    "transcripts": rows,
    "notes": "Fresh process, warm filesystem cache; no microphone/paste/tray. Shared development host, not an exclusive benchmark.",
}
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(report, indent=2) + "\n")
print(
    json.dumps(
        {k: v for k, v in report.items() if k not in ["transcripts", "inference_ms"]}, indent=2
    )
)
