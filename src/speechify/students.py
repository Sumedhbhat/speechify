"""Dictation models that can be retrained on your recordings.

Only Whisper today: Parakeet and Canary would need NVIDIA NeMo, which
doesn't run on Windows. A new kind of student needs a training script with
the same command line and report as scripts/finetune.py, and an entry here.
"""

from __future__ import annotations

import json
import sys
from dataclasses import dataclass
from pathlib import Path

from .engines.catalog import ModelSpec

VERSION_FILE = "speechify-version.json"  # written by the trainer into each model it saves


@dataclass(frozen=True)
class Trainer:
    backend: str
    script: str                 # file name under scripts/
    vram_gb: float              # roughly what a training run needs
    bases: dict[str, str]       # catalog id -> Hugging Face repo to train from
    default_base: str

    def base_repo(self, spec: ModelSpec) -> str:
        """The base model a run starts from. Every run starts from it again."""
        if spec.id in self.bases:
            return self.bases[spec.id]
        info = version_info(Path(spec.repo))
        return info.get("base_repo") or self.default_base

    def base_model_id(self, spec: ModelSpec) -> str:
        if spec.id in self.bases:
            return spec.id
        info = version_info(Path(spec.repo))
        repo = self.base_repo(spec)
        return info.get("base_id") or next((k for k, v in self.bases.items() if v == repo), repo)

    def current_ref(self, spec: ModelSpec) -> str | None:
        """What the trainer loads to score the current model: a Hugging Face
        repo, or the full-precision copy saved next to a trained model.
        None if that copy is gone, and the current model can't be scored."""
        if spec.id in self.bases:
            return self.bases[spec.id]
        path = Path(spec.repo)
        hf = path.parent / ("hf-" + path.name.removeprefix("whisper-ft-"))
        return str(hf) if (hf / "config.json").exists() else None

    def script_path(self) -> Path:
        return script_path(self.script)


TRAINERS = {
    "whisper": Trainer(
        "whisper", "finetune.py", vram_gb=10,
        bases={
            "whisper-large-v3-turbo": "openai/whisper-large-v3-turbo",
            "whisper-large-v3": "openai/whisper-large-v3",
        },
        default_base="openai/whisper-large-v3-turbo",
    ),
}


def trainer_for(spec: ModelSpec | None) -> Trainer | None:
    if spec is None or not spec.finetune:
        return None
    return TRAINERS.get(spec.backend)


def version_info(model_dir: Path) -> dict:
    try:
        return json.loads((model_dir / VERSION_FILE).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}


def script_path(name: str) -> Path:
    """A source checkout has scripts/ next to src/; the installed app bundles
    them in speechify/_scripts (packaging/speechify.spec)."""
    here = Path(__file__).resolve().parent
    candidates = [here.parents[1] / "scripts" / name, here / "_scripts" / name]
    if getattr(sys, "frozen", False):
        candidates.insert(0, Path(sys._MEIPASS) / "speechify" / "_scripts" / name)
    for c in candidates:
        if c.exists():
            return c
    raise FileNotFoundError(f"{name} not found (looked in {', '.join(map(str, candidates))})")


# What a training script imports from speechify. Stdlib + numpy only.
TRAIN_MODULES = ("__init__.py", "audio.py", "config.py", "labels.py", "metrics.py")


def stage(trainer: Trainer, root: Path) -> Path:
    """Copy the training script and the modules it imports into `root`.

    The script runs in its own environment, so it mustn't see the app's
    packages (in the installed app those sit next to these files).
    Returns the staged script; it finds the modules at ../src.
    """
    here = Path(__file__).resolve().parent
    files = {root / "scripts" / trainer.script: trainer.script_path()}
    files.update({root / "src" / "speechify" / name: here / name for name in TRAIN_MODULES})
    for dst, src in files.items():
        dst.parent.mkdir(parents=True, exist_ok=True)
        data = src.read_bytes()
        if not dst.exists() or dst.read_bytes() != data:
            dst.write_bytes(data)
    return root / "scripts" / trainer.script
