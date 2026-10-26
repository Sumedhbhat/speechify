"""User config, stored as TOML in the per-user data directory."""

from __future__ import annotations

import ctypes.util
import functools
import json
import os
import platform
import re
import sys
import tomllib
from dataclasses import dataclass, fields
from pathlib import Path

DEFAULT_CONFIG = """\
# Speechify config. Restart the app after editing.

# Hold this key to talk, release to transcribe and paste.
# Any key name: ctrl_r, alt_r (Right Option on a Mac), f13, scroll_lock, ...
ptt_key = "{ptt_key}"

# Opens the correction window for the last dictation.
correct_hotkey = "<ctrl>+<alt>+f"

# Opens the history of every recording, where you can replay and edit them.
history_hotkey = "<ctrl>+<alt>+h"

# Opens the model picker.
models_hotkey = "<ctrl>+<alt>+m"

# Speech model. Pick one in the model window (Ctrl+Alt+M) or run
# `speechify models` to list them. A path to a model from scripts/finetune.py
# also works.
model = "{model}"
# "auto" uses the best GPU there is: CUDA on NVIDIA; Vulkan on AMD and Intel
# GPUs and Metal on Apple Silicon (through transcribe.cpp); else the CPU.
# Or name one: "cuda", "vulkan", "metal" or "cpu".
device = "{device}"{device_pad}# `speechify selftest --engine-only` shows what it picked
compute_type = "{compute_type}"{compute_pad}# CUDA models: "float16"; CPU: "int8", also for Parakeet/Canary
language = "en"            # "" for auto-detect (slower, less reliable)
beam_size = 5              # Whisper only: 1 is ~2x faster, 5 is a bit more accurate

# CPU Parakeet only: encoder/decoder threads share one pool where supported.
# 4 bounds CPU use; 0 restores ONNX Runtime's automatic thread pools.
onnx_threads = 4

# Input device name or index; "" uses the system default microphone.
input_device = ""

# The microphone is only open while you hold the talk key. true keeps it open
# all the time, with 300 ms of audio from before the key went down, so the
# first word is never cut off; the system shows the mic as in use throughout.
keep_mic_open = false

# Presses shorter than this are ignored, so tapping the key does nothing.
min_hold_ms = 250

# How many times you must make the same correction before it's applied
# automatically. Learned vocabulary is used as a Whisper hint immediately.
autocorrect_after = 2

# Save the audio + text of every dictation (~2 MB per minute of speech).
# Everything saved is used for fine-tuning: your edits where you made them,
# and the pasted text where you didn't. Set false to only keep corrected ones.
keep_all_audio = true

# Optional LLM cleanup via any OpenAI-compatible endpoint (e.g. Ollama).
# Adds latency; leave empty to disable.
llm_url = ""               # e.g. "http://localhost:11434/v1/chat/completions"
llm_model = ""             # e.g. "qwen2.5:3b-instruct"

# --- Getting better on its own ------------------------------------------------
# While you're away and the computer is plugged in, a bigger "teacher" model
# re-transcribes your saved recordings to make training labels, and the
# dictation model is retrained on them now and then. A new version is only
# switched in if it beats the current one. `speechify learn status` shows
# progress; `speechify models --rollback` undoes a switch.
learn_auto = true
# "auto" = Granite Speech 4.1 2B, or Qwen3-ASR 1.7B when VRAM is short.
# Any model id from `speechify models` works, or "off".
teacher = "auto"
idle_minutes = 5           # no dictation for this long counts as idle
teacher_on_cpu = false     # without an NVIDIA GPU, only run the teacher if true
teacher_min_confidence = 0.7  # 0-1; less confident teacher labels aren't trained on
retrain_after_labels = 300 # retrain after this many new labelled recordings...
retrain_after_days = 7     # ...or this many days, whichever comes first
max_regression_wer = 0.5   # WER points a new model may lose on LibriSpeech vs the base
keep_versions = 3          # trained versions kept on disk
"""


