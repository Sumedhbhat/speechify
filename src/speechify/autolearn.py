"""Getting better on its own: teacher labels and automatic retraining.

A background thread checks every TICK_S seconds. When there's been no
dictation for `idle_minutes` and the computer is on mains power, it does one
of these, in this order:

1. Teacher pass. A bigger model re-transcribes every saved recording that
   has no teacher transcript yet, newest first, with your vocabulary as its
   prompt. Each transcript is checked for hallucinations and scored
   (labels.judge) and stored next to the recording.
2. Retraining, when it's due (retrain_status): at least
   `retrain_after_labels` new labelled recordings since the last run, or
   `retrain_after_days` since then with at least one. The training script
   runs in its own environment, starts from the base model, and only saves
   a model that passes both gates. A model that does is switched to.

Dictation always comes first. See teacher.py for where the teacher runs; a
job placed with `kill_on_keydown` is killed the instant the dictation key
goes down, and if it had unloaded the dictation model, that's reloaded
straight away, while you're still speaking. Unplugging the power stops
either job within POWER_CHECK_S.
"""

from __future__ import annotations

import json
import os
import shutil
import signal
import subprocess
import sys
import threading
import time
import traceback
from pathlib import Path
from typing import Callable

from . import gpu as gpu_mod
from . import power, students, versions
from . import teacher as teacher_mod
from .audio import load_wav
from .config import Config, data_dir, set_value
from .engines import host
from .engines.catalog import ModelSpec, resolve
from .labels import classify, judge

TICK_S = 15
POWER_CHECK_S = 30
RETRY_AFTER_S = 3600     # after a failed install or run, wait before trying again
MIN_TRAIN_LABELS = 50    # same as finetune.py's --min-samples


def is_idle(now: float, last_activity: float, idle_s: float, busy: bool) -> bool:
    return not busy and now - last_activity >= idle_s


def retrain_due(total: int, new: int, last_ts: float, now: float,
                every_labels: int, every_days: float) -> tuple[bool, str]:
    """Whether to retrain, and why (or why not).

    `new` counts labelled recordings added since the last run, `last_ts` is
    when that run was (or when automatic learning was first switched on).
    """
    if total < MIN_TRAIN_LABELS:
        return False, f"{total} labelled recordings; training needs {MIN_TRAIN_LABELS}"
    if new >= every_labels:
        return True, f"{new} new labelled recordings"
    days = (now - last_ts) / 86400
    if days >= every_days and new > 0:
        return True, f"{days:.0f} days since the last run"
    return False, f"{new}/{every_labels} new labelled recordings, {days:.1f}/{every_days:g} days"


def counts(clips, min_confidence: float) -> dict[str, int]:
    """labelled / pending / skipped, plus the reason for each."""
    out = {"corrected": 0, "teacher": 0, "pending": 0, "excluded": 0, "rejected": 0,
           "low confidence": 0, "no audio": 0}
    for clip in clips:
        out[classify(clip, min_confidence)[1]] += 1
    out["labelled"] = out["corrected"] + out["teacher"]
    out["skipped"] = out["excluded"] + out["rejected"] + out["low confidence"] + out["no audio"]
    return out


class Job:
    """A running teacher pass or training run."""

    def __init__(self, kind: str, placement: teacher_mod.Placement, student_spec: ModelSpec | None,
                 restore: bool):
        self.kind, self.placement, self.student_spec = kind, placement, student_spec
        self.restore = restore  # reload the dictation model afterwards
        self.killed = threading.Event()
        self.reason: str | None = None
        self._killer: Callable[[], list] | None = None
        self._procs: list = []
        self._lock = threading.Lock()

    def set_killer(self, killer: Callable[[], list]) -> None:
        with self._lock:
            self._killer = killer
            killed = self.killed.is_set()
        if killed:  # killed before it had anything to kill
            self._procs += killer() or []

    def kill(self, reason: str) -> None:
        with self._lock:
            if self.reason is None:
                self.reason = reason
            self.killed.set()
            killer = self._killer
        if killer:
            self._procs += killer() or []

    def wait_dead(self, timeout: float = 10) -> None:
        """Wait for killed processes to exit, so their GPU memory is free."""
        deadline = time.monotonic() + timeout
        for proc in list(self._procs):
            try:
                proc.wait(timeout=max(0.1, deadline - time.monotonic()))
            except Exception:
                pass


