"""Worker process: loads one model and transcribes audio sent by the app.

    SPEECHIFY_WORKER='<json config>' python -m speechify.engines.worker

Protocol over stdin/stdout (stdout is reserved for it; anything a library
prints goes to stderr instead):

    worker -> app   {"event": "ready", "device": ..., "load_s": ...}   (or {"event": "error", ...})
    app -> worker   {"samples": N, "prompt": "..."}\\n  then N float32 samples (raw bytes)
    worker -> app   {"text": "...", "ms": 123}   (or {"error": "..."})

With "download_only" in the config, the worker exits right after loading,
which is how models get downloaded at install time.
"""

from __future__ import annotations

import json
import os
import sys
import time
import traceback


def _claim_stdout():
    """Keep the real stdout for the protocol and point fd 1 at stderr.

    Some libraries print to stdout (even from C code), which would corrupt
    the protocol stream.
    """
    proto = os.fdopen(os.dup(1), "wb", buffering=0)
    os.dup2(2, 1)
    sys.stdout = sys.stderr
    return proto


def main() -> None:
    proto = _claim_stdout()
    # Passed via the environment: JSON with Windows paths doesn't survive argv quoting.
    cfg = json.loads(os.environ["SPEECHIFY_WORKER"])

    def send(msg: dict) -> None:
        proto.write((json.dumps(msg) + "\n").encode())

    try:
        import numpy as np

        from .backends import BACKEND_ENGINES

        t0 = time.perf_counter()
        backend = cfg["backend"]
        if ":" in backend:  # "module:Class", for custom engines and tests
            import importlib

            module, _, name = backend.partition(":")
            engine_cls = getattr(importlib.import_module(module), name)
        else:
            engine_cls = BACKEND_ENGINES[backend]
        engine = engine_cls(cfg["repo"], cfg["opts"])
        # The first call pays for kernel setup and caches; do it before the
        # user's first dictation rather than during it.
        try:
            engine.transcribe(np.zeros(16_000, dtype=np.float32), None)
        except Exception:
            traceback.print_exc()
        send({"event": "ready", "device": getattr(engine, "device", "?"),
              "load_s": round(time.perf_counter() - t0, 1)})
    except Exception as exc:
        traceback.print_exc()
        send({"event": "error", "error": f"{type(exc).__name__}: {exc}"})
        return
    if cfg.get("download_only"):
        return

    stdin = sys.stdin.buffer
    while True:
        line = stdin.readline()
        if not line:
            return  # app closed the pipe
        request = json.loads(line)
        raw = stdin.read(request["samples"] * 4)
        try:
            t0 = time.perf_counter()
            audio = np.frombuffer(raw, dtype=np.float32)
            text = engine.transcribe(audio, request.get("prompt")) or ""
            send({"text": text.strip(), "ms": round((time.perf_counter() - t0) * 1000)})
        except Exception as exc:
            traceback.print_exc()
            send({"error": f"{type(exc).__name__}: {exc}"})


if __name__ == "__main__":
    main()