DRI = Path("/dev/dri")  # GPU device nodes on Linux

# What `device` can be set to. "auto" picks one of the others (accelerator()).
DEVICES = ("auto", "cuda", "vulkan", "metal", "cpu")


def has_cuda() -> bool:
    """Whether to set up models for an NVIDIA GPU.

    On Windows and Linux that depends on the NVIDIA driver being installed;
    Macs have no CUDA. SPEECHIFY_CUDA=1 or 0 overrides this (CI sets it to
    test the GPU packages on runners without a GPU).
    """
    override = os.environ.get("SPEECHIFY_CUDA")
    if override in ("0", "1"):
        return override == "1"
    if sys.platform == "win32":
        # Every NVIDIA display driver installs nvcuda.dll; nothing else does.
        return (_system32() / "nvcuda.dll").exists()
    if sys.platform == "darwin":
        return False
    return Path("/proc/driver/nvidia/version").exists()


def accelerator() -> str:
    """What "auto" means on this machine: "cuda", "vulkan", "metal" or "cpu".

    NVIDIA GPUs keep the CUDA builds of each backend. Any other GPU runs the
    models transcribe.cpp supports through Vulkan, and Apple Silicon through
    Metal. This only checks that a GPU and the Vulkan loader exist, without
    loading a driver into the app; the model worker makes sure the GPU
    really works and falls back to the CPU if it doesn't.
    SPEECHIFY_GPU=<device> overrides it.
    """
    override = os.environ.get("SPEECHIFY_GPU")
    if override in DEVICES[1:]:
        return override
    if has_cuda():
        return "cuda"
    return _probe_gpu()


@functools.cache
def _probe_gpu() -> str:
    if sys.platform == "darwin":
        # transcribe.cpp ships Metal for Apple Silicon only; Intel Macs get its CPU build.
        return "metal" if platform.machine() == "arm64" else "cpu"
    if sys.platform == "win32":
        # vulkan-1.dll comes with every AMD, Intel and NVIDIA driver.
        return "vulkan" if (_system32() / "vulkan-1.dll").exists() else "cpu"
    # Linux: a GPU render node and the Vulkan loader (libvulkan.so.1). Mesa
    # provides the driver for AMD and Intel GPUs.
    if any(DRI.glob("renderD*")) and _find_library("vulkan"):
        return "vulkan"
    return "cpu"


def _find_library(name: str) -> str | None:
    try:
        return ctypes.util.find_library(name)
    except OSError:
        return None


def _system32() -> Path:
    return Path(os.environ.get("SystemRoot", r"C:\Windows")) / "System32"


def platform_defaults() -> dict:
    """Settings that depend on the machine. Without an NVIDIA GPU, Parakeet
    is the default: it's the fastest accurate model on Vulkan, Metal and the
    CPU (with int8 weights there)."""
    cuda = has_cuda()
    return {
        # Mac keyboards have no Right Ctrl.
        "ptt_key": "alt_r" if sys.platform == "darwin" else "ctrl_r",
        "model": "whisper-large-v3-turbo" if cuda else "parakeet-tdt-0.6b-v3",
        "device": "auto",
        "compute_type": "float16" if cuda else "int8",
    }


# The `device` line that config files written before "auto" existed start
# with. Left untouched it only recorded what the machine had then, so it
# becomes "auto"; a line someone edited is kept.
_OLD_DEVICE_LINES = [
    f'device = "{d}"{" " * (16 - len(d))}# "cuda" (NVIDIA GPU) or "cpu"' for d in ("cuda", "cpu")
]


def _upgrade(text: str) -> str:
    for old in _OLD_DEVICE_LINES:
        if re.search(rf"(?m)^{re.escape(old)}$", text):
            new = default_config().split("\n")
            start = next(i for i, line in enumerate(new) if line.startswith("# \"auto\" uses"))
            block = "\n".join(new[start:start + 4])
            text = re.sub(rf"(?m)^{re.escape(old)}$", lambda m: block, text)
            print('config: device is now "auto", so a GPU other than NVIDIA is used too')
    return text


