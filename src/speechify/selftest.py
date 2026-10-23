"""End-to-end self-test: drives the real app the way a person would.

    speechify selftest                         full desktop test (Windows)
    speechify selftest --engine-only           load the model and transcribe, no desktop

The microphone is replaced by a known LibriSpeech clip. Everything else is
real: the hotkeys go through the OS input queue to the global keyboard hook,
the model runs in its worker process, text is pasted into Notepad, and the
fix, history and model windows are opened with their hotkeys. A screenshot is
saved after each step, and report.json / summary.md record what passed and
how long each part took.

Your data is untouched: the test uses a temporary data folder, but reuses
installed model environments so it doesn't download them again.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
import traceback
from contextlib import contextmanager
from pathlib import Path
from typing import Callable

SAMPLE = Path(__file__).with_name("selftest_data") / "librispeech-sample.wav"
REFERENCE = (
    "He hoped there would be stew for dinner, turnips and carrots and bruised potatoes "
    "and fat mutton pieces to be ladled out in thick, peppered, flour-fattened sauce."
)
MAX_WER = 0.15
FIX_WORD = "Stoo"  # what the test "corrects" a word to; no model will produce it by itself


class Report:
    def __init__(self, out: Path):
        self.out = out
        self.steps: list[dict] = []
        self.facts: dict = {}
        out.mkdir(parents=True, exist_ok=True)

    @contextmanager
    def step(self, name: str):
        entry = {"step": name, "ok": False, "detail": "", "seconds": 0.0}
        self.steps.append(entry)
        t0 = time.perf_counter()
        print(f"--- {name}", flush=True)
        try:
            yield entry
            entry["ok"] = True
        except Exception as exc:
            entry["detail"] = (entry["detail"] + f" | {type(exc).__name__}: {exc}").strip(" |")
            traceback.print_exc()
        finally:
            entry["seconds"] = round(time.perf_counter() - t0, 2)
            print(f"    {'PASS' if entry['ok'] else 'FAIL'} ({entry['seconds']}s) {entry['detail']}", flush=True)

    def screenshot(self, entry: dict, name: str) -> None:
        try:
            from PIL import ImageGrab

            path = self.out / f"{len(self.steps):02d}-{name}.png"
            ImageGrab.grab(all_screens=True).save(path)
            entry.setdefault("screenshots", []).append(path.name)
        except Exception as exc:  # screenshots are evidence, not the test
            entry.setdefault("screenshots", []).append(f"unavailable: {exc}")

    @property
    def ok(self) -> bool:
        return bool(self.steps) and all(s["ok"] for s in self.steps)

    def write(self) -> None:
        (self.out / "report.json").write_text(
            json.dumps({"ok": self.ok, "facts": self.facts, "steps": self.steps}, indent=2),
            encoding="utf-8",
        )
        lines = [f"## Speechify self-test: {'PASS' if self.ok else 'FAIL'}", ""]
        for k, v in self.facts.items():
            lines.append(f"- **{k}**: {v}")
        lines += ["", "| Step | Result | Time | Detail |", "| --- | --- | --- | --- |"]
        for s in self.steps:
            detail = s["detail"].replace("|", "\\|").replace("\n", " ")[:300]
            lines.append(f"| {s['step']} | {'✅' if s['ok'] else '❌'} | {s['seconds']}s | {detail} |")
        (self.out / "summary.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def wait_for(predicate: Callable[[], object], timeout: float, what: str, interval: float = 0.1):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(interval)
    raise TimeoutError(f"timed out after {timeout:g}s waiting for {what}")


class FileRecorder:
    """Stands in for the microphone: every 'recording' is the sample clip."""

    def __init__(self, audio):
        self.audio = audio

    def start(self) -> None:
        pass

    def stop(self):
        return self.audio.copy()

    def close(self) -> None:
        pass


# --- Win32 helpers ---------------------------------------------------------------


def _win32():
    import ctypes
    from ctypes import wintypes

    user32 = ctypes.windll.user32
    proc = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

    def windows() -> list[tuple[int, str]]:
        found = []

        def callback(hwnd, _):
            if user32.IsWindowVisible(hwnd):
                n = user32.GetWindowTextLengthW(hwnd)
                buf = ctypes.create_unicode_buffer(n + 1)
                user32.GetWindowTextW(hwnd, buf, n + 1)
                if buf.value:
                    found.append((hwnd, buf.value))
            return True

        user32.EnumWindows(proc(callback), 0)
        return found

    def title(hwnd: int) -> str:
        n = user32.GetWindowTextLengthW(hwnd)
        buf = ctypes.create_unicode_buffer(n + 1)
        user32.GetWindowTextW(hwnd, buf, n + 1)
        return buf.value

    kernel32 = ctypes.windll.kernel32

    def focus(hwnd: int) -> None:
        """Bring a window to the front the way the app does (see ui.bring_to_front)."""
        if user32.GetForegroundWindow() == hwnd:
            return
        fg_thread = user32.GetWindowThreadProcessId(user32.GetForegroundWindow(), None)
        our_thread = kernel32.GetCurrentThreadId()
        attached = fg_thread and fg_thread != our_thread and user32.AttachThreadInput(our_thread, fg_thread, True)
        try:
            user32.ShowWindow(hwnd, 9)  # SW_RESTORE
            user32.BringWindowToTop(hwnd)
            user32.SetForegroundWindow(hwnd)
        finally:
            if attached:
                user32.AttachThreadInput(our_thread, fg_thread, False)

    def edit_control(hwnd: int) -> int:
        edit = None

        def callback(child, _):
            nonlocal edit
            cls = ctypes.create_unicode_buffer(256)
            user32.GetClassNameW(child, cls, 256)
            if "edit" in cls.value.lower():  # "Edit" (classic) or "RichEditD2DPT" (new Notepad)
                edit = child
                return False
            return True

        user32.EnumChildWindows(hwnd, proc(callback), 0)
        if edit is None:
            raise LookupError("no edit control in the window")
        return edit

    user32.SendMessageW.restype = ctypes.c_ssize_t
    user32.SendMessageW.argtypes = (wintypes.HWND, wintypes.UINT, wintypes.WPARAM, ctypes.c_void_p)

    def read_text(hwnd: int) -> str:
        """Text of the window's edit control, read with WM_GETTEXT.

        Reading it this way rather than with Ctrl+A / Ctrl+C keeps the check
        independent of the keyboard path being tested.
        """
        edit = edit_control(hwnd)
        n = user32.SendMessageW(edit, 0x000E, 0, None)  # WM_GETTEXTLENGTH
        buf = ctypes.create_unicode_buffer(n + 1)
        user32.SendMessageW(edit, 0x000D, n + 1, ctypes.addressof(buf))  # WM_GETTEXT
        return buf.value

    def clear_text(hwnd: int) -> None:
        empty = ctypes.create_unicode_buffer("")
        user32.SendMessageW(edit_control(hwnd), 0x000C, 0, ctypes.addressof(empty))  # WM_SETTEXT

    def wm_paste(hwnd: int) -> None:
        user32.SendMessageW(edit_control(hwnd), 0x0302, 0, None)  # WM_PASTE

    return windows, title, focus, user32.GetForegroundWindow, read_text, clear_text, wm_paste


# --- tests --------------------------------------------------------------------------


def run_engine_only(model: str, device: str, report: Report) -> None:
    from .audio import load_wav
    from .config import Config, data_dir
    from .engines import host
    from .engines.catalog import resolve
    from .metrics import wer

    cfg = Config.load()
    if device:
        cfg.device = device
    if cfg.device == "cpu":
        cfg.compute_type = "int8"
    spec = resolve(model or cfg.model, data_dir())
    report.facts.update(model=spec.id, backend=spec.backend)
    audio = load_wav(SAMPLE)
    engine = host.EngineHost()
    with report.step(f"install {spec.id}") as s:
        t0 = time.perf_counter()
        host.install(spec, cfg.engine_opts())
        s["detail"] = f"{time.perf_counter() - t0:.0f}s (0s if already installed)"
    if not host.is_ready(spec):
        return
    with report.step("load model") as s:
        info = engine.start(spec, cfg.engine_opts())
        report.facts.update(device=info.get("device"), load_seconds=info.get("load_s"))
        s["detail"] = f"device {info.get('device')}, {info.get('load_s')}s"
    if not engine.ready.is_set():
        return  # the failed step above says why; transcribing would only time out
    try:
        for attempt in ("first", "warm"):
            with report.step(f"transcribe ({attempt})") as s:
                text, ms = engine.transcribe(audio, None)
                score = wer([REFERENCE], [text])
                report.facts[f"{attempt}_transcribe_ms"] = ms
                report.facts["wer"] = f"{score:.1%}"
                s["detail"] = f"{ms} ms for {audio.size / 16000:.1f}s of audio, WER {score:.1%}: {text}"
                assert score <= MAX_WER, f"WER {score:.1%} is above {MAX_WER:.0%}"
    finally:
        engine.stop()


def run_desktop(model: str, report: Report) -> None:
    if sys.platform != "win32":
        raise SystemExit("The desktop self-test drives Windows apps; use --engine-only elsewhere.")
    import pyperclip
    from pynput.keyboard import Controller, Key, KeyCode

    from . import config, output
    from .app import App
    from .audio import load_wav
    from .config import Config
    from .engines.catalog import resolve
    from .metrics import wer

    windows, title, focus, foreground, read_text, clear_text, wm_paste = _win32()
    kb = Controller()

    config.set_value("autocorrect_after", 1)  # one fix is enough for the rule to apply
    cfg = Config.load()
    cfg.learn_auto = False  # no teacher downloads or training in the middle of a test
    if model:
        cfg.model = model
    spec = resolve(cfg.model, config.data_dir())
    report.facts.update(model=spec.id, backend=spec.backend)

    pasted: list[str] = []
    real_paste = output.paste

    def recording_paste(text: str) -> None:
        pasted.append(text)
        real_paste(text)

    output.paste = recording_paste
    app = App(cfg, recorder=FileRecorder(load_wav(SAMPLE)))
    ui = app.ui

    def on_ui(fn: Callable[[], object], timeout: float = 10):
        """Run fn on the Tk thread and return its result."""
        box: list = []
        done = threading.Event()

        def run() -> None:
            try:
                box.append(fn())
            except Exception as exc:
                box.append(exc)
            done.set()

        ui.post(run)
        if not done.wait(timeout):
            raise TimeoutError("UI thread didn't respond")
        if isinstance(box[0], Exception):
            raise box[0]
        return box[0]

    from . import winkeys

    vks = {Key.ctrl: 0x11, Key.alt: 0x12, Key.shift: 0x10, Key.enter: 0x0D}

    def hotkey(*keys) -> None:
        """Press a shortcut like a physical keyboard: virtual keys plus scan codes."""
        winkeys.chord(*(vks[k] if k in vks else ord(k.upper()) for k in keys))

    def pynput_hotkey(*keys) -> None:
        keys = [KeyCode.from_vk(ord(k.upper())) if isinstance(k, str) else k for k in keys]
        for k in keys:
            kb.press(k)
        time.sleep(0.05)
        for k in reversed(keys):
            kb.release(k)

    def dictate(hold: float = 1.0, during: Callable[[], None] | None = None) -> None:
        kb.press(Key.ctrl_r)
        time.sleep(hold / 2)
        if during:
            during()
        time.sleep(hold / 2)
        kb.release(Key.ctrl_r)

    def read_notepad(hwnd: int) -> str:
        return read_text(hwnd).replace("\r\n", "\n").strip()

    def drive() -> None:
        try:
            with report.step("model installs and loads") as s:
                wait_for(lambda: app.engine.ready.is_set(), 1800, "the model to load", 1)
                info = app.engine.info
                report.facts.update(device=info.get("device"), load_seconds=info.get("load_s"))
                s["detail"] = f"{spec.name} on {info.get('device')} in {info.get('load_s')}s"

            with report.step("Notepad opens") as s:
                subprocess.Popen(["notepad.exe"])
                hwnd = wait_for(
                    lambda: next((h for h, t in windows() if "Notepad" in t), None), 30, "Notepad"
                )
                focus(hwnd)
                wait_for(lambda: foreground() == hwnd, 10, "Notepad to be in front")
                s["detail"] = title(hwnd)
                report.screenshot(s, "notepad")

            with report.step("input probe: which ways of entering text reach Notepad") as s:
                # Diagnostic only: it records what works so a paste failure is
                # explained by the report rather than guessed at.
                def attempt(name: str, action: Callable[[], None], expect: str) -> str:
                    try:
                        clear_text(hwnd)
                        focus(hwnd)
                        time.sleep(0.2)
                        action()
                        time.sleep(0.6)
                        got = read_text(hwnd)
                        return f"{name}={'ok' if got == expect else repr(got)}"
                    except Exception as exc:
                        return f"{name}=error({exc})"

                def with_clipboard(fn):
                    def run():
                        pyperclip.copy("probe")
                        fn()
                    return run

                results = [
                    attempt("type", lambda: kb.type("probe"), "probe"),
                    attempt("pynput-ctrl-v", with_clipboard(lambda: pynput_hotkey(Key.ctrl, "v")), "probe"),
                    attempt("sendinput-ctrl-v", with_clipboard(winkeys.ctrl_v), "probe"),
                    attempt("wm-paste", with_clipboard(lambda: wm_paste(hwnd)), "probe"),
                ]
                pyperclip.copy("clipboard-check")
                results.append(f"clipboard={'ok' if pyperclip.paste() == 'clipboard-check' else 'broken'}")
                clear_text(hwnd)
                focus(hwnd)
                s["detail"] = ", ".join(results)
                report.facts["input_probe"] = s["detail"]

            with report.step("hold-to-talk pastes the transcription into Notepad") as s:
                focus_kept: list[bool] = []

                def mid_hold() -> None:
                    time.sleep(0.2)  # let the overlay appear
                    focus_kept.append(foreground() == hwnd)
                    report.screenshot(s, "listening-overlay")

                t0 = time.perf_counter()
                dictate(1.2, mid_hold)
                wait_for(lambda: len(pasted) >= 1, 600, "the first paste")
                latency = time.perf_counter() - t0 - 1.2
                time.sleep(1.0)  # let Notepad process the paste
                text = read_notepad(hwnd)
                if not text:
                    s["detail"] = f"foreground: {title(foreground())!r}; "
                score = wer([REFERENCE], [text])
                report.facts.update(release_to_paste_seconds=round(latency, 2), wer=f"{score:.1%}")
                s["detail"] += f"{latency:.2f}s after release, WER {score:.1%}: {text}"
                report.screenshot(s, "pasted")
                assert text == pasted[0].strip(), f"Notepad has {text!r}, app pasted {pasted[0]!r}"
                assert score <= MAX_WER, f"WER {score:.1%} is above {MAX_WER:.0%}"

            with report.step("overlay never takes focus from the app being dictated into") as s:
                assert focus_kept and all(focus_kept), "the overlay became the foreground window"
                s["detail"] = "Notepad stayed in front while the overlay showed"

            first = text
            words = re.findall(r"[A-Za-z]+", first)
            target = next((w for w in words if w.lower() == "stew"), max(words, key=len))
            corrected = re.sub(rf"\b{re.escape(target)}\b", FIX_WORD, first, count=1)

            with report.step("Ctrl+Alt+F opens the fix window with the last dictation") as s:
                hotkey(Key.ctrl, Key.alt, "f")
                wait_for(lambda: on_ui(lambda: ui._correction is not None), 10, "the fix window")
                time.sleep(0.5)
                shown = on_ui(lambda: _text_of(ui._correction))
                assert shown == first, f"fix window shows {shown!r}"
                report.screenshot(s, "fix-window")

            with report.step("pasting a fix into the window replaces the selected text") as s:
                # Dictating a fix ends in exactly this paste, so this covers it too.
                pyperclip.copy(corrected)
                hotkey(Key.ctrl, "v")
                time.sleep(0.5)
                shown = on_ui(lambda: _text_of(ui._correction))
                assert shown == corrected, f"fix window has {shown!r}"
                s["detail"] = f"{target} -> {FIX_WORD}"
                report.screenshot(s, "fix-typed")

            with report.step("Ctrl+Enter learns the fix and replaces the text in Notepad") as s:
                hotkey(Key.ctrl, Key.enter)
                wait_for(lambda: any(r[1] == FIX_WORD for r in app.learner.rules()), 10, "the rule")
                time.sleep(len(first) * 0.01 + 2)  # Shift+Left over the old text, then paste
                now = read_notepad(hwnd)
                s["detail"] = f"rules: {app.learner.rules()}"
                report.screenshot(s, "replaced")
                assert now == corrected, f"Notepad has {now!r}"

            with report.step("the learned fix is applied to the next dictation") as s:
                # Wait for a new dictation, not just any paste: replacing the
                # text in Notepad above was a paste too, and contains the fix.
                before = app.last
                dictate(1.0)
                wait_for(lambda: app.last is not before, 600, "the second dictation")
                second = app.last.pasted
                s["detail"] = second
                assert FIX_WORD in second, f"second dictation was {second!r}"
                assert FIX_WORD in app.learner.prompt(), "fix missing from the vocabulary hint"
                wait_for(lambda: app.learner.stats()["samples"] >= 2, 10, "the second recording to be saved")
                time.sleep(1.0)
                report.screenshot(s, "second-dictation")

            with report.step("Ctrl+Alt+H lists both recordings, the first marked edited") as s:
                hotkey(Key.ctrl, Key.alt, "h")
                wait_for(lambda: on_ui(lambda: ui._history is not None), 10, "the history window")
                time.sleep(0.5)
                rows = on_ui(lambda: [ui._history.tree.item(i, "values") for i in ui._history.tree.get_children()])
                s["detail"] = f"{len(rows)} rows: {[r[2] for r in rows]}"
                report.screenshot(s, "history")
                assert len(rows) == 2, f"expected 2 recordings, got {len(rows)}"
                assert rows[1][2] == "edited" and FIX_WORD in rows[1][3], f"first recording: {rows[1]}"
                on_ui(lambda: ui._history.close())

            with report.step("Ctrl+Alt+M lists every model with the active one marked") as s:
                hotkey(Key.ctrl, Key.alt, "m")
                wait_for(lambda: on_ui(lambda: ui._models is not None), 10, "the model window")
                time.sleep(0.5)
                rows = on_ui(lambda: {i: ui._models.tree.item(i, "values") for i in ui._models.tree.get_children()})
                s["detail"] = f"{len(rows)} models, {spec.id}: {rows.get(spec.id, ('?',) * 8)[7]}"
                report.screenshot(s, "models")
                assert len(rows) >= 14, f"only {len(rows)} models listed"
                assert rows[spec.id][7] == "● in use", f"{spec.id} shows {rows[spec.id][7]!r}"
                on_ui(lambda: ui._models.close())
        except Exception:
            traceback.print_exc()
        finally:
            try:
                subprocess.run(["taskkill", "/IM", "notepad.exe", "/F"], capture_output=True)
            finally:
                ui.post(ui.quit)

    threading.Thread(target=drive, daemon=True).start()
    app.run()


def _text_of(window) -> str:
    import tkinter as tk

    for child in window.winfo_children():
        if isinstance(child, tk.Text):
            return child.get("1.0", "end").strip()
    raise LookupError("no text box in the window")


def main(model: str | None, engine_only: bool, device: str | None, out: str | None) -> int:
    from . import __version__
    from .config import data_dir

    out_dir = Path(out or tempfile.mkdtemp(prefix="speechify-selftest-"))
    # Keep the test's recordings and config out of the real data folder, but
    # reuse the model environments that are already installed.
    os.environ.setdefault("SPEECHIFY_ENGINES", str(data_dir() / "engines"))
    os.environ["SPEECHIFY_HOME"] = tempfile.mkdtemp(prefix="speechify-selftest-home-")

    report = Report(out_dir)
    report.facts.update(version=__version__, platform=sys.platform)
    t0 = time.perf_counter()
    try:
        if engine_only:
            run_engine_only(model, device, report)
        else:
            run_desktop(model, report)
    finally:
        report.facts["total_seconds"] = round(time.perf_counter() - t0, 1)
        report.write()
    print(f"\nSelf-test {'PASSED' if report.ok else 'FAILED'}. Report: {out_dir}")
    return 0 if report.ok else 1
