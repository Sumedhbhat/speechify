"""Worker process: loads one model and transcribes audio sent by the app.

    SPEECHIFY_WORKER='<json config>' python -m speechify.engines.worker

Protocol over stdin/stdout (stdout is reserved for it; anything a library
prints goes to stderr instead):

    worker -> app   {"event": "ready", "device": ..., "load_s": ...}   (or {"event": "error", ...})
    app -> worker   {"samples": N, "prompt": "...", "score": false}\\n  then N float32 samples (raw bytes)
    worker -> app   {"text": "...", "ms": 123, "logprob": null, "device": ...}   (or {"error": "..."})

"score": true asks for the mean token log-probability as well (teacher
labels); it's null when the backend doesn't expose one.

With "download_only" in the config, the worker exits right after loading,
which is how models get downloaded at install time. With "fetch_only" in the
opts as well, backends that can download without loading do that instead,
so fetching a big teacher model doesn't take over the GPU.
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


def tidy(text: str) -> str:
    """Two formatting fixes for models that sometimes miss them, applied to
    sentences of three or more words. Shorter fragments ("kubectl", "next
    line") stay exactly as the model wrote them.

    - A capital letter at the start. Parakeet and Canary punctuate, but begin
      sentences with "mister Quilter" (their way of writing "Mr."). A first
      word with a capital inside ("iPhone", "macOS") is left alone.
    - A full stop at the end, if it ends in a letter or digit. Parakeet v2,
      Distil-Whisper and Cohere sometimes leave it out.
    """
    words = text.split()
    if len(words) < 3:
        return text
    if text[0].islower() and words[0].islower():
        text = text[0].upper() + text[1:]
    if text[-1].isalnum():
        text += "."
    return text


def warmup_audio(path: str | None, seconds: float = 3.0):
    """The first `seconds` of the self-test clip (speech), or silence if it's missing.

    Speech, not silence: Whisper's VAD filter drops silence before the
    model sees it, so warming up on silence never ran the encoder and the
    first dictation paid for the warm-up instead.
    """
    import wave

    import numpy as np

    try:
        with wave.open(path, "rb") as f:
            pcm = np.frombuffer(f.readframes(int(f.getframerate() * seconds)), dtype=np.int16)
        return pcm.astype(np.float32) / 32768.0
    except Exception as exc:
        print(f"Warm-up clip unavailable ({exc}); warming up on silence.", file=sys.stderr)
        return np.zeros(16_000, dtype=np.float32)


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
        if cfg.get("download_only") and cfg["opts"].get("fetch_only") and \
                getattr(engine_cls, "fetch", None) and engine_cls.fetch(cfg["repo"], cfg["opts"]):
            send({"event": "ready", "device": "none", "load_s": round(time.perf_counter() - t0, 1)})
            return
        engine = engine_cls(cfg["repo"], cfg["opts"])
        # The first call pays for kernel setup and caches; do it before the
        # user's first dictation rather than during it.
        try:
            engine.transcribe(warmup_audio(cfg["opts"].get("warmup_wav")), None)
        except Exception:
            traceback.print_exc()
        send({"event": "ready", "device": getattr(engine, "device", "?"),
              "load_s": round(time.perf_counter() - t0, 1)})
    except Exception as exc:
        traceback.print_exc()
        send({"event": "error", "error": f"{type(exc).__name__}: {exc}"})
        sys.exit(1)  # so an install's download step fails instead of marking the model ready
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
            logprob = None
            if request.get("score") and hasattr(engine, "transcribe_scored"):
                text, logprob = engine.transcribe_scored(audio, request.get("prompt"))
            else:
                text = engine.transcribe(audio, request.get("prompt"))
            # "device" too: GgmlEngine may have moved to the CPU during this run.
            send({"text": tidy((text or "").strip()), "ms": round((time.perf_counter() - t0) * 1000),
                  "logprob": logprob, "device": getattr(engine, "device", "?")})
        except Exception as exc:
            traceback.print_exc()
            send({"error": f"{type(exc).__name__}: {exc}"})


if __name__ == "__main__":
    main()