class AutoLearner:
    def __init__(self, cfg: Config, learner, student: host.EngineHost, *,
                 student_lock: threading.Lock | None = None,
                 log: Callable[[str], None] = print,
                 on_switch: Callable[[ModelSpec], None] | None = None,
                 clock: Callable[[], float] = time.monotonic,
                 wall: Callable[[], float] = time.time,
                 on_mains: Callable[[], bool] = power.on_mains_power,
                 gpu_query: Callable[[], gpu_mod.GpuInfo | None] = gpu_mod.query,
                 host_factory: Callable[[], host.EngineHost] = host.EngineHost):
        self.cfg, self.learner, self.student = cfg, learner, student
        self.log, self.on_switch = log, on_switch
        self.clock, self.wall, self.on_mains, self.gpu_query = clock, wall, on_mains, gpu_query
        self.host_factory = host_factory
        self._student_lock = student_lock or threading.Lock()
        self._lock = threading.Lock()
        self._job: Job | None = None
        self._busy = threading.Event()
        self._last_activity = clock()
        self._retry_at: dict[str, float] = {}
        self._said: dict[str, str] = {}
        self._stopping = threading.Event()
        if learner.get_state("autolearn_since") is None:
            learner.set_state("autolearn_since", wall())

    # --- called by the app -------------------------------------------------

    def start(self) -> None:
        threading.Thread(target=self._loop, name="autolearn", daemon=True).start()

    def stop(self) -> None:
        self._stopping.set()
        job = self._job
        if job:
            job.kill("quitting")

    def key_down(self) -> None:
        """The dictation key went down. Called on the keyboard hook thread,
        so it only takes a short lock and hands the rest to a thread."""
        t0 = time.perf_counter()
        with self._lock:
            self._busy.set()
            self._last_activity = self.clock()
            job = self._job
        if job and job.placement.kill_on_keydown and not job.killed.is_set():
            threading.Thread(target=self._preempt, args=(job, "dictation", t0), daemon=True).start()

    def dictation_done(self) -> None:
        with self._lock:
            self._busy.clear()
            self._last_activity = self.clock()

    def preempt(self, reason: str, restore: bool = True) -> None:
        """Stop whatever is running, whatever its placement (e.g. you're
        switching models). Blocks until it has exited, and with `restore`,
        until the dictation model is back."""
        job = self._job
        if job is None:
            return
        if restore:
            self._preempt(job, reason, None)
        else:
            job.restore = False  # the caller loads a model itself; don't race it
            job.kill(reason)
            job.wait_dead()

    @property
    def job(self) -> Job | None:
        return self._job

    # --- scheduling ------------------------------------------------------------

    def _loop(self) -> None:
        while not self._stopping.wait(TICK_S):
            try:
                self.tick()
            except Exception:
                traceback.print_exc()

    def idle(self) -> bool:
        return is_idle(self.clock(), self._last_activity, self.cfg.idle_minutes * 60, self._busy.is_set())

    def tick(self) -> str:
        """Start whatever is due, if anything. Blocks while it runs."""
        if not self.cfg.learn_auto:
            return "off"
        if self._job is not None:
            return "running"
        if not self.idle():
            return "active"
        if not self.on_mains():
            return "on battery"
        if self.learner.teacher_queue(1) and self._retry_at.get("teacher", 0) <= self.clock():
            result = self.teacher_pass()
            if result != "no teacher":
                return result
        due, _ = self.retrain_status()
        if due and self._retry_at.get("retrain", 0) <= self.clock():
            return self.retrain()
        return "nothing to do"

    def _note(self, key: str, message: str) -> None:
        """Log `message` once, not on every tick."""
        if self._said.get(key) != message:
            self._said[key] = message
            self.log(message)

    def _register(self, job: Job, check_idle: bool = True) -> bool:
        with self._lock:
            if self._job is not None or (check_idle and not self.idle()):
                return False
            self._job = job
            return True

    def _unregister(self, job: Job) -> None:
        with self._lock:
            if self._job is job:
                self._job = None

    def _student_spec(self) -> ModelSpec | None:
        if self.student.spec is not None:
            return self.student.spec
        try:
            return resolve(self.cfg.model, data_dir())
        except KeyError:
            return None

    # --- the dictation model -------------------------------------------------

    def _unload_student(self, job: Job) -> bool:
        with self._student_lock:
            if job.killed.is_set():
                return False
            self.student.stop()
            return True

    def _restore_student(self, spec: ModelSpec | None) -> bool:
        if spec is None or self._stopping.is_set():
            return False
        with self._student_lock:
            current = self.student.spec
            if self.student.ready.is_set() and current is not None and current.id == spec.id:
                return True
            try:
                self.student.start(spec, self.cfg.engine_opts())
                return True
            except Exception as exc:
                self.log(f"Couldn't reload {spec.name}: {exc}")
                return False

    def _preempt(self, job: Job, reason: str, t0: float | None) -> None:
        job.kill(reason)
        job.wait_dead()
        if not (job.placement.unload_student and job.restore):
            return
        if self._restore_student(job.student_spec) and t0 is not None:
            ms = (time.perf_counter() - t0) * 1000
            self.log(f"Stopped the {job.kind} for dictation; {job.student_spec.name} ready "
                     f"{ms:.0f} ms after the key went down.")
            self.learner.set_state("last_preemption", {
                "ts": self.wall(), "job": job.kind, "model": job.student_spec.id, "ms": round(ms),
            })

    # --- teacher pass ----------------------------------------------------------

    def teacher_pass(self, check_idle: bool = True) -> str:
        gpu = self.gpu_query()
        spec, why = teacher_mod.choose(self.cfg.teacher, gpu, self.cfg.teacher_on_cpu, data_dir())
        if spec is None:
            self._note("teacher", f"Teacher labels paused: {why}.")
            return "no teacher"
        on_gpu = gpu is not None and spec.vram_gb + teacher_mod.HEADROOM_GB <= gpu.total_gb
        device = "cuda" if on_gpu else "cpu"
        if not host.is_ready(spec):
            try:
                self.log(f"Downloading the teacher model, {spec.name} (one time)...")
                host.install(spec, {**teacher_mod.opts(self.cfg.language, device), "fetch_only": True}, self.log)
            except Exception as exc:
                self.log(f"Couldn't install the teacher {spec.name}: {exc}")
                self._retry_at["teacher"] = self.clock() + RETRY_AFTER_S
                return "install failed"
            return "installed"  # the next tick checks you're still away first

        placement = teacher_mod.place(spec.vram_gb, gpu) if on_gpu else teacher_mod.place(0, None)
        student_spec = self._student_spec()
        job = Job("teacher pass", placement, student_spec, restore=self.student.ready.is_set())
        teacher_host = self.host_factory()
        job.set_killer(teacher_host.kill)
        if not self._register(job, check_idle):
            return "active"
        stats = {"started": self.wall(), "teacher": spec.id, "device": device, "placement": placement.why,
                 "labelled": 0, "low_confidence": 0, "rejected": 0, "errors": 0}
        stopped = "done"
        try:
            if placement.unload_student and not self._unload_student(job):
                stopped = job.reason or "dictation"
                return stopped
            self.log(f"Teacher pass: loading {spec.name} on {device}, {placement.why}.")
            teacher_host.start(spec, teacher_mod.opts(self.cfg.language, device))
            stopped = self._label_all(job, teacher_host, spec, stats)
        except host.EngineError as exc:
            stopped = job.reason if job.killed.is_set() else f"error: {exc}"
            if not job.killed.is_set():
                self._retry_at["teacher"] = self.clock() + RETRY_AFTER_S
        finally:
            if job.killed.is_set():
                job.wait_dead()
            else:
                teacher_host.stop()
            self._unregister(job)
            if placement.unload_student and job.restore:
                self._restore_student(student_spec)
            stats.update(finished=self.wall(), stopped=stopped)
            self.learner.set_state("last_teacher_pass", stats)
            self.log(f"Teacher pass {stopped}: {stats['labelled']} labelled, {stats['low_confidence']} "
                     f"low confidence, {stats['rejected']} rejected.")
        return stopped

    def _label_all(self, job: Job, teacher_host: host.EngineHost, spec: ModelSpec, stats: dict) -> str:
        checked_power = self.clock()
        while True:
            queue = self.learner.teacher_queue(25)
            if not queue:
                return "done"
            for sample_id, path in queue:
                # Next to the dictation model: let your dictation have the GPU.
                while self._busy.is_set() and not job.killed.is_set():
                    job.killed.wait(0.05)
                if job.killed.is_set():
                    return job.reason
                if self.clock() - checked_power >= POWER_CHECK_S:
                    checked_power = self.clock()
                    if not self.on_mains():
                        job.kill("on battery")
                        return "on battery"
                try:
                    audio = load_wav(path)
                except (OSError, ValueError, EOFError) as exc:
                    self.learner.set_teacher(sample_id, "", spec.id, 0.0, f"unreadable audio: {exc}"[:200])
                    stats["rejected"] += 1
                    continue
                try:
                    text, logprob = teacher_host.transcribe_scored(audio, self.learner.prompt() or None)
                except host.EngineError as exc:
                    if job.killed.is_set():
                        return job.reason
                    if not teacher_host.ready.is_set():
                        raise  # the worker died
                    self.learner.set_teacher(sample_id, "", spec.id, 0.0, f"teacher error: {exc}"[:200])
                    stats["errors"] += 1
                    continue
                result = judge(text, audio, logprob)
                self.learner.set_teacher(sample_id, text, spec.id, result.confidence, result.note)
                if result.note:
                    stats["rejected"] += 1
                elif result.confidence < self.cfg.teacher_min_confidence:
                    stats["low_confidence"] += 1
                else:
                    stats["labelled"] += 1

    # --- retraining -----------------------------------------------------------

    def retrain_status(self) -> tuple[bool, str]:
        c = counts(self.learner.clips(), self.cfg.teacher_min_confidence)
        mark = self.learner.get_state("retrain_mark") or {}
        last = mark.get("ts") or self.learner.get_state("autolearn_since") or self.wall()
        new = max(0, c["labelled"] - mark.get("labelled", 0))
        return retrain_due(c["labelled"], new, last, self.wall(),
                           self.cfg.retrain_after_labels, self.cfg.retrain_after_days)

    def retrain(self, check_idle: bool = True) -> str:
        spec = self._student_spec()
        trainer = students.trainer_for(spec)
        if trainer is None:
            name = spec.name if spec else self.cfg.model
            self._note("retrain", f"Retraining paused: {name} can't be fine-tuned (only Whisper can).")
            return "not trainable"
        gpu = self.gpu_query()
        if gpu is None:
            self._note("retrain", "Retraining paused: training needs an NVIDIA GPU.")
            return "no gpu"
        current = trainer.current_ref(spec)
        if current is None:
            self._note("retrain", f"Retraining paused: the full-precision copy of {spec.name} is missing, "
                                  "so a new model can't be compared with it.")
            return "can't score current"
        try:
            script = students.stage(trainer, host._root() / "_train")
            uv = host.find_uv()
        except (OSError, host.EngineError) as exc:
            self._note("retrain", f"Retraining paused: {exc}")
            return "no trainer"

        stamp = time.strftime("%Y%m%d-%H%M%S")
        logs = data_dir() / "logs"
        logs.mkdir(parents=True, exist_ok=True)
        report, log_path = logs / f"retrain-{stamp}.json", logs / f"retrain-{stamp}.log"
        cmd = training_command(uv, script, [
            "--auto", "--report", str(report),
            "--base", trainer.base_repo(spec), "--base-id", trainer.base_model_id(spec),
            "--current", current, "--min-confidence", str(self.cfg.teacher_min_confidence),
            "--max-regression", str(self.cfg.max_regression_wer),
        ])
        labelled = counts(self.learner.clips(), self.cfg.teacher_min_confidence)["labelled"]
        placement = teacher_mod.place(trainer.vram_gb, gpu)
        job = Job("retraining", placement, spec, restore=self.student.ready.is_set())
        if not self._register(job, check_idle):
            return "active"
        started = self.wall()
        result: dict = {}
        stopped = "done"
        try:
            if placement.unload_student and not self._unload_student(job):
                return job.reason or "dictation"
            self.log(f"Retraining {spec.name} ({placement.why}); log: {log_path}")
            with open(log_path, "w", encoding="utf-8", errors="replace") as log_file:
                proc = run_low_priority(cmd, log_file)
                job.set_killer(lambda: kill_tree(proc))
                checked_power = self.clock()
                while proc.poll() is None:
                    if job.killed.wait(1):
                        break
                    if self.clock() - checked_power >= POWER_CHECK_S:
                        checked_power = self.clock()
                        if not self.on_mains():
                            job.kill("on battery")
                if job.killed.is_set():
                    job.wait_dead()
                    stopped = job.reason
                else:
                    proc.wait()
            if not job.killed.is_set():
                try:
                    result = json.loads(report.read_text(encoding="utf-8"))
                except (OSError, ValueError):
                    result = {"status": "error", "error": f"no report; see {log_path}"}
                stopped = result.get("status", "error")
        finally:
            self._unregister(job)

        adopted = False
        if result:
            self.learner.set_state("retrain_mark", {"labelled": labelled, "ts": started})
            record = {"started": started, "finished": self.wall(), "student": spec.id, "log": str(log_path),
                      **{k: result.get(k) for k in ("status", "error", "train", "gate1", "gate2", "model")}}
            self.learner.set_state("last_retrain", record)
            if result.get("status") == "accepted":
                adopted = self._adopt(result, spec, trainer)
            else:
                self.log(f"Retraining finished: {_describe(result)}. Keeping {spec.name}.")
                if result.get("status") == "error":
                    self._retry_at["retrain"] = self.clock() + RETRY_AFTER_S
        elif job.killed.is_set():
            self.log(f"Retraining stopped ({stopped}); it'll start over next time you're away.")
        if placement.unload_student and job.restore and not adopted and not job.killed.is_set():
            self._restore_student(spec)
        return stopped

    def _adopt(self, result: dict, old: ModelSpec, trainer: students.Trainer) -> bool:
        path = result["model"]
        try:
            new = resolve(path, data_dir())
        except KeyError:
            self.log(f"Retraining saved {path}, but it can't be loaded; keeping {old.name}.")
            return False
        removed = versions.record(self.learner, {
            "name": Path(path).name, "path": path, "hf": result.get("hf"), "ts": self.wall(),
            "base": trainer.base_model_id(old), "replaced": self.cfg.model,
            "gate1": result.get("gate1"), "gate2": result.get("gate2"),
        }, self.cfg.keep_versions, active=[path, self.cfg.model])
        if self.student.ready.is_set():
            # It trained next to the dictation model and you may be back:
            # swapping models leaves a few seconds without one, so wait
            # until you're away again.
            while not self.idle() and not self._stopping.is_set():
                self._stopping.wait(5)
        if self._stopping.is_set():
            return False
        with self._student_lock:
            try:
                self.student.start(new, self.cfg.engine_opts())
            except Exception as exc:
                self.log(f"The retrained model failed to load ({exc}); keeping {old.name}.")
                self._restore_student_locked(old)
                return False
        set_value("model", path)
        self.cfg.model = path
        self.log(f"Switched to the retrained model {new.name}: {_describe(result)}."
                 + (f" Removed old versions: {', '.join(removed)}." if removed else ""))
        if self.on_switch:
            self.on_switch(new)
        return True

    def _restore_student_locked(self, spec: ModelSpec) -> None:
        try:
            self.student.start(spec, self.cfg.engine_opts())
        except Exception as exc:
            self.log(f"Couldn't reload {spec.name}: {exc}")


