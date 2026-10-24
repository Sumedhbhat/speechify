"""Wires the recorder, model, hotkeys and UI together."""

from __future__ import annotations

import queue
import sys
import threading
import time
import traceback
from typing import Callable

from . import __version__, applog, config, keys, output, postprocess, settings, versions
from .autolearn import AutoLearner
from .audio import Recorder, is_silent
from .combos import describe_combo, describe_key
from .config import Config, data_dir
from .engines import host
from .engines.catalog import ModelSpec, resolve
from .hotkeys import PushToTalk, global_hotkeys
from .learning import Dictation, Learner
from .tray import Tray
from .ui import UI

MAC = sys.platform == "darwin"


class App:
    def __init__(self, cfg: Config, recorder=None, welcome: bool = False):
        """`recorder` replaces the microphone (the self-test feeds a WAV file).
        `welcome` opens the settings window with a short introduction, for the
        first start."""
        self.cfg = cfg
        self.welcome = welcome
        self._own_recorder = recorder is None
        self.ui = UI()
        self.learner = Learner(data_dir(), cfg.autocorrect_after)
        self.recorder = recorder or Recorder(
            cfg.input_device or None, cfg.keep_mic_open, on_error=self._mic_failed
        )
        self.ui.meter = self.recorder  # the overlay's level bars read it
        self.engine = host.EngineHost()
        self._switch_lock = threading.Lock()
        # Teacher labels and retraining while you're away (autolearn.py).
        self.autolearn = AutoLearner(
            cfg, self.learner, self.engine, student_lock=self._switch_lock, log=self._log,
            on_switch=lambda spec: self.ui.post(self.ui.refresh_models),
        )
        self.last: Dictation | None = None
        # One worker, so dictations are pasted in the order they were spoken.
        self._jobs: queue.Queue[Callable[[], None]] = queue.Queue()
        threading.Thread(target=self._work, daemon=True).start()

        self.ptt = PushToTalk(
            cfg.ptt_key, cfg.min_hold_ms, self._on_start, self._on_stop, self._on_cancel
        )
        self.hotkeys = global_hotkeys(self._hotkey_bindings())
        self._loading: str | None = None  # the model being loaded, for the tray
        self.tray = Tray(self._status_line, self._hint, [
            ("Settings…", lambda: self.ui.post(self.open_settings)),
            (lambda: self._menu_label("History…", self.cfg.history_hotkey), self.open_history),
            (lambda: self._menu_label("Models…", self.cfg.models_hotkey), lambda: self.ui.post(self._open_models)),
            (lambda: self._menu_label("Fix last dictation", self.cfg.correct_hotkey), self._on_fix),
            None,
            ("Open log", lambda: self._open(applog.log_path())),
            ("Open data folder", lambda: self._open(data_dir())),
            None,
            ("Quit Speechify", self.quit),
        ])

    def _hotkey_bindings(self) -> dict[str, Callable[[], None]]:
        return {
            self.cfg.correct_hotkey: self._on_fix,
            self.cfg.history_hotkey: self.open_history,
            self.cfg.models_hotkey: lambda: self.ui.post(self._open_models),
        }

    # --- tray and settings ------------------------------------------------------

    @staticmethod
    def _menu_label(label: str, hotkey: str) -> str:
        try:
            return f"{label}    {describe_combo(hotkey, MAC)}"
        except ValueError:
            return label

    def _status_line(self) -> str:
        spec = self.engine.spec
        if self._loading:
            return f"Loading {self._loading}…"
        if spec is not None and self.engine.ready.is_set():
            return f"{spec.name} on {self.engine.info.get('device') or '?'}"
        return "Starting…"

    def _hint(self) -> str:
        return f"Hold {describe_key(self.cfg.ptt_key, MAC)} to talk"

    def _open(self, path) -> None:
        if path.exists():
            applog.open_in_os(path)
        else:
            self.ui.post(lambda: self.ui.show("info", "No log yet: started from a terminal, it prints there", 3000))

    def open_history(self) -> None:
        self.ui.post(lambda: self.ui.open_history(self.learner))

    def open_settings(self, welcome: bool = False) -> None:
        """Open the settings window (on the UI thread)."""
        def model_name() -> str:
            spec = self.engine.spec
            return spec.name if spec else self._loading or self.cfg.model

        self.ui.open_settings(self.cfg, self.apply_settings, self._open_models, model_name, welcome)

    def apply_settings(self, changed: dict) -> None:
        """Put saved settings into effect without a restart (on the UI thread)."""
        print(f"Settings changed: {', '.join(sorted(changed))}")
        if {"ptt_key", "min_hold_ms"} & changed.keys():
            self.ptt.key = keys.check_key_name(self.cfg.ptt_key)
            self.ptt.min_hold = self.cfg.min_hold_ms / 1000
        if {"input_device", "keep_mic_open"} & changed.keys() and self._own_recorder:
            try:
                recorder = Recorder(self.cfg.input_device or None, self.cfg.keep_mic_open,
                                    on_error=self._mic_failed)
            except Exception as exc:
                self._mic_failed(str(exc))
            else:
                old, self.recorder = self.recorder, recorder
                self.ui.meter = recorder
                old.close()
        if set(settings.HOTKEYS) & changed.keys():
            self.hotkeys.rebind(self._hotkey_bindings())
        if "autocorrect_after" in changed:
            self.learner.autocorrect_after = self.cfg.autocorrect_after
        if settings.RELOADS_MODEL & changed.keys() and self.engine.spec is not None:
            self.switch_model(self.engine.spec, reload=True)
        else:
            self.ui.show("ok", "Settings saved", 1500)
        self.tray.refresh()

    def quit(self) -> None:
        self.ui.post(self.ui.quit)

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
        if not self.tray.start() and not self.welcome:
            self.ui.show("info", "Speechify is running. Start it again to open its settings", 5000)
        if self.welcome:
            self.open_settings(welcome=True)
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
        self.autolearn.start()
        try:
            self.ui.run()
        except KeyboardInterrupt:
            pass
        finally:
            self.autolearn.stop()
            self.tray.stop()
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

    def switch_model(self, spec: ModelSpec, reload: bool = False) -> None:
        """Install if needed, stop the current model, load the new one.
        `reload` loads it again even if it's the current one (new settings)."""

        current = self.engine.spec
        if not reload and current is not None and current.id == spec.id and self.engine.ready.is_set():
            self.ui.post(lambda: self.ui.show("ok", f"Already using {spec.name}", 1500))
            return

        def run() -> None:
            # A teacher or training run may be using the GPU the new model needs.
            self.autolearn.preempt("model switch", restore=False)
            with self._switch_lock:
                self._loading = spec.name
                self.ui.post(self.tray.refresh)
                try:
                    if not host.is_ready(spec, self.cfg.resolved_device()):
                        self.ui.post(lambda: self.ui.show("working", f"Installing {spec.name}…"))
                        host.install(spec, self.cfg.engine_opts(), self._log)
                    self.ui.post(lambda: self.ui.show("working", f"Loading {spec.name}…"))
                    self._log(f"Loading {spec.name}...")
                    info = self.engine.start(spec, self.cfg.engine_opts())
                except Exception as exc:
                    self._loading = None
                    self._log(str(exc))
                    self.ui.post(lambda: self.ui.show("error", f"{spec.name} failed to load", 4000))
                    self.ui.post(self.ui.refresh_models)
                    return
                # Fine-tuned models are saved by path, catalog models by id.
                saved = spec.repo if spec.id.startswith("ft:") else spec.id
                if self.cfg.model != saved:
                    self.cfg.model = saved
                    config.set_value("model", saved)
                self._loading = None
                self.ui.post(self.tray.refresh)
                msg = f"{spec.name} ready on {info.get('device')} ({info.get('load_s')}s)"
                self._log(msg)
                self.ui.post(lambda: self.ui.show("ok", msg, 2000))
                self.ui.post(self.ui.refresh_models)

        threading.Thread(target=run, daemon=True).start()

    def _open_models(self) -> None:
        self.ui.open_models(
            active=self._active_model,
            use=self.switch_model,
            install=self.install_model,
            rollback=self.rollback,
            device=self.cfg.resolved_device(),
        )

    def _active_model(self) -> tuple[str, str] | None:
        """(model id, the device it's actually running on), for the model window."""
        spec = self.engine.spec
        return (spec.id, self.engine.info.get("device") or "?") if spec else None

    def rollback(self) -> None:
        """Go back one retrained version, and switch to it now."""
        try:
            target, description = versions.rollback(self.learner, self.cfg)
            spec = resolve(target, data_dir())
        except (versions.RollbackError, KeyError) as exc:
            self._log(str(exc))
            self.ui.post(lambda: self.ui.show("info", "Nothing to roll back", 2000))
            return
        self._log(f"Rolling back to {description}.")
        self.switch_model(spec)

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
        # Stops a background job that would slow dictation down, and gets
        # the dictation model loading again if it had been unloaded for it.
        self.autolearn.key_down()
        self.ui.post(lambda: (self.ui.show("listening", "Listening…"), self.tray.set_listening(True)))

    def _mic_failed(self, error: str) -> None:
        print(f"Can't open the microphone: {error}")
        self.ui.post(lambda: self.ui.show("error", "Can't open the microphone", 4000))

    def _on_cancel(self) -> None:
        self.recorder.stop()
        self.autolearn.dictation_done()
        self.ui.post(lambda: (self.ui.hide(), self.tray.set_listening(False)))

    def _on_stop(self) -> None:
        audio = self.recorder.stop()
        self.ui.post(lambda: self.tray.set_listening(False))
        if getattr(self.recorder, "error", None):  # _mic_failed already said so
            self.autolearn.dictation_done()
            return
        self.ui.post(lambda: self.ui.show("working", "Transcribing…"))
        self._jobs.put(lambda: self._dictate_and_finish(audio))

    def _dictate_and_finish(self, audio) -> None:
        try:
            self._dictate(audio)
        finally:
            self.autolearn.dictation_done()

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
