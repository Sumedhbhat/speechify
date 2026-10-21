"""A stand-in engine for testing the worker protocol without a real model."""


class Echo:
    device = "test"

    def __init__(self, repo, opts):
        if repo == "fail":
            raise RuntimeError("could not load")

    def transcribe(self, audio, prompt):
        if audio.size == 3:
            raise ValueError("bad clip")
        return f"{audio.size} samples, sum {float(audio.sum()):.1f}, prompt {prompt}"
