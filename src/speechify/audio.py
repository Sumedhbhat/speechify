"""Microphone capture.

The input stream stays open the whole time the app runs. Opening a stream on
Windows can take 100-300 ms, and doing that on every key press is a big part
of why dictation feels laggy and clips the first word. We also keep a short
pre-roll buffer, so audio from just before the key went down is included.
"""

from __future__ import annotations

import collections
import threading
import wave
from pathlib import Path

import numpy as np

SAMPLE_RATE = 16_000
PREROLL_SECONDS = 0.3
BLOCK = 480  # 30 ms


class Recorder:
    def __init__(self, device: str | int | None = None):
        import sounddevice as sd  # imported here so the fine-tune script doesn't need PortAudio

        if isinstance(device, str) and device.isdigit():
            device = int(device)
        self._lock = threading.Lock()
        self._recording = False
        self._chunks: list[np.ndarray] = []
        self._preroll: collections.deque[np.ndarray] = collections.deque(
            maxlen=int(PREROLL_SECONDS * SAMPLE_RATE / BLOCK)
        )
        self._stream = sd.InputStream(
            samplerate=SAMPLE_RATE,
            channels=1,
            dtype="float32",
            blocksize=BLOCK,
            device=device or None,
            callback=self._callback,
            latency="low",
        )
        self._stream.start()

    def _callback(self, indata, frames, time_info, status) -> None:
        block = indata[:, 0].copy()
        with self._lock:
            if self._recording:
                self._chunks.append(block)
            else:
                self._preroll.append(block)

    def start(self) -> None:
        with self._lock:
            self._chunks = list(self._preroll)
            self._preroll.clear()
            self._recording = True

    def stop(self) -> np.ndarray:
        with self._lock:
            self._recording = False
            chunks, self._chunks = self._chunks, []
        if not chunks:
            return np.zeros(0, dtype=np.float32)
        return np.concatenate(chunks)

    def close(self) -> None:
        self._stream.stop()
        self._stream.close()


def is_silent(audio: np.ndarray, threshold: float = 0.004) -> bool:
    """True if the clip is too quiet to contain speech.

    Whisper hallucinates phrases like "Thank you." on silence, so we skip
    those clips entirely.
    """
    if audio.size == 0:
        return True
    return float(np.sqrt(np.mean(audio**2))) < threshold


def save_wav(audio: np.ndarray, path: Path) -> None:
    pcm = (np.clip(audio, -1.0, 1.0) * 32767).astype(np.int16)
    with wave.open(str(path), "wb") as f:
        f.setnchannels(1)
        f.setsampwidth(2)
        f.setframerate(SAMPLE_RATE)
        f.writeframes(pcm.tobytes())


def load_wav(path: Path) -> np.ndarray:
    with wave.open(str(path), "rb") as f:
        if f.getframerate() != SAMPLE_RATE or f.getnchannels() != 1:
            raise ValueError(f"{path}: expected 16 kHz mono")
        pcm = np.frombuffer(f.readframes(f.getnframes()), dtype=np.int16)
    return pcm.astype(np.float32) / 32768.0


def play(path: Path) -> None:
    """Play a saved recording without blocking."""
    import sys

    if sys.platform == "win32":
        import winsound

        winsound.PlaySound(str(path), winsound.SND_FILENAME | winsound.SND_ASYNC)
    else:
        import sounddevice as sd

        sd.play(load_wav(path), SAMPLE_RATE)


def stop_playback() -> None:
    import sys

    if sys.platform == "win32":
        import winsound

        winsound.PlaySound(None, 0)
    else:
        import sounddevice as sd

        sd.stop()
