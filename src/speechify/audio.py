"""Microphone capture.

By default the microphone is only open while the talk key is held, so the
system's "microphone in use" light means you're dictating. Opening a stream
can take 100-300 ms on Windows, which must not happen on the keyboard hook
thread, so a worker thread opens and closes it. `keep_mic_open` keeps the
stream open the whole time instead, with a short pre-roll buffer, so the
first word can never be cut off.
"""

from __future__ import annotations

import collections
import threading
import wave
from pathlib import Path
from typing import Callable

import numpy as np

SAMPLE_RATE = 16_000
PREROLL_SECONDS = 0.3
BLOCK = 480  # 30 ms


class Recorder:
    """`level` (RMS of the last 30 ms) and `live` (audio is arriving) are read
    by the overlay to draw the level bars."""

    def __init__(
        self,
        device: str | int | None = None,
        keep_open: bool = False,
        on_error: Callable[[str], None] | None = None,
    ):
        import sounddevice as sd  # imported here so the fine-tune script doesn't need PortAudio

        if isinstance(device, str) and device.isdigit():
            device = int(device)
        self._sd = sd
        self._device = device or None
        self.keep_open = keep_open
        self._on_error = on_error
        self.level = 0.0
        self.live = False
        self.error: str | None = None  # why the last open failed
        self._lock = threading.Lock()
        self._recording = False
        self._chunks: list[np.ndarray] = []
        self._preroll: collections.deque[np.ndarray] = collections.deque(
            maxlen=int(PREROLL_SECONDS * SAMPLE_RATE / BLOCK)
        )
        self._stream = None
        self._want_open = keep_open
        self._closing = False
        self._changed = threading.Condition(self._lock)
        if keep_open:
            self._stream = self._open()  # fails at startup, as a missing mic should
        threading.Thread(target=self._manage, daemon=True, name="mic").start()

    def _open(self):
        stream = self._sd.InputStream(
            samplerate=SAMPLE_RATE,
            channels=1,
            dtype="float32",
            blocksize=BLOCK,
            device=self._device,
            callback=self._callback,
            latency="low",
        )
        stream.start()
        return stream

    def _manage(self) -> None:
        """Open or close the stream to match what start() and stop() asked for.

        Requests are coalesced: a quick press and release while the stream is
        still opening just closes it again.
        """
        while True:
            with self._changed:
                while not self._closing and self._want_open == (self._stream is not None):
                    self._changed.wait()
                want, stream = self._want_open and not self._closing, self._stream
            if want:
                try:
                    stream = self._open()
                    self.error = None
                except Exception as exc:  # no microphone, or permission denied
                    with self._lock:
                        self._want_open = False
                    self.error = str(exc) or type(exc).__name__
                    if self._on_error:
                        self._on_error(self.error)
                    continue
                with self._lock:
                    self._stream = stream
            else:
                with self._lock:
                    self._stream = None
                    self._preroll.clear()
                try:
                    stream.stop()
                    stream.close()
                except Exception:
                    pass
                if self._closing:
                    return

    def _callback(self, indata, frames, time_info, status) -> None:
        block = indata[:, 0].copy()
        with self._lock:
            if self._recording:
                self._chunks.append(block)
                self.level = float(np.sqrt(np.mean(block**2)))
                self.live = True
            else:
                self._preroll.append(block)

    def start(self) -> None:
        with self._lock:
            self._chunks = list(self._preroll) if self.keep_open else []
            self._preroll.clear()
            self._recording = True
            self.level, self.live = 0.0, False
            self.error = None
            if not self._want_open:
                self._want_open = True
                self._changed.notify()

    def stop(self) -> np.ndarray:
        with self._lock:
            self._recording = False
            self.level, self.live = 0.0, False
            chunks, self._chunks = self._chunks, []
            if not self.keep_open and self._want_open:
                self._want_open = False
                self._changed.notify()
        if not chunks:
            return np.zeros(0, dtype=np.float32)
        return np.concatenate(chunks)

    def close(self) -> None:
        with self._lock:
            self._closing = True
            self._recording = False
            stream, self._stream = self._stream, None
            self._changed.notify()
        if stream is not None:
            stream.stop()
            stream.close()


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
