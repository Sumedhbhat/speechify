"""What the settings window may change, checked and saved without Tk."""

from __future__ import annotations

from dataclasses import fields

from . import config, keys
from .combos import parse
from .config import DEVICES, Config

HOTKEYS = ("correct_hotkey", "history_hotkey", "models_hotkey")
# Changing these reloads the speech model.
RELOADS_MODEL = {"device", "compute_type", "language", "beam_size"}
_TYPES = {f.name: f.type for f in fields(Config)}


def coerce(name: str, raw) -> object:
    """A value from a Tk variable, as the type the config uses."""
    kind = _TYPES[name]
    if kind in ("bool", bool):
        return bool(raw)
    if kind in ("int", int):
        return int(str(raw).strip())
    if kind in ("float", float):
        value = float(str(raw).strip())
        return int(value) if value.is_integer() else value
    return str(raw).strip()


def problems(values: dict) -> list[str]:
    """Why these settings can't be saved, in words for the settings window."""
    out = []
    numbers = {}
    for name in values:
        try:
            numbers[name] = coerce(name, values[name])
        except ValueError:
            out.append(f"{name.replace('_', ' ')} must be a number")
    if "ptt_key" in values:
        try:
            keys.check_key_name(str(values["ptt_key"]).strip())
        except ValueError as exc:
            out.append(str(exc))
    combos = {}
    for name in HOTKEYS:
        if name in values:
            try:
                combos[name] = parse(str(values[name]).strip())
            except ValueError as exc:
                out.append(str(exc))
    if len(set(combos.values())) < len(combos):
        out.append("two shortcuts are the same")
    if "device" in values and values["device"] not in DEVICES:
        out.append(f"device must be one of {', '.join(DEVICES)}")
    if numbers.get("min_hold_ms", 0) < 0:
        out.append("the shortest press can't be negative")
    if not 1 <= numbers.get("beam_size", 1) <= 10:
        out.append("beam size must be between 1 and 10")
    if numbers.get("autocorrect_after", 1) < 1:
        out.append("a fix has to be seen at least once before it's automatic")
    return out


def changes(cfg: Config, values: dict) -> dict:
    """The settings in `values` that differ from `cfg`, as config types."""
    out = {}
    for name, raw in values.items():
        value = coerce(name, raw)
        if value != getattr(cfg, name):
            out[name] = value
    return out


def save(cfg: Config, changed: dict) -> None:
    """Write each changed setting to config.toml and to the running config."""
    for name, value in changed.items():
        config.set_value(name, value)
        setattr(cfg, name, value)
