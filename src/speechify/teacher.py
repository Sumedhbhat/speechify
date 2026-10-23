"""Which teacher model to run, and where it goes so dictation never waits.

Placement rules (see place()):

- GPU with room for both: the teacher runs in its own worker next to the
  dictation model. It pauses between clips while you dictate, and isn't
  stopped.
- GPU without room for both: the dictation model is unloaded while the
  teacher runs. Pressing the dictation key kills the teacher at once and
  reloads the dictation model while you speak.
- No NVIDIA GPU: nothing runs unless `teacher_on_cpu = true`. Then the
  teacher runs on the CPU next to the dictation model and is killed the
  moment you press the key, so it never competes for the CPU.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from .engines.catalog import MODELS, ModelSpec, resolve
from .gpu import GpuInfo

# "auto": the most accurate first, then the one that fits in less VRAM.
AUTO_TEACHERS = ("granite-speech-4.1-2b", "qwen3-asr-1.7b")
# Room left on the GPU for the CUDA context, activations and other apps.
HEADROOM_GB = 1.0


@dataclass(frozen=True)
class Placement:
    device: str            # "cuda" or "cpu"
    unload_student: bool   # the dictation model is stopped while this runs
    kill_on_keydown: bool  # pressing the dictation key kills it
    why: str


def choose(setting: str, gpu: GpuInfo | None, allow_cpu: bool, data_dir: Path) -> tuple[ModelSpec | None, str]:
    """(teacher, reason). The teacher is None when there's nothing to run."""
    setting = (setting or "").strip()
    if setting.lower() in ("", "off", "none", "false"):
        return None, "the teacher is turned off (teacher = \"off\")"
    if setting == "auto":
        candidates = [MODELS[m] for m in AUTO_TEACHERS]
    else:
        try:
            candidates = [resolve(setting, data_dir)]
        except KeyError as exc:
            return None, str(exc)
    if gpu is None:
        if not allow_cpu:
            return None, "no NVIDIA GPU; set teacher_on_cpu = true to run the teacher on the CPU"
        # The smallest is the least slow on a CPU.
        return min(candidates, key=lambda m: m.vram_gb), "CPU (teacher_on_cpu = true)"
    for spec in candidates:
        if spec.vram_gb + HEADROOM_GB <= gpu.total_gb:
            return spec, f"{gpu.name}, {gpu.total_gb:.0f} GB"
    if allow_cpu:
        return min(candidates, key=lambda m: m.vram_gb), "too little VRAM, so on the CPU"
    names = " or ".join(m.name for m in candidates)
    return None, f"{gpu.name} has {gpu.total_gb:.0f} GB, too little for {names}"


def place(need_gb: float, gpu: GpuInfo | None) -> Placement:
    """Where a background job needing `need_gb` of VRAM runs.

    `gpu.free_gb` is measured with the dictation model already loaded.
    """
    if gpu is None:
        return Placement("cpu", unload_student=False, kill_on_keydown=True,
                         why="CPU: killed whenever you dictate")
    if gpu.free_gb >= need_gb + HEADROOM_GB:
        return Placement("cuda", unload_student=False, kill_on_keydown=False,
                         why=f"next to the dictation model ({gpu.free_gb:.1f} GB free)")
    return Placement("cuda", unload_student=True, kill_on_keydown=True,
                     why=f"alone on the GPU ({gpu.free_gb:.1f} GB free is too little for both)")


def opts(language: str, device: str) -> dict:
    return {"device": device, "compute_type": "float16" if device == "cuda" else "int8",
            "language": language, "beam_size": 5}
