"""Status overlay and correction window (Tk, main thread only).

Other threads must go through UI.post(); Tk isn't thread-safe.
"""

from __future__ import annotations

import queue
import sys
import tkinter as tk
from typing import Callable

COLORS = {
    "listening": "#e5484d",
    "working": "#f5a524",
    "ok": "#30a46c",
    "info": "#8b8d98",
    "error": "#e5484d",
}


# --- Win32 focus handling ----------------------------------------------------
# The overlay must never take focus, or Ctrl+V would paste into it instead of
# the app you're dictating into.

if sys.platform == "win32":
    import ctypes

    _user32 = ctypes.windll.user32
    _kernel32 = ctypes.windll.kernel32
    _GWL_EXSTYLE = -20
    _WS_EX_LAYERED = 0x00080000
    _WS_EX_TRANSPARENT = 0x00000020
    _WS_EX_TOOLWINDOW = 0x00000080
    _WS_EX_NOACTIVATE = 0x08000000

    def _make_passive(widget: tk.Misc) -> None:
        hwnd = _user32.GetParent(widget.winfo_id())
        style = _user32.GetWindowLongW(hwnd, _GWL_EXSTYLE)
        style |= _WS_EX_LAYERED | _WS_EX_TRANSPARENT | _WS_EX_TOOLWINDOW | _WS_EX_NOACTIVATE
        _user32.SetWindowLongW(hwnd, _GWL_EXSTYLE, style)

    def foreground_window() -> int:
        return _user32.GetForegroundWindow()

    def focus_window(hwnd: int) -> None:
        if hwnd:
            _user32.SetForegroundWindow(hwnd)

    def bring_to_front(win: tk.Misc) -> None:
        """Make one of our windows the foreground window, although a hotkey
        rather than a click on us opened it.

        Windows refuses SetForegroundWindow from a background app. Attaching
        our input queue to the foreground app's thread for a moment lifts that
        restriction without sending any keys (a synthetic Alt press would
        toggle the target app's menu bar and swallow the next shortcut).
        """
        win.update_idletasks()
        hwnd = _user32.GetParent(win.winfo_id()) or win.winfo_id()
        fg_thread = _user32.GetWindowThreadProcessId(_user32.GetForegroundWindow(), None)
        our_thread = _kernel32.GetCurrentThreadId()
        attached = fg_thread and fg_thread != our_thread and _user32.AttachThreadInput(our_thread, fg_thread, True)
        try:
            _user32.ShowWindow(hwnd, 5)  # SW_SHOW
            _user32.BringWindowToTop(hwnd)
            _user32.SetForegroundWindow(hwnd)
        finally:
            if attached:
                _user32.AttachThreadInput(our_thread, fg_thread, False)
        win.focus_force()

else:

    def _make_passive(widget: tk.Misc) -> None:
        pass

    def foreground_window() -> int:
        return 0

    def focus_window(hwnd: int) -> None:
        pass

    def bring_to_front(win: tk.Misc) -> None:
        win.lift()
        win.focus_force()


