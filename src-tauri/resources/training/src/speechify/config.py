"""Trainer configuration. Reads only the Speechify data directory it is given."""

import os
import tomllib
from pathlib import Path
from types import SimpleNamespace


def data_dir():
    p = Path(os.environ["SPEECHIFY_TAURI_TRAINING_HOME"]).resolve()
    if p.name != "org.sumedhbhat.speechify":
        raise ValueError("Speechify data directory required")
    return p


class Config:
    @staticmethod
    def load():
        return SimpleNamespace(**tomllib.loads((data_dir() / "config.toml").read_text()))


def set_value(*args):
    raise RuntimeError("Only the Rust supervisor may adopt a trained model")
