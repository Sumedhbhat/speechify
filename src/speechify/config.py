"""User config, stored as TOML in the per-user data directory."""

from __future__ import annotations

import json
import os
import re
import sys
import tomllib
from dataclasses import dataclass, fields
from pathlib import Path

DEFAULT_CONFIG = """\
# Speechify config. Restart the app after editing.

# Hold this key to talk, release to transcribe and paste.
# Any key name: ctrl_r, alt_r (Right Option on a Mac), f13, caps_lock, ...
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
device = "{device}"{device_pad}# "cuda" (NVIDIA GPU) or "cpu"
compute_type = "{compute_type}"{compute_pad}# "float16" on GPU; "int8" on CPU, also for Parakeet/Canary
language = "en"            # "" for auto-detect (slower, less reliable)
beam_size = 5              # Whisper only: 1 is ~2x faster, 5 is a bit more accurate

# Input device name or index; "" uses the system default microphone.
input_device = ""

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
"""


def has_cuda() -> bool:
    """Whether to set up models for an NVIDIA GPU.

    Windows always does: that's the machine Speechify is built for, and CI has
    to test the GPU packages there even though its runners have no GPU. On
    Linux it depends on the NVIDIA driver being loaded; Macs have no CUDA.
    SPEECHIFY_CUDA=1 or 0 overrides this.
    """
    override = os.environ.get("SPEECHIFY_CUDA")
    if override in ("0", "1"):
        return override == "1"
    if sys.platform == "win32":
        return True
    if sys.platform == "darwin":
        return False
    return Path("/proc/driver/nvidia/version").exists()


def platform_defaults() -> dict:
    """Settings that depend on the machine. Without a GPU, Parakeet is the
    default: it's the fastest accurate model on a CPU, with int8 weights."""
    cuda = has_cuda()
    return {
        # Mac keyboards have no Right Ctrl.
        "ptt_key": "alt_r" if sys.platform == "darwin" else "ctrl_r",
        "model": "whisper-large-v3-turbo" if cuda else "parakeet-tdt-0.6b-v3",
        "device": "cuda" if cuda else "cpu",
        "compute_type": "float16" if cuda else "int8",
    }


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
    device: str = "cuda"
    compute_type: str = "float16"
    language: str = "en"
    beam_size: int = 5
    input_device: str = ""
    min_hold_ms: int = 250
    autocorrect_after: int = 2
    keep_all_audio: bool = True
    llm_url: str = ""
    llm_model: str = ""

    @classmethod
    def load(cls) -> Config:
        path = data_dir() / "config.toml"
        if not path.exists():
            path.write_text(default_config(), encoding="utf-8")
        raw = tomllib.loads(path.read_text(encoding="utf-8"))
        known = {f.name for f in fields(cls)}
        unknown = set(raw) - known
        if unknown:
            print(f"config: ignoring unknown keys {sorted(unknown)}")
        return cls(**{**platform_defaults(), **{k: v for k, v in raw.items() if k in known}})

    def engine_opts(self) -> dict:
        return {
            "device": self.device,
            "compute_type": self.compute_type,
            "language": self.language,
            "beam_size": self.beam_size,
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
