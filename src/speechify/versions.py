"""Models trained by automatic retraining: which ones exist, and rollback.

Kept in learning.db (learn_state "versions"), oldest first. Each entry:

    {"name": "whisper-ft-20261023-0300", "path": ".../models/whisper-ft-...",
     "hf": ".../models/hf-...", "base": "whisper-large-v3-turbo", "ts": ...,
     "gate1": {...}, "gate2": {...}}

Only versions made by automatic retraining are pruned: models you trained by
hand with scripts/finetune.py are never deleted.
"""

from __future__ import annotations

import shutil
import time
from pathlib import Path

KEY = "versions"


def entries(learner) -> list[dict]:
    return list(learner.get_state(KEY, []))


def _same(a: str, b: str) -> bool:
    return Path(a).name == Path(b).name if a and b else False


def record(learner, entry: dict, keep: int, active: str | list[str]) -> list[str]:
    """Add a new version and delete all but the newest `keep`. Returns the
    names of the versions deleted. The `active` model(s) are never deleted."""
    protected = [active] if isinstance(active, str) else list(active)
    versions = entries(learner) + [entry]
    versions.sort(key=lambda v: v["ts"])
    removed = []
    while len(versions) > max(1, keep):
        victim = next((v for v in versions if not any(_same(v["path"], a) for a in protected)), None)
        if victim is None:
            break
        versions.remove(victim)
        for d in (victim.get("path"), victim.get("hf")):
            if d:
                shutil.rmtree(d, ignore_errors=True)
        removed.append(victim["name"])
    learner.set_state(KEY, versions)
    return removed


class RollbackError(RuntimeError):
    pass


def rollback_target(learner, current: str) -> tuple[str, str]:
    """(model value for config.toml, description) one step before `current`.

    From a trained version that's the previous trained version still on
    disk; from the oldest one, the base model it was trained from.
    """
    versions = sorted(entries(learner), key=lambda v: v["ts"])
    index = next((i for i, v in enumerate(versions) if _same(v["path"], current)), None)
    if index is None:
        if not versions:
            raise RollbackError("Speechify hasn't switched to a retrained model yet, so there's nothing to roll back.")
        raise RollbackError(f"The current model ({current}) wasn't made by automatic retraining; "
                            "pick another one in the model window instead.")
    for v in reversed(versions[:index]):
        if (Path(v["path"]) / "model.bin").exists():
            return v["path"], f"the previous version ({v['name']})"
    base = versions[index]["base"]
    return base, f"the base model ({base})"


def rollback(learner, cfg) -> tuple[str, str]:
    """Switch config.toml back one version. Returns (model, description)."""
    from .config import set_value

    target, description = rollback_target(learner, cfg.model)
    learner.set_state("last_rollback", {"ts": time.time(), "from": cfg.model, "to": target})
    set_value("model", target)
    cfg.model = target
    return target, description
