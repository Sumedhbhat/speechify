"""Hold-to-talk local dictation that learns from your corrections."""

from __future__ import annotations

import argparse
import sys

from . import __version__
from .config import DEVICES, Config, data_dir


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
    models.add_argument("--rollback", action="store_true",
                        help="go back to the version before the current retrained model")
    learn = sub.add_parser("learn", help="teacher labels and automatic retraining")
    learn.add_argument("action", nargs="?", choices=["status", "run"], default="status",
                       help="status (default), or run: label and retrain now, without waiting to be idle")
    learn.add_argument("--retrain", action="store_true", help="with run: retrain even if it isn't due")
    selftest = sub.add_parser("selftest", help="drive the real app end to end and report what works")
    selftest.add_argument("--model", help="model id to test (default: the configured one)")
    selftest.add_argument("--engine-only", action="store_true",
                          help="only install, load and transcribe; no desktop needed")
    selftest.add_argument("--device", choices=DEVICES, help="override the configured device")
    selftest.add_argument("--out", help="folder for report.json, summary.md and screenshots")
    bench = sub.add_parser("bench", help="time each engine for a model on this machine (GPU vs transcribe.cpp)")
    bench.add_argument("--model", action="append", help="model id (repeatable; default: Parakeet v3 and Whisper turbo)")
    bench.add_argument("--runs", type=int, default=5, help="runs per clip; the median is reported (default 5)")
    bench.add_argument("--cpu", action="store_true", help="also time both engines on the CPU")
    bench.add_argument("--clips", help="folder with clips.json and WAV files (default: the self-test clip)")
    bench.add_argument("--out", help="JSON file for the results")
    sub.add_parser("devices", help="list microphones")
    sub.add_parser("where", help="print the config and data directory")
    args = parser.parse_args()
    if args.cmd in (None, "run"):
        from .applog import redirect_to_file

        redirect_to_file()

    if args.cmd == "devices":
        import sounddevice as sd

        print(sd.query_devices())
        return
    if args.cmd == "where":
        print(data_dir())
        return

    if args.cmd == "bench":
        from .bench import main as bench

        raise SystemExit(bench(args.model, args.runs, args.cpu, args.clips, args.out))
    if args.cmd == "selftest":
        from .selftest import main as selftest

        raise SystemExit(selftest(args.model, args.engine_only, args.device, args.out))

    cfg = Config.load()
    if args.cmd == "models":
        if args.rollback:
            rollback_command(cfg)
        else:
            models_command(cfg, args.action, args.model)
        return
    if args.cmd == "learn":
        learn_command(cfg, args.action, args.retrain)
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


def rollback_command(cfg: Config) -> None:
    from . import versions
    from .learning import Learner

    learner = Learner(data_dir(), cfg.autocorrect_after)
    try:
        target, description = versions.rollback(learner, cfg)
    except versions.RollbackError as exc:
        raise SystemExit(str(exc)) from None
    print(f"Rolled back to {description}. A running Speechify picks it up on restart, "
          "or use Roll back in the model window (Ctrl+Alt+M) to switch live.")


def learn_command(cfg: Config, action: str, force_retrain: bool) -> None:
    from . import autolearn
    from .engines.host import EngineHost
    from .learning import Learner

    learner = Learner(data_dir(), cfg.autocorrect_after)
    if action == "status":
        print(autolearn.status_text(cfg, learner))
        return
    # No dictation model is loaded here, so nothing needs unloading or reloading.
    auto = autolearn.AutoLearner(cfg, learner, EngineHost())
    if learner.teacher_queue(1):
        print(f"Teacher pass: {auto.teacher_pass(check_idle=False)}")
        if learner.teacher_queue(1) and not auto.job:  # it only installed the teacher
            print(f"Teacher pass: {auto.teacher_pass(check_idle=False)}")
    due, why = auto.retrain_status()
    if due or force_retrain:
        print(f"Retraining ({why}): {auto.retrain(check_idle=False)}")
    else:
        print(f"Not retraining: {why}. Pass --retrain to retrain anyway.")


def models_command(cfg: Config, action: str, model: str | None) -> None:
    from .config import set_value
    from .engines import host
    from .engines.catalog import UNSUPPORTED, all_models, placed, resolve, stars

    if action == "list":
        current = cfg.model
        try:
            "★½".encode(sys.stdout.encoding or "utf-8")
        except UnicodeEncodeError:  # e.g. a cp1252 pipe on Windows

            def stars(rating: float) -> str:
                return f"{rating:g}/5" if rating > 0 else "yours"

        device = cfg.resolved_device()
        print(f"Device: {cfg.device}" + (f" ({device} on this machine)" if cfg.device == "auto" else ""))
        print(f"{'':2}{'id':<36}{'accuracy':<10}{'speed':<8}{'VRAM':<7}{'engine':<14}{'status':<16}languages")
        for spec in all_models(data_dir()):
            mark = "*" if current in (spec.id, spec.repo) else " "
            print(f"{mark} {spec.id:<36}{stars(spec.accuracy):<10}{stars(spec.speed):<8}"
                  f"{spec.vram_gb:<7g}{placed(spec, device).backend:<14}{host.status(spec, device):<16}"
                  f"{spec.languages}" + ("" if spec.punctuates else "  (lowercase at times)"))
        print("\n* = selected.  speechify models use <id>  installs (if needed) and selects a model.")
        if any(not spec.punctuates for spec in all_models(data_dir())):
            print("  (lowercase at times) = sometimes writes a dictation without capitals or punctuation.")
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
