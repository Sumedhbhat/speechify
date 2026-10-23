"""Stand-in engines for testing the worker protocol without a real model."""

import time


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
        if audio.size > 16_000:  # the warm-up clip is exactly one second
            time.sleep(float(self.cfg.get("clip", 0)))
        text = self.cfg.get("text", "the teacher heard this")
        logprob = self.cfg.get("logprob")
        return text or (prompt or ""), float(logprob) if logprob else None