class UI:
    def __init__(self):
        self.root = tk.Tk()
        self.root.withdraw()
        self._queue: queue.Queue[Callable[[], None]] = queue.Queue()
        self._hide_job: str | None = None
        self._correction: tk.Toplevel | None = None
        self._history = None
        self._models = None
        self._build_pill()
        self.root.after(16, self._drain)

    # --- threading ----------------------------------------------------------

    def post(self, fn: Callable[[], None]) -> None:
        self._queue.put(fn)

    def _drain(self) -> None:
        while True:
            try:
                self._queue.get_nowait()()
            except queue.Empty:
                break
        self.root.after(16, self._drain)

    def run(self) -> None:
        self.root.mainloop()

    def quit(self) -> None:
        self.root.quit()

    # --- status pill --------------------------------------------------------

    def _build_pill(self) -> None:
        pill = tk.Toplevel(self.root)
        pill.overrideredirect(True)
        pill.attributes("-topmost", True)
        pill.configure(bg="#18191b")
        frame = tk.Frame(pill, bg="#18191b", padx=14, pady=8)
        frame.pack()
        self._dot = tk.Label(frame, text="●", bg="#18191b", font=("Segoe UI", 11))
        self._dot.pack(side="left")
        self._label = tk.Label(frame, bg="#18191b", fg="#edeef0", font=("Segoe UI", 10))
        self._label.pack(side="left", padx=(6, 0))
        pill.update_idletasks()
        _make_passive(pill)
        # Keep it mapped and toggle opacity instead of withdraw/deiconify:
        # re-showing a window on Windows can activate it and steal focus.
        pill.attributes("-alpha", 0.0)
        self._pill = pill

    def show(self, kind: str, text: str, hide_after_ms: int | None = None) -> None:
        if self._hide_job:
            self.root.after_cancel(self._hide_job)
            self._hide_job = None
        self._dot.configure(fg=COLORS[kind])
        self._label.configure(text=text)
        self._pill.update_idletasks()
        w, h = self._pill.winfo_reqwidth(), self._pill.winfo_reqheight()
        sw, sh = self._pill.winfo_screenwidth(), self._pill.winfo_screenheight()
        self._pill.geometry(f"+{(sw - w) // 2}+{sh - h - 90}")
        self._pill.attributes("-alpha", 0.92)
        if hide_after_ms:
            self._hide_job = self.root.after(hide_after_ms, self.hide)

    def hide(self) -> None:
        self._hide_job = None
        self._pill.attributes("-alpha", 0.0)

    # --- correction window --------------------------------------------------

    @property
    def correcting(self) -> bool:
        """True while a window is open that you might be dictating a fix into."""
        return self._correction is not None or self._history is not None

    def open_models(self, active, use, install) -> None:
        from .models_ui import ModelsWindow

        if self._models is not None:
            self._models.refresh()
            bring_to_front(self._models.win)
            return

        def closed() -> None:
            self._models = None

        self._models = ModelsWindow(self.root, active, use, install, on_close=closed)
        bring_to_front(self._models.win)

    def refresh_models(self) -> None:
        if self._models is not None:
            self._models.refresh()

    def model_log(self, line: str) -> None:
        if self._models is not None:
            self._models.append_log(line)

    def open_history(self, learner) -> None:
        from .history import HistoryWindow

        if self._history is not None:
            self._history.refresh()
            bring_to_front(self._history.win)
            return

        def closed() -> None:
            self._history = None

        self._history = HistoryWindow(self.root, learner, on_close=closed)
        bring_to_front(self._history.win)

    def ask_correction(
        self,
        raw: str,
        pasted: str,
        on_save: Callable[[str, bool], None],
    ) -> None:
        """Let the user edit the last dictation.

        on_save(corrected, replace) is called on save; `replace` means also
        swap the text in the app it was pasted into.
        """
        if self._correction is not None:
            self._correction.destroy()
        previous = foreground_window()

        win = tk.Toplevel(self.root)
        self._correction = win
        win.title("Speechify: fix last dictation")
        win.attributes("-topmost", True)
        win.configure(padx=14, pady=12)

        tk.Label(
            win,
            text="Fix the text below. You can type, or hold the talk key to dictate the fix.",
            anchor="w",
        ).pack(fill="x")
        text = tk.Text(win, width=72, height=6, wrap="word", font=("Segoe UI", 11), undo=True)
        text.insert("1.0", pasted)
        text.pack(fill="both", expand=True, pady=(6, 4))
        if raw != pasted:
            tk.Label(win, text=f"Whisper heard: {raw}", fg="#8b8d98", anchor="w",
                     wraplength=560, justify="left").pack(fill="x")

        def close() -> None:
            self._correction = None
            win.destroy()

        def save(replace: bool) -> None:
            corrected = text.get("1.0", "end").strip()
            close()
            focus_window(previous)
            on_save(corrected, replace)

        buttons = tk.Frame(win)
        buttons.pack(fill="x", pady=(8, 0))
        tk.Button(buttons, text="Save + replace in app  (Ctrl+Enter)",
                  command=lambda: save(True)).pack(side="right")
        tk.Button(buttons, text="Save only  (Shift+Enter)",
                  command=lambda: save(False)).pack(side="right", padx=6)
        tk.Button(buttons, text="Cancel  (Esc)", command=close).pack(side="left")

        win.bind("<Control-Return>", lambda e: (save(True), "break")[1])
        win.bind("<Shift-Return>", lambda e: (save(False), "break")[1])
        win.bind("<Escape>", lambda e: close())
        win.protocol("WM_DELETE_WINDOW", close)

        text.tag_add("sel", "1.0", "end-1c")
        win.lift()
        bring_to_front(win)
        text.focus_set()
