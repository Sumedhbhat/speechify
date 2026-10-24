"""Status overlay and correction window (Tk, main thread only).

Other threads must go through UI.post(); Tk isn't thread-safe.
"""

from __future__ import annotations

import math
import queue
import sys
import time
import tkinter as tk
import tkinter.font as tkfont
from typing import Callable

COLORS = {
    "listening": "#ff5a5f",
    "working": "#f5a524",
    "ok": "#3dd68c",
    "info": "#8b8d98",
    "error": "#ff5a5f",
}
GLYPHS = {"ok": "✓", "info": "•", "error": "!"}
PILL_BG = "#141416"
PILL_EDGE = "#2c2d32"
PILL_TEXT = "#edeef0"
PILL_MUTED = "#8b8d98"
WAITING = "#5d6068"  # the mic dot before the first audio arrives
BARS = 16
FRAME_MS = 33

# Corners can only be see-through where Tk supports it: a color key on
# Windows, a transparent window on macOS. On Linux the pill is a rectangle.
if sys.platform == "win32":
    _CLEAR = "#010203"
elif sys.platform == "darwin":
    _CLEAR = "systemTransparent"
else:
    _CLEAR = None


def level_to_bar(rms: float) -> float:
    """Microphone RMS to a bar height from 0 to 1, on a log scale so quiet
    speech still moves the bars: -55 dBFS and below is flat, -15 dBFS full."""
    db = 20 * math.log10(max(rms, 1e-6))
    return min(1.0, max(0.0, (db + 55) / 40))


def capsule(x0: float, y0: float, x1: float, y1: float, steps: int = 10) -> list[float]:
    """Polygon points for a rectangle with fully rounded ends."""
    r = (y1 - y0) / 2
    points: list[float] = []
    for cx, start in ((x1 - r, -90), (x0 + r, 90)):
        for i in range(steps + 1):
            a = math.radians(start + 180 * i / steps)
            points += [cx + r * math.cos(a), y0 + r - r * math.sin(a)]
    return points


def _mix(fg: str, bg: str, t: float) -> str:
    """Blend two #rrggbb colors; t=1 is all fg. Tk canvases have no alpha."""
    a, b = int(fg[1:], 16), int(bg[1:], 16)
    out = 0
    for shift in (16, 8, 0):
        ca, cb = (a >> shift) & 255, (b >> shift) & 255
        out |= round(cb + (ca - cb) * t) << shift
    return f"#{out:06x}"

if sys.platform == "win32":
    FONT, MONO = "Segoe UI", "Consolas"
elif sys.platform == "darwin":
    FONT, MONO = "Helvetica Neue", "Menlo"
else:
    FONT, MONO = "DejaVu Sans", "DejaVu Sans Mono"


# --- focus handling ----------------------------------------------------------
# The overlay must never take focus, or Ctrl+V would paste into it instead of
# the app you're dictating into. On Linux, window managers don't focus
# override-redirect windows and give focus back when ours close, so the
# defaults at the bottom are enough there.

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

elif sys.platform == "darwin":
    from AppKit import NSApplication, NSWorkspace

    _ACTIVATE_IGNORING_OTHER_APPS = 1 << 1

    def _make_passive(widget: tk.Misc) -> None:
        pass  # overlay windows that are only made visible don't activate the app

    def foreground_window():
        return NSWorkspace.sharedWorkspace().frontmostApplication()

    def focus_window(app) -> None:
        """Closing our window leaves Speechify active on macOS; hand focus back."""
        if app is not None:
            app.activateWithOptions_(_ACTIVATE_IGNORING_OTHER_APPS)

    def bring_to_front(win: tk.Misc) -> None:
        NSApplication.sharedApplication().activateIgnoringOtherApps_(True)
        win.lift()
        win.focus_force()

