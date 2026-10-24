"""History window: every saved recording, playable and editable.

Whatever text a recording has here is what fine-tuning trains on. Editing
it also teaches the vocabulary and auto-replace rules, the same as a fix
made with the correction hotkey.
"""

from __future__ import annotations

import sys
import time
import tkinter as tk
from tkinter import messagebox, ttk
from typing import Callable

from .audio import SAMPLE_RATE, play, stop_playback
from .learning import Learner, Sample
from .ui import FONT


def _duration(sample: Sample) -> str:
    path = sample.path
    if path is None or not path.exists():
        return "–"
    seconds = max(0, path.stat().st_size - 44) / (SAMPLE_RATE * 2)
    return f"{seconds:.1f}s"


FILTERS = {
    "All recordings": lambda s: True,
    "Edited": lambda s: not s.excluded and s.corrected is not None,
    "As pasted": lambda s: not s.excluded and s.corrected is None,
    "Excluded": lambda s: s.excluded,
}


def _status(sample: Sample) -> str:
    if sample.excluded:
        return "excluded"
    if sample.corrected is not None:
        return "edited"
    return "as pasted"


class HistoryWindow:
    def __init__(self, master: tk.Misc, learner: Learner, on_close: Callable[[], None] | None = None):
        self.learner = learner
        self.on_close = on_close
        self.samples: dict[str, Sample] = {}
        self.current: Sample | None = None

        win = self.win = tk.Toplevel(master)
        win.title("Speechify: recordings")
        # Fit small screens (e.g. 1024x768) instead of running off the edge.
        win.geometry(f"{min(980, win.winfo_screenwidth() - 40)}x{min(600, win.winfo_screenheight() - 80)}+20+20")
        win.protocol("WM_DELETE_WINDOW", self.close)

        top = ttk.Frame(win, padding=(10, 10, 10, 0))
        top.pack(fill="x")
        ttk.Label(top, text="Search").pack(side="left")
        self.search = tk.StringVar()
        entry = ttk.Entry(top, textvariable=self.search, width=40)
        entry.pack(side="left", padx=6)
        entry.bind("<Return>", lambda e: self.refresh())
        self.filter = tk.StringVar(value=next(iter(FILTERS)))
        shown = ttk.Combobox(top, textvariable=self.filter, values=list(FILTERS), state="readonly", width=14)
        shown.pack(side="left", padx=(0, 6))
        shown.bind("<<ComboboxSelected>>", lambda e: self.refresh())
        ttk.Button(top, text="Refresh", command=self.refresh).pack(side="left")
        self.summary = ttk.Label(top, foreground="#666")
        self.summary.pack(side="right")

        panes = ttk.PanedWindow(win, orient="horizontal")
        panes.pack(fill="both", expand=True, padx=10, pady=10)

        left = ttk.Frame(panes)
        self.tree = ttk.Treeview(
            left, columns=("when", "len", "status", "text"), show="headings", selectmode="browse"
        )
        for col, title, width, stretch in [
            ("when", "When", 120, False),
            ("len", "Length", 60, False),
            ("status", "Status", 80, False),
            ("text", "Text used for training", 300, True),
        ]:
            self.tree.heading(col, text=title)
            self.tree.column(col, width=width, stretch=stretch)
        self.tree.tag_configure("excluded", foreground="#999")
        self.tree.tag_configure("edited", foreground="#1a7f37")
        scroll = ttk.Scrollbar(left, orient="vertical", command=self.tree.yview)
        self.tree.configure(yscrollcommand=scroll.set)
        self.tree.pack(side="left", fill="both", expand=True)
        scroll.pack(side="right", fill="y")
        self.tree.bind("<<TreeviewSelect>>", lambda e: self._select())
        self.tree.bind("<Double-1>", lambda e: self.copy())
        panes.add(left, weight=3)

        right = ttk.Frame(panes, padding=(10, 0, 0, 0))
        controls = ttk.Frame(right)
        controls.pack(fill="x")
        ttk.Button(controls, text="▶ Play", command=self.play).pack(side="left")
        ttk.Button(controls, text="■ Stop", command=stop_playback).pack(side="left", padx=4)
        ttk.Button(controls, text="Copy text", command=self.copy).pack(side="left")
        self.exclude_btn = ttk.Button(controls, text="Exclude from training", command=self.toggle_excluded)
        self.exclude_btn.pack(side="right")

        ttk.Label(right, text="Text (edit and save to use it for training):").pack(anchor="w", pady=(10, 2))
        self.text = tk.Text(right, height=8, wrap="word", font=(FONT, 11), undo=True)
        self.text.pack(fill="both", expand=True)
        self.heard = ttk.Label(right, foreground="#666", wraplength=380, justify="left")
        self.heard.pack(anchor="w", pady=(6, 0))

        actions = ttk.Frame(right)
        actions.pack(fill="x", pady=(10, 0))
        ttk.Button(actions, text="Save  (Ctrl+S)", command=self.save).pack(side="right")
        ttk.Button(actions, text="Revert to original", command=self.revert).pack(side="right", padx=4)
        ttk.Button(actions, text="Delete", command=self.delete).pack(side="left")
        self.flash = ttk.Label(right, foreground="#1a7f37")
        self.flash.pack(anchor="w", pady=(6, 0))
        panes.add(right, weight=2)

        win.bind("<Control-s>", lambda e: self.save())
        self.refresh()
        win.lift()
        win.focus_force()

    # --- list ----------------------------------------------------------------

    def refresh(self) -> None:
        selected = self.current.id if self.current else None
        self.tree.delete(*self.tree.get_children())
        self.samples.clear()
        keep = FILTERS.get(self.filter.get(), FILTERS["All recordings"])
        for s in self.learner.samples(self.search.get()):
            if not keep(s):
                continue
            iid = str(s.id)
            self.samples[iid] = s
            when = time.strftime("%b %d %H:%M", time.localtime(s.ts))
            tags = ("excluded",) if s.excluded else ("edited",) if s.corrected is not None else ()
            self.tree.insert("", "end", iid=iid, values=(when, _duration(s), _status(s), s.text), tags=tags)
        st = self.learner.stats()
        self.summary.configure(
            text=f"{st['samples']} recordings · {st['corrected']} edited · "
                 f"{st['samples'] - st['excluded']} used for training"
        )
        if selected is not None and str(selected) in self.samples:
            self.tree.selection_set(str(selected))
            self.tree.see(str(selected))

    def _select(self) -> None:
        sel = self.tree.selection()
        if not sel:
            return
        self.current = self.samples[sel[0]]
        self.text.delete("1.0", "end")
        self.text.insert("1.0", self.current.text)
        self.heard.configure(text=f"Whisper heard: {self.current.raw}")
        self.exclude_btn.configure(
            text="Include in training" if self.current.excluded else "Exclude from training"
        )
        self.flash.configure(text="")

    # --- actions --------------------------------------------------------------

    def play(self) -> None:
        if self.current and self.current.path and self.current.path.exists():
            play(self.current.path)

    def copy(self) -> None:
        """Put the selected recording's text on the clipboard, to paste anywhere."""
        if not self.current:
            return
        text = self.text.get("1.0", "end").strip()
        self.win.clipboard_clear()
        self.win.clipboard_append(text)
        self.flash.configure(text="Copied. Paste it with Ctrl+V." if sys.platform != "darwin"
                             else "Copied. Paste it with Command+V.")

    def save(self) -> None:
        if not self.current:
            return
        edits = self.learner.edit_sample(self.current.id, self.text.get("1.0", "end"))
        learned = ", ".join(f"{w} → {r}" for w, r in edits[:3])
        self.flash.configure(text=f"Saved. Learned: {learned}" if learned else "Saved.")
        self.refresh()

    def revert(self) -> None:
        if not self.current:
            return
        self.text.delete("1.0", "end")
        self.text.insert("1.0", self.current.pasted)
        self.flash.configure(text="Reverted in the editor. Save to keep it.")

    def toggle_excluded(self) -> None:
        if not self.current:
            return
        self.learner.set_excluded(self.current.id, not self.current.excluded)
        self.refresh()
        self._select()

    def delete(self) -> None:
        if not self.current:
            return
        if not messagebox.askyesno("Delete recording", "Delete this audio and its text for good?",
                                   parent=self.win):
            return
        self.learner.delete_sample(self.current.id)
        self.current = None
        self.text.delete("1.0", "end")
        self.heard.configure(text="")
        self.refresh()

    def close(self) -> None:
        stop_playback()
        self.win.destroy()
        if self.on_close:
            self.on_close()


def run_standalone(learner: Learner) -> None:
    root = tk.Tk()
    root.withdraw()
    HistoryWindow(root, learner, on_close=root.quit)
    root.mainloop()
