"""Hold-to-talk local dictation that learns from your corrections."""

from __future__ import annotations

import argparse
import sys

from . import __version__
from .config import Config, data_dir


def main() -> None:
    # Pipes on Windows default to cp1252, which can't encode the ★ ratings.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(errors="replace")
    parser = argparse.ArgumentParser(prog="speechify", description=__doc__)
    parser.add_argument("--version", action="version", version=f"speechify {__version__}")
    sub = parser.add_subparsers(dest="cmd")
    sub.add_parser("run", help="start dictation (default)")
    sub.add_parser("stats", help="show what has been learned")
    sub.add_parser("rules", help="list learned corrections")
    forget = sub.add_parser("forget", help="delete a learned correction or vocabulary term")
    forget.add_argument("phrase")
    sub.add_parser("history", help="browse, replay and edit every saved recording")
    export = sub.add_parser("export", help="copy recordings + text to a folder (Hugging Face audiofolder)")
    export.add_argument("folder")
    models = sub.add_parser("models", help="list speech models, or install / switch to one")
    models.add_argument("action", nargs="?", choices=["list", "install", "use"], default="list")
    models.add_argument("model", nargs="?", help="model id from `speechify models`")
    selftest = sub.add_parser("selftest", help="drive the real app end to end and report what works")
    selftest.add_argument("--model", help="model id to test (default: the configured one)")
    selftest.add_argument("--engine-only", action="store_true",
                          help="only install, load and transcribe; no desktop needed")
    selftest.add_argument("--device", choices=["cuda", "cpu"], help="override the configured device")
    selftest.add_argument("--out", help="folder for report.json, summary.md and screenshots")
    sub.add_parser("devices", help="list microphones")
    sub.add_parser("where", help="print the config and data directory")
    args = parser.parse_args()
    if args.cmd in (None, "run"):
        _log_to_file_without_terminal()

    if args.cmd == "devices":
        import sounddevice as sd

        print(sd.query_devices())
        return
    if args.cmd == "where":
        print(data_dir())
        return

    if args.cmd == "selftest":
        from .selftest import main as selftest

        raise SystemExit(selftest(args.model, args.engine_only, args.device, args.out))

    cfg = Config.load()
    if args.cmd == "models":
        models_command(cfg, args.action, args.model)
        return
    if args.cmd in ("stats", "rules", "forget", "history", "export"):
        from .learning import Learner

        learner = Learner(data_dir(), cfg.autocorrect_after)
        if args.cmd == "history":
            from .history import run_standalone

            run_standalone(learner)
        elif args.cmd == "export":
            from pathlib import Path

            n = learner.export(Path(args.folder))
            print(f"Exported {n} recordings to {args.folder}")
        elif args.cmd == "stats":
            for k, v in learner.stats().items():
                print(f"{k:>13}: {v}")
        elif args.cmd == "rules":
            for wrong, right, count in learner.rules():
                active = "*" if count >= cfg.autocorrect_after else " "
                print(f"{active} {count:>3}  {wrong!r} -> {right!r}")
            print(f"\n* = applied automatically (seen >= {cfg.autocorrect_after} times)")
        else:
            print(f"Removed {learner.forget(args.phrase)} entries")
        return

    from .app import App

    App(cfg).run()


def _log_to_file_without_terminal() -> None:
    """Started from the app menu, Finder or the Dock, the installed app has no
    console, so its output goes to a log file instead. (On Windows it keeps
    its console window.)"""
    if not getattr(sys, "frozen", False) or sys.platform == "win32":
        return
    if sys.stdout is not None and sys.stdout.isatty():
        return
    import os
    from pathlib import Path

    if sys.platform == "darwin":
        log = Path.home() / "Library" / "Logs" / "Speechify.log"
    else:
        state = Path(os.environ.get("XDG_STATE_HOME") or Path.home() / ".local" / "state")
        log = state / "speechify" / "speechify.log"
    log.parent.mkdir(parents=True, exist_ok=True)
    stream = open(log, "a", buffering=1, encoding="utf-8", errors="replace")
    # At the descriptor level too, so the model worker's output lands there.
    os.dup2(stream.fileno(), 1)
    os.dup2(stream.fileno(), 2)
    sys.stdout = sys.stderr = stream


def models_command(cfg: Config, action: str, model: str | None) -> None:
    from .config import set_value
    from .engines import host
    from .engines.catalog import UNSUPPORTED, all_models, resolve, stars

    if action == "list":
        current = cfg.model
        try:
            "★½".encode(sys.stdout.encoding or "utf-8")
        except UnicodeEncodeError:  # e.g. a cp1252 pipe on Windows

            def stars(rating: float) -> str:
                return f"{rating:g}/5" if rating > 0 else "yours"

        print(f"{'':2}{'id':<36}{'accuracy':<10}{'speed':<8}{'VRAM':<7}{'status':<16}languages")
        for spec in all_models(data_dir()):
            mark = "*" if current in (spec.id, spec.repo) else " "
            print(f"{mark} {spec.id:<36}{stars(spec.accuracy):<10}{stars(spec.speed):<8}"
                  f"{spec.vram_gb:<7g}{host.status(spec):<16}{spec.languages}")
        print("\n* = selected.  speechify models use <id>  installs (if needed) and selects a model.")
        for name, why in UNSUPPORTED.items():
            print(f"  Not available: {name}, {why}.")
        return
    if not model:
        raise SystemExit(f"usage: speechify models {action} <id>")
    try:
        spec = resolve(model, data_dir())
    except KeyError as exc:
        raise SystemExit(str(exc)) from None
    host.install(spec, cfg.engine_opts())
    if action == "use":
        set_value("model", spec.repo if spec.id.startswith("ft:") else spec.id)
        print(f"Selected {spec.name}. A running Speechify picks it up on restart, or switch live with Ctrl+Alt+M.")


if __name__ == "__main__":
    main()