def _describe(result: dict) -> str:
    if result.get("status") == "error":
        return f"error: {result.get('error')}"
    parts = []
    g1, g2 = result.get("gate1") or {}, result.get("gate2") or {}
    if g1:
        parts.append(f"your clips {g1['current']:.1%} -> {g1['candidate']:.1%} WER "
                     f"({'pass' if g1['passed'] else 'fail'})")
    if g2:
        parts.append(f"LibriSpeech {g2['base']:.1%} -> {g2['candidate']:.1%} "
                     f"({'pass' if g2['passed'] else 'fail'})")
    return "; ".join(parts) or str(result.get("status"))


def training_command(uv: str, script: Path, args: list[str]) -> list[str]:
    return [uv, "run", "--script", str(script), *args]


def run_low_priority(cmd: list[str], log_file) -> subprocess.Popen:
    """Start a training run at low CPU priority, in its own process group so
    the whole tree (uv and the Python it starts) can be killed at once."""
    if sys.platform == "win32":
        flags = (subprocess.BELOW_NORMAL_PRIORITY_CLASS | subprocess.CREATE_NO_WINDOW
                 | subprocess.CREATE_NEW_PROCESS_GROUP)
        return subprocess.Popen(cmd, stdout=log_file, stderr=subprocess.STDOUT, creationflags=flags)
    if shutil.which("nice"):
        cmd = ["nice", "-n", "10", *cmd]
    return subprocess.Popen(cmd, stdout=log_file, stderr=subprocess.STDOUT, start_new_session=True)