else:

    def _make_passive(widget: tk.Misc) -> None:
        pass

    def foreground_window():
        return None

    def focus_window(window) -> None:
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
        self._settings = None
        self.meter = None  # anything with .level (RMS) and .live, e.g. the Recorder
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
        if sys.platform == "darwin":
            pill.attributes("-transparent", True)
        bg = _CLEAR or PILL_BG
        pill.configure(bg=bg)
        if sys.platform == "win32":
            pill.attributes("-transparentcolor", _CLEAR)
        self._scale = max(1.0, self.root.winfo_fpixels("1i") / 96)
        self._font = tkfont.Font(family=FONT, size=10)
        self._mono = tkfont.Font(family=MONO, size=9)
        self._canvas = tk.Canvas(pill, bg=bg, highlightthickness=0, bd=0)
        self._canvas.pack()
        pill.update_idletasks()
        _make_passive(pill)
        # Keep it mapped and toggle opacity instead of withdraw/deiconify:
        # re-showing a window on Windows can activate it and steal focus.
        pill.attributes("-alpha", 0.0)
        self._pill = pill
        self._kind: str | None = None
        self._anim_job: str | None = None
        self._levels = [0.0] * BARS
        self._shown_at = 0.0

    def _px(self, n: float) -> float:
        return n * self._scale

    def show(self, kind: str, text: str, hide_after_ms: int | None = None) -> None:
        """Show the overlay pill. "listening" draws live level bars from
        self.meter and a timer, "working" an animation next to `text`, and
        the others a glyph and `text`."""
        if self._hide_job:
            self.root.after_cancel(self._hide_job)
            self._hide_job = None
        if kind != self._kind or kind != "listening":
            self._layout(kind, text)
        self._kind = kind
        self._pill.attributes("-alpha", 0.96)
        if kind in ("listening", "working") and self._anim_job is None:
            self._tick()
        if hide_after_ms:
            self._hide_job = self.root.after(hide_after_ms, self.hide)

    def hide(self) -> None:
        self._hide_job = None
        self._kind = None
        if self._anim_job:
            self.root.after_cancel(self._anim_job)
            self._anim_job = None
        self._pill.attributes("-alpha", 0.0)

    def _layout(self, kind: str, text: str) -> None:
        c, px = self._canvas, self._px
        c.delete("all")
        h, pad = px(40), px(16)
        mid = h / 2
        if kind == "listening":
            self._levels = [0.0] * BARS
            self._shown_at = time.monotonic()
            bars_w = BARS * px(3) + (BARS - 1) * px(3)
            timer_w = self._mono.measure("0:00")
            w = pad + px(10) + px(12) + bars_w + px(12) + timer_w + pad
            self._dot = c.create_oval(0, 0, 0, 0, fill=WAITING, outline="")
            x = pad + px(10) + px(12)
            self._bars = []
            for i in range(BARS):
                bx = x + i * px(6)
                self._bars.append(c.create_rectangle(bx, mid - px(1.5), bx + px(3), mid + px(1.5),
                                                     fill=PILL_MUTED, outline=""))
            self._timer = c.create_text(w - pad, mid, text="0:00", anchor="e",
                                        fill=PILL_MUTED, font=self._mono)
        elif kind == "working":
            text_w = self._font.measure(text)
            w = pad + px(26) + px(10) + text_w + pad + px(2)
            self._dots = [c.create_oval(0, 0, 0, 0, fill=COLORS["working"], outline="") for _ in range(3)]
            c.create_text(pad + px(36), mid, text=text, anchor="w", fill=PILL_TEXT, font=self._font)
        else:
            text_w = self._font.measure(text)
            w = pad + px(14) + px(8) + text_w + pad + px(2)
            c.create_text(pad + px(7), mid, text=GLYPHS.get(kind, "•"), fill=COLORS[kind],
                          font=(FONT, 11, "bold"))
            c.create_text(pad + px(22), mid, text=text, anchor="w", fill=PILL_TEXT, font=self._font)
        # The capsule goes underneath everything drawn above.
        shape = capsule(px(0.5), px(0.5), w - px(0.5), h - px(0.5)) if _CLEAR else [0, 0, w, 0, w, h, 0, h]
        c.tag_lower(c.create_polygon(shape, fill=PILL_BG, outline=PILL_EDGE))
        w, h = round(w), round(h)
        c.configure(width=w, height=h)
        sw, sh = self._pill.winfo_screenwidth(), self._pill.winfo_screenheight()
        self._pill.geometry(f"{w}x{h}+{(sw - w) // 2}+{sh - h - round(px(80))}")
        if kind in ("listening", "working"):
            self._animate(kind)

    def _tick(self) -> None:
        self._anim_job = None
        if self._kind not in ("listening", "working"):
            return
        self._animate(self._kind)
        self._anim_job = self.root.after(FRAME_MS, self._tick)

    def _animate(self, kind: str) -> None:
        c, px = self._canvas, self._px
        h = px(40)
        mid, now = h / 2, time.monotonic()
        if kind == "working":
            for i, dot in enumerate(self._dots):
                lift = max(0.0, math.sin(now * 7 - i * 0.9)) * px(4)
                cx, r = px(16) + px(4) + i * px(9), px(3)
                c.coords(dot, cx - r, mid - r - lift, cx + r, mid + r - lift)
            return
        live = bool(getattr(self.meter, "live", True))
        level = level_to_bar(getattr(self.meter, "level", 0.0)) if live else 0.0
        # Fast attack, slow release, so speech reads as a moving waveform.
        self._levels = self._levels[1:] + [max(level, self._levels[-1] * 0.6)]
        for i, (bar, v) in enumerate(zip(self._bars, self._levels)):
            # Taper the ends so the bars sit in an envelope rather than a box.
            v *= 0.55 + 0.45 * math.sin(math.pi * (i + 0.5) / BARS)
            half = px(1.5) + v * px(10)
            x0 = px(16) + px(22) + i * px(6)
            c.coords(bar, x0, mid - half, x0 + px(3), mid + half)
            c.itemconfigure(bar, fill=_mix(PILL_TEXT, PILL_MUTED, min(1.0, v * 1.4)))
        pulse = 1 + 0.18 * math.sin(now * 6) if live else 1
        r = px(5) * pulse
        cx = px(16) + px(5)
        c.coords(self._dot, cx - r, mid - r, cx + r, mid + r)
        c.itemconfigure(self._dot, fill=COLORS["listening"] if live else WAITING)
        secs = int(now - self._shown_at)
        c.itemconfigure(self._timer, text=f"{secs // 60}:{secs % 60:02d}")

    # --- correction window --------------------------------------------------

    @property
    def correcting(self) -> bool:
        """True while a window is open that you might be dictating a fix into."""
        return self._correction is not None or self._history is not None

    def open_models(self, active, use, install, rollback, device) -> None:
        from .models_ui import ModelsWindow

        if self._models is not None:
            self._models.refresh()
            bring_to_front(self._models.win)
            return

        def closed() -> None:
            self._models = None

        self._models = ModelsWindow(self.root, active, use, install, rollback, device, on_close=closed)
        bring_to_front(self._models.win)

    def open_settings(self, cfg, on_save, open_models, model_name, welcome: bool = False) -> None:
        from .settings_ui import SettingsWindow

        if self._settings is not None:
            bring_to_front(self._settings.win)
            return

        def closed() -> None:
            self._settings = None

        self._settings = SettingsWindow(self.root, cfg, on_save, open_models, model_name,
                                        welcome=welcome, on_close=closed)
        bring_to_front(self._settings.win)

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
        text = tk.Text(win, width=72, height=6, wrap="word", font=(FONT, 11), undo=True)
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
