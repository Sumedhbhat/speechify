"""Stand-in engines for testing the worker protocol without a real model."""

import time

# The worker warms up on the first 3 s of the self-test clip (worker.warmup_audio).
WARMUP_SAMPLES = 3 * 16_000


class Echo:
    device = "test"

    def __init__(self, repo, opts):
        if repo == "fail":
            raise RuntimeError("could not load")

    def transcribe(self, audio, prompt):
        if audio.size == 3:
            raise ValueError("bad clip")
        return f"{audio.size} samples, sum {float(audio.sum()):.1f}, prompt {prompt}"


class Teacher:
    """A teacher with token scores. `repo` sets its behaviour, e.g.
    "load=0.5,clip=2,logprob=-0.1,text=hello there" (seconds to load,
    seconds per clip). "text=" with no value echoes the prompt."""

    device = "test"

    def __init__(self, repo, opts):
        self.cfg = dict(part.split("=", 1) for part in repo.split(",") if "=" in part)
        time.sleep(float(self.cfg.get("load", 0)))

    def transcribe(self, audio, prompt):
        return self.transcribe_scored(audio, prompt)[0]

    def transcribe_scored(self, audio, prompt):
        if audio.size != WARMUP_SAMPLES:
            time.sleep(float(self.cfg.get("clip", 0)))
        text = self.cfg.get("text", "the teacher heard this")
        logprob = self.cfg.get("logprob")
        return text or (prompt or ""), float(logprob) if logprob else None


class GpuCrash:
    """Takes the whole process down on any device but the CPU, the way a
    broken GPU driver does. repo "load" crashes while loading, "run" in the
    first real transcription."""

    def __init__(self, repo, opts):
        import os

        self.device = opts["device"]
        self.crash = repo if self.device != "cpu" else ""
        if self.crash == "load":
            os._exit(3)

    def transcribe(self, audio, prompt):
        import os

        if self.crash == "run" and audio.size != WARMUP_SAMPLES:
            os._exit(3)
        return f"{audio.size} samples on {self.device}"