def kill_tree(proc: subprocess.Popen) -> list:
    if proc.poll() is not None:
        return [proc]
    try:
        if sys.platform == "win32":
            subprocess.run(["taskkill", "/F", "/T", "/PID", str(proc.pid)], capture_output=True,
                           creationflags=subprocess.CREATE_NO_WINDOW)
        else:
            os.killpg(proc.pid, signal.SIGKILL)
    except OSError:
        pass
    try:
        proc.kill()
    except OSError:
        pass
    return [proc]


# --- `speechify learn status` ------------------------------------------------------


def _when(ts: float | None) -> str:
    return time.strftime("%Y-%m-%d %H:%M", time.localtime(ts)) if ts else "never"


def _gate_lines(run: dict) -> list[str]:
    lines = []
    g1, g2 = run.get("gate1"), run.get("gate2")
    if g1:
        lines.append(f"  gate 1, your audio ({g1['held_out']} {g1['source']} held out): "
                     f"{g1['candidate']:.1%} vs {g1['current']:.1%} for the current model -> "
                     f"{'pass' if g1['passed'] else 'fail'}")
    if g2:
        lines.append(f"  gate 2, LibriSpeech ({g2['clips']} clips): {g2['candidate']:.1%} vs "
                     f"{g2['base']:.1%} for the base model, {g2['max_increase'] * 100:g} points allowed -> "
                     f"{'pass' if g2['passed'] else 'fail'}")
    return lines


