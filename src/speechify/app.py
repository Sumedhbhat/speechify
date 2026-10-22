"""Wires the recorder, model, hotkeys and UI together."""

from __future__ import annotations

import queue
import sys
import threading
import time
import traceback
from typing import Callable

from . import __version__, config, keys, output, postprocess
from .audio import Recorder, is_silent
from .config import Config, data_dir
from .engines import host
from .engines.catalog import ModelSpec, resolve
from .hotkeys import PushToTalk, global_hotkeys
from .learning import Dictation, Learner
from .ui import UI


class App:
    def __init__(self, cfg: Config, recorder=None):
        """`recorder` replaces the microphone (the self-test feeds a WAV file)."""
        self.cfg = cfg
        self.ui = UI()
        self.learner = Learner(data_dir(), cfg.autocorrect_after)
        self.recorder = recorder or Recorder(cfg.input_device or None)
        self.engine = host.EngineHost()
        self._switch_lock = threading.Lock()
        self.last: Dictation | None = None
        # One worker, so dictations are pasted in the order they were spoken.
        self._jobs: queue.Queue[Callable[[], None]] = queue.Queue()
        threading.Thread(target=self._work, daemon=True).start()

        self.ptt = PushToTalk(
            cfg.ptt_key, cfg.min_hold_ms, self._on_start, self._on_stop, self._on_cancel
        )
        self.hotkeys = global_hotkeys({
            cfg.correct_hotkey: self._on_fix,
            cfg.history_hotkey: lambda: self.ui.post(lambda: self.ui.open_history(self.learner)),
            cfg.models_hotkey: lambda: self.ui.post(self._open_models),
        })

    def run(self) -> None:
        problem = keys.permission_problem()
        try:
            self.ptt.start()
            self.hotkeys.start()
            output.prepare()
        except OSError as exc:  # PermissionError, or no usable /dev/uinput
            if isinstance(exc, PermissionError) and keys.backend() == "evdev":
                from . import linuxsetup

                linuxsetup.offer_wayland_access(str(exc))  # restarts Speechify if it worked
            self._fail(problem or str(exc))
        if problem:  # listening works but typing doesn't yet
            print(problem)
            self.ui.show("error", "Speechify can't paste yet: see System Settings", 8000)
        if sys.platform.startswith("linux"):
            from . import linuxsetup

            linuxsetup.register_appimage()
            missing = linuxsetup.clipboard_problem()
            if missing:
                from tkinter import messagebox

                print(missing)
                messagebox.showwarning("Speechify", missing)
        stats = self.learner.stats()
        print(
            f"Speechify {__version__}. Hold [{self.cfg.ptt_key}] to talk, "
            f"{self.cfg.correct_hotkey} to fix the last dictation, {self.cfg.history_hotkey} "
            f"for all recordings, {self.cfg.models_hotkey} to switch models. "
            f"{stats['samples']} recordings saved, {stats['active_rules']} active rules, "
            f"{stats['vocab']} vocabulary terms."
        )
        try:
            spec = resolve(self.cfg.model, data_dir())
        except KeyError as exc:
            print(f"{exc}; using the default model")
            spec = resolve(config.platform_defaults()["model"], data_dir())
        self.switch_model(spec)
        try:
            self.ui.run()
        except KeyboardInterrupt:
            pass
        finally:
            self.ptt.stop()
            self.hotkeys.stop()
            self.engine.stop()
            self.recorder.close()

    def _fail(self, message: str) -> None:
        """Explain a setup problem, in a dialog too: started from the app menu
        or the Dock there's no console to read."""
        from tkinter import messagebox

        print(message)
        messagebox.showerror("Speechify", message)
        self.recorder.close()
        raise SystemExit(1)

    # --- models ---------------------------------------------------------------

    def _log(self, line: str) -> None:
        print(line)
        self.ui.post(lambda: self.ui.model_log(line))

    def install_model(self, spec: ModelSpec) -> None:
        """Download a model in the background without switching to it."""

        def run() -> None:
            try:
                host.install(spec, self.cfg.engine_opts(), self._log)
            except Exception as exc:
                self._log(f"Install failed: {exc}")
            self.ui.post(self.ui.refresh_models)

        threading.Thread(target=run, daemon=True).start()

    def switch_model(self, spec: ModelSpec) -> None:
        """Install if needed, stop the current model, load the new one."""

        current = self.engine.spec
        if current is not None and current.id == spec.id and self.engine.ready.is_set():
            self.ui.post(lambda: self.ui.show("ok", f"Already using {spec.name}", 1500))
            return

        def run() -> None:
            with self._switch_lock:
                try:
                    if not host.is_ready(spec):
                        self.ui.post(lambda: self.ui.show("working", f"Installing {spec.name}…"))
                        host.install(spec, self.cfg.engine_opts(), self._log)
                    self.ui.post(lambda: self.ui.show("working", f"Loading {spec.name}…"))
                    self._log(f"Loading {spec.name}...")
                    info = self.engine.start(spec, self.cfg.engine_opts())
                except Exception as exc:
                    self._log(str(exc))
                    self.ui.post(lambda: self.ui.show("error", f"{spec.name} failed to load", 4000))
                    self.ui.post(self.ui.refresh_models)
                    return
                # Fine-tuned models are saved by path, catalog models by id.
                saved = spec.repo if spec.id.startswith("ft:") else spec.id
                if self.cfg.model != saved:
                    self.cfg.model = saved
                    config.set_value("model", saved)
                msg = f"{spec.name} ready on {info.get('device')} ({info.get('load_s')}s)"
                self._log(msg)
                self.ui.post(lambda: self.ui.show("ok", msg, 2000))
                self.ui.post(self.ui.refresh_models)

        threading.Thread(target=run, daemon=True).start()

    def _open_models(self) -> None:
        self.ui.open_models(
            active=lambda: self.engine.spec.id if self.engine.spec else None,
            use=self.switch_model,
            install=self.install_model,
        )

    def _work(self) -> None:
        while True:
            job = self._jobs.get()
            try:
                job()
            except Exception:
                traceback.print_exc()
                self.ui.post(lambda: self.ui.show("error", "Error, see console", 2500))

    # --- push to talk (called on the keyboard hook thread; keep it fast) ----

    def _on_start(self) -> None:
        self.recorder.start()
        self.ui.post(lambda: self.ui.show("listening", "Listening…"))

    def _on_cancel(self) -> None:
        self.recorder.stop()
        self.ui.post(self.ui.hide)

    def _on_stop(self) -> None:
        audio = self.recorder.stop()
        self.ui.post(lambda: self.ui.show("working", "Transcribing…"))
        self._jobs.put(lambda: self._dictate(audio))

    def _dictate(self, audio) -> None:
        if is_silent(audio):
            self.ui.post(lambda: self.ui.show("info", "No speech heard", 1000))
            return
        if not self.engine.ready.is_set():
            self.ui.post(lambda: self.ui.show("working", "Waiting for the model to load…"))
        t0 = time.perf_counter()
        spec = self.engine.spec
        prompt = self.learner.prompt() if spec is None or spec.prompt else None
        raw, model_ms = self.engine.transcribe(audio, prompt)
        if not raw:
            self.ui.post(lambda: self.ui.show("info", "No speech heard", 1000))
            return
        text = self.learner.apply(raw)
        if self.cfg.llm_url and self.cfg.llm_model:
            text = postprocess.clean(text, self.cfg.llm_url, self.cfg.llm_model, self.learner.prompt())
        output.paste(text)
        elapsed = time.perf_counter() - t0
        print(f"[{elapsed * 1000:.0f} ms total, {model_ms} ms model, {audio.size / 16000:.1f}s audio] {text}")

        # Dictating the fix into the correction window shouldn't replace the
        # dictation being corrected.
        if not self.ui.correcting:
            self.last = Dictation(audio=audio, raw=raw, pasted=text)
            self.learner.record(self.last, self.cfg.keep_all_audio)
        self.ui.post(lambda: self.ui.show("ok", f"{elapsed:.2f}s", 600))

    # --- corrections ----------------------------------------------------------

    def _on_fix(self) -> None:
        last = self.last
        if last is None:
            self.ui.post(lambda: self.ui.show("info", "Nothing to fix yet", 1200))
            return

        def save(corrected: str, replace: bool) -> None:
            self._jobs.put(lambda: self._save_fix(last, corrected, replace))

        self.ui.post(lambda: self.ui.ask_correction(last.raw, last.pasted, save))

    def _save_fix(self, d: Dictation, corrected: str, replace: bool) -> None:
        if not corrected or corrected == d.pasted:
            return
        edits = self.learner.correct(d, corrected)
        if replace:
            output.replace_last(d.pasted, corrected)
        d.pasted = corrected  # a second fix diffs against this one
        summary = ", ".join(f"{w} → {r}" for w, r in edits[:3]) or "saved for training"
        print(f"Learned: {summary}")
        self.ui.post(lambda: self.ui.show("ok", f"Learned: {summary}", 2000))
