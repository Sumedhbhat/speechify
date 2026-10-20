"""Model picker: every model, its ratings, and whether it's on disk."""

from __future__ import annotations

import tkinter as tk
from tkinter import ttk
from typing import Callable

from .config import data_dir
from .engines import host
from .engines.catalog import BACKENDS, UNSUPPORTED, ModelSpec, all_models, stars


class ModelsWindow:
    """use(spec) installs if needed and switches; install(spec) only downloads.

    Both run in the background and report through log(); the window just
    calls them and refreshes.
    """

    def __init__(
        self,
        master: tk.Misc,
        active: Callable[[], str | None],
        use: Callable[[ModelSpec], None],
        install: Callable[[ModelSpec], None],
        on_close: Callable[[], None],
    ):
        self.active, self.use, self.install, self.on_close = active, use, install, on_close
        self.specs: dict[str, ModelSpec] = {}
        win = self.win = tk.Toplevel(master)
        win.title("Speechify: models")
        win.geometry("1060x560")
        win.protocol("WM_DELETE_WINDOW", self.close)

        top = ttk.Frame(win, padding=(10, 10, 10, 0))
        top.pack(fill="x")
        self.only_ready = tk.BooleanVar(value=False)
        ttk.Checkbutton(top, text="Only show models on this PC", variable=self.only_ready,
                        command=self.refresh).pack(side="left")
        self.current = ttk.Label(top, foreground="#1a7f37")
        self.current.pack(side="right")

        cols = [("name", "Model", 230), ("acc", "Accuracy", 80), ("speed", "Speed", 70),
                ("langs", "Languages", 170), ("vram", "VRAM", 60), ("size", "Download", 75),
                ("learn", "Learns", 110), ("status", "Status", 110)]
        self.tree = ttk.Treeview(win, columns=[c[0] for c in cols], show="headings", selectmode="browse")
        for key, title, width in cols:
            self.tree.heading(key, text=title)
            self.tree.column(key, width=width, stretch=key in ("name", "langs"))
        self.tree.tag_configure("active", foreground="#1a7f37")
        self.tree.tag_configure("missing", foreground="#888")
        self.tree.pack(fill="both", expand=True, padx=10, pady=8)
        self.tree.bind("<<TreeviewSelect>>", lambda e: self._describe())
        self.tree.bind("<Double-1>", lambda e: self._use())

        self.notes = ttk.Label(win, wraplength=1000, justify="left", padding=(10, 0))
        self.notes.pack(fill="x")

        actions = ttk.Frame(win, padding=10)
        actions.pack(fill="x")
        ttk.Button(actions, text="Use this model", command=self._use).pack(side="right")
        ttk.Button(actions, text="Download only", command=self._install).pack(side="right", padx=6)
        ttk.Label(
            actions, foreground="#888",
            text="Not on Windows: " + "; ".join(f"{k} ({v})" for k, v in UNSUPPORTED.items()),
        ).pack(side="left")

        self.log = tk.Text(win, height=5, font=("Consolas", 9), state="disabled", background="#f6f6f6")
        self.log.pack(fill="x", padx=10, pady=(0, 10))
        self.refresh()
        win.lift()
        win.focus_force()

    def refresh(self) -> None:
        selected = self.tree.selection()
        self.tree.delete(*self.tree.get_children())
        self.specs.clear()
        active = self.active()
        for spec in all_models(data_dir()):
            state = host.status(spec)
            if self.only_ready.get() and state != "ready":
                continue
            label = "● in use" if spec.id == active else state
            learns = "hints + training" if spec.finetune else "hints" if spec.prompt else "auto-fixes"
            tags = ("active",) if spec.id == active else () if state == "ready" else ("missing",)
            self.specs[spec.id] = spec
            self.tree.insert("", "end", iid=spec.id, tags=tags, values=(
                spec.name, stars(spec.accuracy), stars(spec.speed), spec.languages,
                f"{spec.vram_gb:g} GB", f"{spec.download_gb:g} GB" if spec.download_gb else "local",
                learns, label,
            ))
        if selected and selected[0] in self.specs:
            self.tree.selection_set(selected[0])
        name = self.specs[active].name if active in self.specs else active
        self.current.configure(text=f"In use: {name}" if active else "No model loaded")

    def _selected(self) -> ModelSpec | None:
        sel = self.tree.selection()
        return self.specs.get(sel[0]) if sel else None

    def _describe(self) -> None:
        spec = self._selected()
        if spec:
            backend = BACKENDS[spec.backend].label
            self.notes.configure(text=f"{spec.name}, runs on {backend}. {spec.notes}".strip())

    def _use(self) -> None:
        spec = self._selected()
        if spec:
            self.use(spec)

    def _install(self) -> None:
        spec = self._selected()
        if spec:
            self.install(spec)

    def append_log(self, line: str) -> None:
        self.log.configure(state="normal")
        self.log.insert("end", line + "\n")
        self.log.delete("1.0", "end-200l")  # keep the last 200 lines
        self.log.see("end")
        self.log.configure(state="disabled")

    def close(self) -> None:
        self.win.destroy()
        self.on_close()