def default_config() -> str:
    d = platform_defaults()
    pad = lambda value, width: " " * max(1, width - len(value))  # noqa: E731  keeps comments aligned
    return DEFAULT_CONFIG.format(
        **d, device_pad=pad(d["device"], 16), compute_pad=pad(d["compute_type"], 10)
    )


def data_dir() -> Path:
    if os.environ.get("SPEECHIFY_HOME"):  # isolates test runs from real data
        path = Path(os.environ["SPEECHIFY_HOME"])
        path.mkdir(parents=True, exist_ok=True)
        return path
    if sys.platform == "win32":
        base = Path(os.environ.get("APPDATA", Path.home() / "AppData" / "Roaming"))
    elif sys.platform == "darwin":
        base = Path.home() / "Library" / "Application Support"
    else:
        base = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share"))
    path = base / "speechify"
    path.mkdir(parents=True, exist_ok=True)
    return path


@dataclass
class Config:
    ptt_key: str = "ctrl_r"
    correct_hotkey: str = "<ctrl>+<alt>+f"
    history_hotkey: str = "<ctrl>+<alt>+h"
    models_hotkey: str = "<ctrl>+<alt>+m"
    model: str = "whisper-large-v3-turbo"
    device: str = "auto"
    compute_type: str = "float16"
    language: str = "en"
    beam_size: int = 5
    onnx_threads: int = 4
    input_device: str = ""
    keep_mic_open: bool = False
    min_hold_ms: int = 250
    autocorrect_after: int = 2
    keep_all_audio: bool = True
    llm_url: str = ""
    llm_model: str = ""
    learn_auto: bool = True
    teacher: str = "auto"
    idle_minutes: float = 5
    teacher_on_cpu: bool = False
    teacher_min_confidence: float = 0.7
    retrain_after_labels: int = 300
    retrain_after_days: float = 7
    max_regression_wer: float = 0.5
    keep_versions: int = 3

    @classmethod
    def load(cls) -> Config:
        path = data_dir() / "config.toml"
        if not path.exists():
            path.write_text(default_config(), encoding="utf-8")
        text = path.read_text(encoding="utf-8")
        upgraded = _upgrade(text)
        if upgraded != text:
            path.write_text(upgraded, encoding="utf-8")
        raw = tomllib.loads(upgraded)
        known = {f.name for f in fields(cls)}
        unknown = set(raw) - known
        if unknown:
            print(f"config: ignoring unknown keys {sorted(unknown)}")
        return cls(**{**platform_defaults(), **{k: v for k, v in raw.items() if k in known}})

    def resolved_device(self) -> str:
        """`device` with "auto" replaced by what this machine has."""
        if self.device not in DEVICES:
            print(f"config: unknown device {self.device!r}; using \"auto\"")
            return accelerator()
        return accelerator() if self.device == "auto" else self.device

    def engine_opts(self) -> dict:
        return {
            "device": self.resolved_device(),
            "compute_type": self.compute_type,
            "language": self.language,
            "beam_size": self.beam_size,
            "onnx_threads": self.onnx_threads,
        }


def set_value(key: str, value) -> None:
    """Change one setting in config.toml, keeping the rest of the file as is."""
    path = data_dir() / "config.toml"
    text = path.read_text(encoding="utf-8") if path.exists() else default_config()
    line = f"{key} = {json.dumps(value)}"  # JSON strings, numbers and bools are valid TOML
    text, n = re.subn(rf"(?m)^{re.escape(key)}\s*=.*$", lambda m: line, text)
    if n == 0:
        text = text.rstrip("\n") + f"\n{line}\n"
    path.write_text(text, encoding="utf-8")
