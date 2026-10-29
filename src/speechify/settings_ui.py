"""Settings window: everything in config.toml worth changing, without
editing the file. Most changes apply at once (see App.apply_settings)."""

from __future__ import annotations

import sys
import tkinter as tk
from tkinter import ttk
from typing import Callable

from . import __version__, autostart, settings
from .applog import open_in_os
from .audio import microphones
from .combos import describe_combo, describe_key, key_from_tk
from .config import DEVICES, Config, accelerator, data_dir
from .ui import FONT

MAC = sys.platform == "darwin"
TALK_KEYS = (
    ["alt_r", "cmd_r", "ctrl_r", "shift_r", "f13", "f14"] if MAC
    else ["ctrl_r", "alt_r", "shift_r", "caps_lock", "scroll_lock", "pause", "insert", "f13", "f14"]
)
DEFAULT_MIC = "System default"
MUTED = "#6b6e76"


class SettingsWindow:
    def __init__(
        self,
        master: tk.Misc,
        cfg: Config,
        on_save: Callable[[dict], None],
        open_models: Callable[[], None],
        model_name: Callable[[], str],
        welcome: bool = False,
        on_close: Callable[[], None] | None = None,
        check_update: Callable[[], None] | None = None,
    ):
        self.cfg, self.on_save, self.on_close = cfg, on_save, on_close
        self.check_update = check_update
        self.vars: dict[str, tk.Variable] = {}
        self._recording_key = False

        win = self.win = tk.Toplevel(master)
        win.title("Speechify settings")
        win.minsize(560, 460)
        win.protocol("WM_DELETE_WINDOW", self.close)
        win.bind("<Escape>", lambda e: self.close())

        outer = ttk.Frame(win, padding=14)
        outer.pack(fill="both", expand=True)
        if welcome:
            hello = ttk.Frame(outer, padding=(0, 0, 0, 10))
            hello.pack(fill="x")
            ttk.Label(hello, text="Speechify is running", font=(FONT, 13, "bold")).pack(anchor="w")
            place = "menu bar" if MAC else "tray, next to the clock"
            ttk.Label(
                hello, wraplength=520, justify="left",
                text=f"Hold {describe_key(cfg.ptt_key, MAC)} in any app, talk, and let go: the text is "
                     f"pasted where your cursor is. Speechify keeps running in the {place}; open "
                     "this window again from there, or by starting Speechify again.",
            ).pack(anchor="w", pady=(2, 0))

        tabs = ttk.Notebook(outer)
        tabs.pack(fill="both", expand=True)
        for title, build in [
            ("General", self._general),
            ("Microphone", self._microphone),
            ("Shortcuts", self._shortcuts),
            ("Model", lambda f: self._model(f, open_models, model_name)),
            ("Learning", self._learning),
        ]:
            frame = ttk.Frame(tabs, padding=14)
            frame.columnconfigure(1, weight=1)
            build(frame)
            tabs.add(frame, text=title)

        bottom = ttk.Frame(outer, padding=(0, 10, 0, 0))
        bottom.pack(fill="x")
        ttk.Button(bottom, text="Open data folder", command=lambda: open_in_os(data_dir())).pack(side="left")
        ttk.Button(bottom, text="Save", command=self.save).pack(side="right")
        ttk.Button(bottom, text="Cancel", command=self.close).pack(side="right", padx=6)
        self.status = ttk.Label(outer, foreground="#c62828", wraplength=520, justify="left")
        self.status.pack(fill="x", pady=(6, 0))
        win.bind("<Control-s>", lambda e: self.save())

    # --- building blocks -------------------------------------------------------

    def _var(self, name: str, kind=tk.StringVar) -> tk.Variable:
        value = getattr(self.cfg, name)
        var = kind(value=value if kind is tk.BooleanVar else str(value))
        self.vars[name] = var
        return var

    @staticmethod
    def _row(frame: ttk.Frame, label: str, widget: tk.Widget, hint: str = "") -> ttk.Label:
        row = frame.grid_size()[1]
        ttk.Label(frame, text=label).grid(row=row, column=0, sticky="nw", padx=(0, 12), pady=(6, 0))
        widget.grid(row=row, column=1, sticky="w", pady=(4, 0))
        note = ttk.Label(frame, text=hint, foreground=MUTED, wraplength=380, justify="left")
        note.grid(row=row + 1, column=1, sticky="w")
        return note

    def _check(self, frame: ttk.Frame, name: str, text: str, hint: str = "") -> None:
        row = frame.grid_size()[1]
        ttk.Checkbutton(frame, text=text, variable=self._var(name, tk.BooleanVar)).grid(
            row=row, column=0, columnspan=2, sticky="w", pady=(8, 0))
        if hint:
            ttk.Label(frame, text=hint, foreground=MUTED, wraplength=500, justify="left").grid(
                row=row + 1, column=0, columnspan=2, sticky="w", padx=(22, 0))

    # --- tabs ------------------------------------------------------------------

    def _general(self, f: ttk.Frame) -> None:
        key = self._var("ptt_key")
        box = ttk.Frame(f)
        combo = ttk.Combobox(box, textvariable=key, values=TALK_KEYS, width=16)
        combo.pack(side="left")
        self._record = ttk.Button(box, text="Press a key…", command=self._start_recording_key)
        self._record.pack(side="left", padx=6)
        note = self._row(f, "Talk key", box)
        key.trace_add("write", lambda *a: note.configure(text=self._describe_key(key.get())))
        note.configure(text=self._describe_key(key.get()))

        hold = ttk.Spinbox(f, from_=0, to=2000, increment=50, width=8, textvariable=self._var("min_hold_ms"))
        self._row(f, "Shortest press (ms)", hold, "Shorter taps are ignored, so tapping the key does nothing.")

        self._autostart = tk.BooleanVar(value=_autostart_enabled())
        row = f.grid_size()[1]
        ttk.Checkbutton(f, text="Start Speechify when I sign in", variable=self._autostart).grid(
            row=row, column=0, columnspan=2, sticky="w", pady=(12, 0))

        box = ttk.Frame(f)
        ttk.Label(box, text=f"Speechify {__version__}").pack(side="left")
        if self.check_update:
            ttk.Button(box, text="Check for updates", command=self.check_update).pack(side="left", padx=8)
        row = f.grid_size()[1]
        box.grid(row=row, column=0, columnspan=2, sticky="w", pady=(18, 0))
        self._check(f, "check_updates", "Check for updates automatically",
                    "Once a day, from GitHub. The tray menu then offers the new version; "
                    "nothing is installed until you click it.")

    def _describe_key(self, name: str) -> str:
        name = name.strip()
        try:
            return f"Hold {describe_key(name, MAC)} to talk." if name else ""
        except Exception:
            return ""

    def _start_recording_key(self) -> None:
        self._recording_key = True
        self._record.configure(text="Press it now…")
        self.win.bind("<KeyPress>", self._key_pressed)
        self.win.focus_force()

    def _key_pressed(self, event: tk.Event) -> str:
        name = key_from_tk(event.keysym)
        if name is None:
            self.status.configure(text=f"{event.keysym} can't be the talk key: it types. Try a "
                                       "modifier like Right Ctrl, or a key you don't use like F13.")
        else:
            self.vars["ptt_key"].set(name)
            self.status.configure(text="")
        self.win.unbind("<KeyPress>")
        self._recording_key = False
        self._record.configure(text="Press a key…")
        return "break"

    def _microphone(self, f: ttk.Frame) -> None:
        current = self.cfg.input_device or DEFAULT_MIC
        names = [DEFAULT_MIC] + microphones()
        if current not in names:
            names.append(current)
        mic = tk.StringVar(value=current)
        self._mic = mic
        ttk.Combobox(f, textvariable=mic, values=names, state="readonly", width=44).grid(
            row=0, column=0, columnspan=2, sticky="w")
        ttk.Label(f, text="The microphone Speechify listens to.", foreground=MUTED).grid(
            row=1, column=0, columnspan=2, sticky="w")
        self._check(
            f, "keep_mic_open", "Keep the microphone open",
            "Off: the microphone is only open while you hold the talk key. On: it stays open, "
            "with 300 ms of audio from before the key went down, so the first word is never "
            "cut off, but the system shows the microphone as in use all the time.",
        )

    def _shortcuts(self, f: ttk.Frame) -> None:
        for name, label in [
            ("correct_hotkey", "Fix the last dictation"),
            ("history_hotkey", "Open the history"),
            ("models_hotkey", "Open the model picker"),
        ]:
            var = self._var(name)
            note = self._row(f, label, ttk.Entry(f, textvariable=var, width=22))
            note.configure(text=_describe_combo(var.get()))
            var.trace_add("write", lambda *a, v=var, n=note: n.configure(text=_describe_combo(v.get())))
        row = f.grid_size()[1]
        ttk.Label(
            f, foreground=MUTED, wraplength=500, justify="left",
            text="Write shortcuts like <ctrl>+<alt>+h. "
                 + ("On a Mac, <ctrl> is Control, <alt> is Option and <cmd> is Command." if MAC else
                    "<cmd> is the Windows key."),
        ).grid(row=row, column=0, columnspan=2, sticky="w", pady=(14, 0))

    def _model(self, f: ttk.Frame, open_models: Callable[[], None], model_name: Callable[[], str]) -> None:
        box = ttk.Frame(f)
        ttk.Label(box, text=model_name(), font=(FONT, 10, "bold")).pack(side="left")
        ttk.Button(box, text="Choose model…", command=open_models).pack(side="left", padx=8)
        self._row(f, "Speech model", box)

        device = ttk.Combobox(f, textvariable=self._var("device"), values=DEVICES, state="readonly", width=10)
        self._row(f, "Runs on", device, f'"auto" is {accelerator()} on this computer.')
        lang = ttk.Combobox(f, textvariable=self._var("language"), width=10,
                            values=["en", "de", "fr", "es", "it", "pt", "nl", "hi", "ja", "zh", ""])
        self._row(f, "Language", lang, "A language code. Empty detects it, which is slower and less reliable.")
        beam = ttk.Spinbox(f, from_=1, to=10, width=6, textvariable=self._var("beam_size"))
        self._row(f, "Whisper beam size", beam, "1 is about twice as fast; 5 is a little more accurate.")
        url = ttk.Entry(f, textvariable=self._var("llm_url"), width=44)
        self._row(f, "LLM cleanup URL", url, "Optional, any OpenAI-compatible endpoint, e.g. "
                                              "http://localhost:11434/v1/chat/completions. Adds latency.")
        model = ttk.Entry(f, textvariable=self._var("llm_model"), width=24)
        self._row(f, "LLM model", model)

    def _learning(self, f: ttk.Frame) -> None:
        self._check(f, "keep_all_audio", "Save every dictation's audio",
                    "About 2 MB per minute of speech. Everything saved is used to improve the model; "
                    "off keeps only the ones you corrected.")
        after = ttk.Spinbox(f, from_=1, to=20, width=6, textvariable=self._var("autocorrect_after"))
        self._row(f, "Automatic fixes after", after, "Make the same correction this many times and "
                                                     "Speechify starts applying it on its own.")
        self._check(f, "learn_auto", "Improve the speech model while I'm away",
                    "When the computer is idle and plugged in, a bigger model re-transcribes your "
                    "recordings and the dictation model is retrained on them.")
        idle = ttk.Spinbox(f, from_=1, to=120, width=6, textvariable=self._var("idle_minutes"))
        self._row(f, "Idle after (minutes)", idle)
        teacher = ttk.Entry(f, textvariable=self._var("teacher"), width=24)
        self._row(f, "Teacher model", teacher, '"auto", "off", or a model id from the model picker.')
        self._check(f, "teacher_on_cpu", "Run the teacher without an NVIDIA GPU too (slow)")

    # --- saving ----------------------------------------------------------------

    def values(self) -> dict:
        values = {name: var.get() for name, var in self.vars.items()}
        mic = self._mic.get()
        values["input_device"] = "" if mic == DEFAULT_MIC else mic
        return values

    def save(self) -> None:
        values = self.values()
        problems = settings.problems(values)
        if problems:
            self.status.configure(text="Not saved: " + "; ".join(problems) + ".")
            return
        changed = settings.changes(self.cfg, values)
        settings.save(self.cfg, changed)
        if self._autostart.get() != _autostart_enabled():
            try:
                autostart.set_enabled(self._autostart.get())
            except OSError as exc:
                self.status.configure(text=f"Couldn't change starting at sign-in: {exc}")
                return
        self.close()
        if changed:
            self.on_save(changed)

    def close(self) -> None:
        self.win.destroy()
        if self.on_close:
            self.on_close()


def _autostart_enabled() -> bool:
    try:
        return autostart.is_enabled()
    except OSError:
        return False


def _describe_combo(combo: str) -> str:
    try:
        return describe_combo(combo, MAC)
    except ValueError:
        return "Not a shortcut yet"
