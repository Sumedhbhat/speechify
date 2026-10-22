"""Stands in for scripts/finetune.py in tests. FAKE_TRAIN picks the outcome:
"accept", "reject", "sleep" (runs until killed) or "crash"."""

import argparse
import json
import os
import sys
import time
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument("--report", type=Path)
args, _ = p.parse_known_args()
mode = os.environ.get("FAKE_TRAIN", "accept")
Path(os.environ["FAKE_TRAIN_ARGS"]).write_text(json.dumps(sys.argv[1:]))

if mode == "sleep":
    time.sleep(600)
if mode == "crash":
    sys.exit(3)

gate1 = {"current": 0.10, "base": 0.10, "candidate": 0.08 if mode == "accept" else 0.12,
         "held_out": 20, "source": "corrected", "passed": mode == "accept"}
gate2 = {"base": 0.03, "candidate": 0.032, "max_increase": 0.005, "clips": 50, "passed": True}
report = {"status": "accepted" if mode == "accept" else "rejected", "gate1": gate1, "gate2": gate2,
          "train": {"recordings": 80, "held_out": 20, "held_out_source": "corrected"}}
if mode == "accept":
    out = Path(os.environ["SPEECHIFY_HOME"]) / "models" / f"whisper-ft-{time.time_ns()}"
    out.mkdir(parents=True)
    (out / "model.bin").write_bytes(b"")
    report.update(model=out.as_posix(), hf=str(out.parent / ("hf-" + out.name[11:])))
args.report.write_text(json.dumps(report))