def status_text(cfg: Config, learner, on_mains: Callable[[], bool] = power.on_mains_power) -> str:
    from .metrics import wer

    clips = learner.clips()
    c = counts(clips, cfg.teacher_min_confidence)
    lines = [
        f"Automatic learning: {'on' if cfg.learn_auto else 'off'} (teacher {cfg.teacher!r}, after "
        f"{cfg.idle_minutes:g} min idle, on mains power; now {'plugged in' if on_mains() else 'on battery'})",
        "",
        f"Training labels (teacher confidence >= {cfg.teacher_min_confidence:g}):",
        f"  labelled {c['labelled']:>6}  ({c['corrected']} your corrections, {c['teacher']} teacher)",
        f"  pending  {c['pending']:>6}  (waiting for the teacher)",
        f"  skipped  {c['skipped']:>6}  ({c['excluded']} excluded, {c['rejected']} rejected as hallucinations, "
        f"{c['low confidence']} low confidence, {c['no audio']} without audio)",
    ]
    checked = [x for x in clips if x.corrected and x.teacher_text and not x.teacher_note]
    if checked:
        lines.append(f"  teacher vs your corrections: {wer([x.corrected for x in checked], [x.teacher_text for x in checked]):.1%} "
                     f"WER on {len(checked)} recordings")

    tp = learner.get_state("last_teacher_pass")
    lines.append("")
    if tp:
        lines.append(f"Last teacher pass: {_when(tp.get('started'))}, {tp['teacher']} on {tp['device']} "
                     f"({tp['placement']}): {tp.get('stopped')}, {tp['labelled']} labelled, "
                     f"{tp['low_confidence']} low confidence, {tp['rejected']} rejected")
    else:
        lines.append("Last teacher pass: never")
    pre = learner.get_state("last_preemption")
    if pre:
        lines.append(f"Last time dictation stopped it: {_when(pre['ts'])}, {pre['model']} ready "
                     f"{pre['ms']} ms after the key went down")

    run = learner.get_state("last_retrain")
    if run:
        status = run.get("status")
        extra = f": {run.get('error')}" if status == "error" else ""
        lines.append(f"Last retrain: {_when(run.get('started'))}, {status}{extra}")
        lines += _gate_lines(run)
    else:
        lines.append("Last retrain: never")
    mark = learner.get_state("retrain_mark") or {}
    last = mark.get("ts") or learner.get_state("autolearn_since") or time.time()
    _, why = retrain_due(c["labelled"], max(0, c["labelled"] - mark.get("labelled", 0)), last, time.time(),
                         cfg.retrain_after_labels, cfg.retrain_after_days)
    lines.append(f"Next retrain: {why}")

    vs = versions.entries(learner)
    lines += ["", f"Model in use: {cfg.model}"]
    if vs:
        lines.append("Retrained versions (newest last):")
        for v in vs:
            mark = " <- in use" if Path(v["path"]).name == Path(cfg.model).name else ""
            lines.append(f"  {v['name']}  from {v['base']}{mark}")
        lines.append("`speechify models --rollback` goes back one version.")
    return "\n".join(lines)
