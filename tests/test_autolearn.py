"""Scheduling, placement, pre-emption and retraining, with fake engines.

The workers here are real processes running tests/fake_engine.py, so the
kill and reload paths are the ones the app uses.
"""

import importlib.util
import json
import os
import sys
import threading
import time
from pathlib import Path

import numpy as np
import pytest

from speechify import autolearn, config, gpu, power, students, teacher, versions
from speechify.engines import catalog, host
from speechify.learning import Dictation, Learner

TESTS = Path(__file__).parent


# --- fixtures ------------------------------------------------------------------


@pytest.fixture
def home(tmp_path, monkeypatch):
    monkeypatch.setenv("SPEECHIFY_HOME", str(tmp_path))
    monkeypatch.delenv("SPEECHIFY_ENGINES", raising=False)
    return tmp_path


@pytest.fixture
def fake_workers(home, monkeypatch):
    """Every worker runs this interpreter with the fake engines importable."""
    monkeypatch.setattr(host, "env_python", lambda backend: Path(sys.executable))
    monkeypatch.setattr(host, "is_ready", lambda spec: True)
    real_env = host._worker_env

    def env(*args, **kwargs):
        e = real_env(*args, **kwargs)
        e["PYTHONPATH"] += os.pathsep + str(TESTS)
        return e

    monkeypatch.setattr(host, "_worker_env", env)


def student_spec(repo="ok", id="student", finetune=False):
    return catalog.ModelSpec(id, "Fake student", "fake_engine:Echo", repo, "any", 1, 1, 2, 0,
                             prompt=True, finetune=finetune)


def teacher_spec(repo="text=the teacher heard this,logprob=-0.05"):
    return catalog.ModelSpec("fake-teacher", "Fake teacher", "fake_engine:Teacher", repo, "any", 1, 1, 6, 0)


def make_cfg(**kw):
    cfg = config.Config(idle_minutes=0, model="student", device="cpu", compute_type="int8")
    for k, v in kw.items():
        setattr(cfg, k, v)
    return cfg


def add_clips(learner, n, seconds=1.5):
    rng = np.random.default_rng(0)
    for i in range(n):
        audio = rng.uniform(-0.3, 0.3, int(seconds * 16_000)).astype(np.float32)
        learner.record(Dictation(audio, raw=f"student {i}", pasted=f"student {i}"), keep_audio=True)


class Hosts:
    """host_factory that remembers the teacher hosts it made."""

    def __init__(self):
        self.made = []

    def __call__(self):
        h = host.EngineHost()
        self.made.append(h)
        return h


def make_auto(cfg, learner, student, *, gpu_info, mains=lambda: True, hosts=None, clock=time.monotonic):
    return autolearn.AutoLearner(cfg, learner, student, log=lambda m: None, clock=clock,
                                 on_mains=mains, gpu_query=lambda: gpu_info,
                                 host_factory=hosts or Hosts())


def wait_for(cond, timeout=20.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if cond():
            return True
        time.sleep(0.01)
    return False


# --- power ---------------------------------------------------------------------------


def test_windows_power_status():
    assert power.windows_status(1, 0) is True
    assert power.windows_status(0, 0) is False
    assert power.windows_status(255, 128) is True    # unknown, and no battery: a desktop
    assert power.windows_status(255, 8) is False     # unknown, with a battery charging?
    assert power.windows_status(255, 255) is False


def _supply(root, name, **files):
    d = root / name
    d.mkdir(parents=True)
    for k, v in files.items():
        (d / k).write_text(v + "\n")


def test_linux_power(tmp_path):
    laptop = tmp_path / "laptop"
    _supply(laptop, "AC", type="Mains", online="1")
    _supply(laptop, "BAT0", type="Battery", status="Charging")
    assert power._linux(laptop) is True

    unplugged = tmp_path / "unplugged"
    _supply(unplugged, "ACAD", type="Mains", online="0")
    _supply(unplugged, "BAT1", type="Battery", status="Discharging")
    assert power._linux(unplugged) is False

    battery_only = tmp_path / "battery_only"
    _supply(battery_only, "BAT0", type="Battery", status="Discharging")
    assert power._linux(battery_only) is False

    desktop = tmp_path / "desktop"
    desktop.mkdir()
    # A wireless mouse's battery doesn't make a desktop a laptop.
    _supply(desktop, "hidpp_battery_0", type="Battery", scope="Device", status="Discharging")
    assert power._linux(desktop) is True
    assert power._linux(tmp_path / "missing") is True


def test_macos_power():
    assert power._macos("Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t100%; charged;") is True
    assert power._macos("Now drawing from 'Battery Power'\n -InternalBattery-0\t80%; discharging;") is False
    with pytest.raises(OSError):
        power._macos("")


def test_power_read_failure_means_battery(monkeypatch):
    monkeypatch.setattr(power, "_linux", lambda root: (_ for _ in ()).throw(PermissionError("no")))
    monkeypatch.setattr(power, "_windows", lambda: (_ for _ in ()).throw(OSError("no")))
    monkeypatch.setattr(power.subprocess, "run", lambda *a, **k: (_ for _ in ()).throw(OSError("no")))
    assert power.on_mains_power() is False


# --- gpu and teacher choice ------------------------------------------------------


def test_parse_nvidia_smi():
    info = gpu.parse("NVIDIA GeForce RTX 4070 Laptop GPU, 8188, 6012\n")
    assert info.name.startswith("NVIDIA") and round(info.total_gb) == 8 and 5.8 < info.free_gb < 5.9
    assert gpu.parse("") is None and gpu.parse("garbage") is None


def test_teacher_choice(tmp_path):
    big = gpu.GpuInfo("big", 16, 14)
    small = gpu.GpuInfo("small", 6.5, 5)
    tiny = gpu.GpuInfo("tiny", 4, 3)
    assert teacher.choose("auto", big, False, tmp_path)[0].id == "granite-speech-4.1-2b"
    assert teacher.choose("auto", small, False, tmp_path)[0].id == "qwen3-asr-1.7b"
    assert teacher.choose("auto", tiny, False, tmp_path)[0] is None
    assert teacher.choose("off", big, False, tmp_path)[0] is None
    assert teacher.choose("qwen3-asr-1.7b", big, False, tmp_path)[0].id == "qwen3-asr-1.7b"
    assert teacher.choose("nope", big, False, tmp_path)[0] is None


def test_teacher_never_on_cpu_unless_enabled(tmp_path):
    spec, why = teacher.choose("auto", None, False, tmp_path)
    assert spec is None and "teacher_on_cpu" in why
    spec, _ = teacher.choose("auto", None, True, tmp_path)
    assert spec.id == "qwen3-asr-1.7b"  # the smaller one, on a CPU


def test_placement():
    shared = teacher.place(6, gpu.GpuInfo("g", 24, 20))
    assert not shared.unload_student and not shared.kill_on_keydown and shared.device == "cuda"
    exclusive = teacher.place(6, gpu.GpuInfo("g", 8, 5.5))
    assert exclusive.unload_student and exclusive.kill_on_keydown
    cpu = teacher.place(6, None)
    assert cpu.device == "cpu" and cpu.kill_on_keydown and not cpu.unload_student


# --- idle and retrain scheduling -------------------------------------------------


def test_is_idle():
    assert autolearn.is_idle(now=400, last_activity=100, idle_s=300, busy=False)
    assert not autolearn.is_idle(now=399, last_activity=100, idle_s=300, busy=False)
    assert not autolearn.is_idle(now=10_000, last_activity=100, idle_s=300, busy=True)


def test_retrain_due():
    day = 86400
    assert autolearn.retrain_due(40, 40, 0, 30 * day, 300, 7)[0] is False   # too few in total
    assert autolearn.retrain_due(400, 300, 0, 1, 300, 7)[0] is True          # N new labels
    assert autolearn.retrain_due(400, 299, 0, 1, 300, 7)[0] is False
    assert autolearn.retrain_due(400, 5, 0, 7 * day, 300, 7)[0] is True      # weekly
    assert autolearn.retrain_due(400, 0, 0, 30 * day, 300, 7)[0] is False    # nothing new


def test_tick_waits_for_idle_and_mains(home, monkeypatch):
    learner = Learner(home)
    add_clips(learner, 1)
    now = [1000.0]
    mains = [True]
    auto = make_auto(make_cfg(idle_minutes=5), learner, host.EngineHost(), gpu_info=None,
                     mains=lambda: mains[0], clock=lambda: now[0])
    ran = []
    monkeypatch.setattr(auto, "teacher_pass", lambda check_idle=True: ran.append(1) or "done")

    assert auto.tick() == "active"          # just started: not idle yet
    now[0] += 299
    assert auto.tick() == "active"
    now[0] += 1
    mains[0] = False
    assert auto.tick() == "on battery"
    mains[0] = True
    auto.key_down()
    assert auto.tick() == "active"          # key held
    auto.dictation_done()
    now[0] += 300
    assert auto.tick() == "done" and ran == [1]
    auto.cfg.learn_auto = False
    assert auto.tick() == "off"


def test_cpu_only_machine_skips_the_teacher(home, fake_workers):
    learner = Learner(home)
    add_clips(learner, 2)
    auto = make_auto(make_cfg(), learner, host.EngineHost(), gpu_info=None)
    assert auto.teacher_pass() == "no teacher"
    assert learner.get_state("last_teacher_pass") is None


# --- teacher passes with real worker processes ----------------------------------


def use_teacher(monkeypatch, spec):
    monkeypatch.setattr(autolearn.teacher_mod, "choose", lambda *a: (spec, "test"))


def test_teacher_pass_next_to_student(home, fake_workers, monkeypatch):
    learner = Learner(home)
    add_clips(learner, 3)
    learner.correct(Dictation(np.zeros(1600, np.float32), "cube control", "cube control"), "kubectl")
    use_teacher(monkeypatch, teacher_spec("text=,logprob=-0.05"))  # echoes the prompt it gets
    student = host.EngineHost()
    student.start(student_spec(), {})
    auto = make_auto(make_cfg(), learner, student, gpu_info=gpu.GpuInfo("g", 24, 20))
    try:
        assert auto.teacher_pass() == "done"
        assert student.ready.is_set()  # never unloaded
    finally:
        student.stop()
    stats = learner.get_state("last_teacher_pass")
    assert stats["device"] == "cuda" and "next to" in stats["placement"]
    clips = learner.clips()
    assert all(c.teacher_ts for c in clips)
    # It got your vocabulary as its prompt.
    assert {c.teacher_text for c in clips} == {"Glossary: kubectl."}
    assert all(c.teacher_model == "fake-teacher" for c in clips)
    assert not learner.teacher_queue()


def test_hallucinations_are_stored_as_rejected(home, fake_workers, monkeypatch):
    learner = Learner(home)
    add_clips(learner, 2)
    use_teacher(monkeypatch, teacher_spec("text=thank you thank you thank you thank you,logprob=-0.01"))
    auto = make_auto(make_cfg(), learner, host.EngineHost(), gpu_info=gpu.GpuInfo("g", 24, 20))
    assert auto.teacher_pass() == "done"
    assert {c.teacher_note for c in learner.clips()} == {"repeated phrase"}
    assert learner.get_state("last_teacher_pass")["rejected"] == 2


def test_shared_teacher_pauses_while_you_dictate(home, fake_workers, monkeypatch):
    learner = Learner(home)
    add_clips(learner, 3)
    use_teacher(monkeypatch, teacher_spec("clip=0.2,logprob=-0.1"))
    auto = make_auto(make_cfg(), learner, host.EngineHost(), gpu_info=gpu.GpuInfo("g", 24, 20))
    result = []
    t = threading.Thread(target=lambda: result.append(auto.teacher_pass()))
    t.start()
    assert wait_for(lambda: auto.job is not None and auto.job.kind == "teacher pass")
    auto.key_down()  # not killed: it fits next to the student
    time.sleep(0.6)
    labelled_while_busy = sum(1 for c in learner.clips() if c.teacher_ts)
    assert labelled_while_busy <= 1  # at most the clip already in flight
    assert not auto.job.killed.is_set()
    auto.dictation_done()
    t.join(20)
    assert result == ["done"] and all(c.teacher_ts for c in learner.clips())


def test_keydown_kills_teacher_and_reloads_student(home, fake_workers, monkeypatch):
    """The GPU can't hold both: the student is unloaded for the teacher, and
    the dictation key brings it straight back."""
    learner = Learner(home)
    add_clips(learner, 3)
    use_teacher(monkeypatch, teacher_spec("clip=30,logprob=-0.1"))  # stuck on the first clip
    student = host.EngineHost()
    student.start(student_spec(), {})
    hosts = Hosts()
    auto = make_auto(make_cfg(), learner, student, gpu_info=gpu.GpuInfo("g", 8, 4), hosts=hosts)
    result = []
    t = threading.Thread(target=lambda: result.append(auto.teacher_pass()))
    t.start()
    try:
        assert wait_for(lambda: hosts.made and hosts.made[0].ready.is_set())
        assert not student.ready.is_set()  # unloaded while the teacher runs
        teacher_proc = hosts.made[0]._proc

        t0 = time.perf_counter()
        auto.key_down()
        assert wait_for(student.ready.is_set, timeout=10)
        ready_s = time.perf_counter() - t0
        assert wait_for(lambda: teacher_proc.poll() is not None, timeout=2)
        t.join(10)
        assert result == ["dictation"]
        assert student.spec.id == "student"
        assert ready_s < 5, ready_s
        # The clip in flight wasn't labelled; it's still queued for next time.
        assert len(learner.teacher_queue()) == 3
        pre = learner.get_state("last_preemption")
        assert pre["job"] == "teacher pass" and pre["ms"] >= 0
        print(f"\nkey down -> student ready: {ready_s * 1000:.0f} ms (fake engine; real load time adds to this)")
    finally:
        student.stop()


def test_keydown_while_teacher_is_still_loading(home, fake_workers, monkeypatch):
    learner = Learner(home)
    add_clips(learner, 1)
    use_teacher(monkeypatch, teacher_spec("load=30"))
    student = host.EngineHost()
    student.start(student_spec(), {})
    hosts = Hosts()
    auto = make_auto(make_cfg(), learner, student, gpu_info=gpu.GpuInfo("g", 8, 4), hosts=hosts)
    result = []
    t = threading.Thread(target=lambda: result.append(auto.teacher_pass()))
    t.start()
    try:
        assert wait_for(lambda: hosts.made and hosts.made[0]._loading is not None)
        loading = hosts.made[0]._loading
        auto.key_down()
        assert wait_for(student.ready.is_set, timeout=10)
        t.join(10)
        assert result == ["dictation"] and loading.poll() is not None
        assert not hosts.made[0].ready.is_set()  # the cancelled load can't finish later
    finally:
        student.stop()


def test_model_switch_stops_teacher_without_reloading_the_old_model(home, fake_workers, monkeypatch):
    learner = Learner(home)
    add_clips(learner, 2)
    use_teacher(monkeypatch, teacher_spec("clip=30"))
    student = host.EngineHost()
    student.start(student_spec(), {})
    hosts = Hosts()
    auto = make_auto(make_cfg(), learner, student, gpu_info=gpu.GpuInfo("g", 8, 4), hosts=hosts)
    t = threading.Thread(target=auto.teacher_pass)
    t.start()
    try:
        assert wait_for(lambda: hosts.made and hosts.made[0].ready.is_set())
        auto.preempt("model switch", restore=False)  # the app loads the new model itself
        t.join(10)
        assert not t.is_alive() and not student.ready.is_set()
    finally:
        student.stop()


def test_cpu_teacher_killed_on_keydown_without_touching_student(home, fake_workers, monkeypatch):
    learner = Learner(home)
    add_clips(learner, 2)
    use_teacher(monkeypatch, teacher_spec("clip=30"))
    student = host.EngineHost()
    student.start(student_spec(), {})
    hosts = Hosts()
    auto = make_auto(make_cfg(teacher_on_cpu=True), learner, student, gpu_info=None, hosts=hosts)
    result = []
    t = threading.Thread(target=lambda: result.append(auto.teacher_pass()))
    t.start()
    try:
        assert wait_for(lambda: hosts.made and hosts.made[0].ready.is_set())
        assert student.ready.is_set()
        auto.key_down()
        t.join(10)
        assert result == ["dictation"] and student.ready.is_set()
    finally:
        student.stop()


def test_unplugging_stops_the_teacher(home, fake_workers, monkeypatch):
    learner = Learner(home)
    add_clips(learner, 5)
    use_teacher(monkeypatch, teacher_spec("clip=0.05"))
    monkeypatch.setattr(autolearn, "POWER_CHECK_S", 0)
    checks = []
    auto = make_auto(make_cfg(), learner, host.EngineHost(), gpu_info=gpu.GpuInfo("g", 24, 20),
                     mains=lambda: checks.append(1) or len(checks) < 3)
    assert auto.teacher_pass() == "on battery"
    assert 0 < len(learner.teacher_queue()) < 5


# --- retraining ---------------------------------------------------------------------


@pytest.fixture
def fake_training(home, fake_workers, monkeypatch, tmp_path_factory):
    args_file = tmp_path_factory.mktemp("args") / "args.json"
    monkeypatch.setenv("FAKE_TRAIN_ARGS", str(args_file))
    monkeypatch.setitem(students.TRAINERS, "fake_engine:Echo", students.Trainer(
        "fake_engine:Echo", "finetune.py", vram_gb=8, bases={"student": "base/repo"}, default_base="base/repo"))
    monkeypatch.setattr(host, "find_uv", lambda: "uv")
    monkeypatch.setattr(autolearn, "training_command",
                        lambda uv, script, args: [sys.executable, str(TESTS / "fake_trainer.py"), *args])
    monkeypatch.setattr(autolearn, "resolve",
                        lambda name, d: student_spec(id=f"ft:{Path(name).name}", repo=name, finetune=True))
    config.Config.load()  # writes config.toml
    return args_file


def test_retrain_accepted_switches_and_records(home, fake_training, monkeypatch):
    monkeypatch.setenv("FAKE_TRAIN", "accept")
    learner = Learner(home)
    student = host.EngineHost()
    student.start(student_spec(finetune=True), {})
    switched = []
    auto = make_auto(make_cfg(), learner, student, gpu_info=gpu.GpuInfo("g", 8, 4))
    auto.on_switch = switched.append
    try:
        assert auto.retrain() == "accepted"
        args = json.loads(fake_training.read_text())
        assert args[args.index("--current") + 1] == "base/repo"
        assert args[args.index("--base") + 1] == "base/repo"
        new = learner.get_state("last_retrain")
        assert new["gate1"]["passed"] and new["gate2"]["passed"]
        path = new["model"]
        assert auto.cfg.model == path and student.spec.repo == path and student.ready.is_set()
        assert config.Config.load().model == path
        assert switched and versions.entries(learner)[0]["base"] == "student"
    finally:
        student.stop()


def test_retrain_rejected_keeps_current_model(home, fake_training, monkeypatch):
    monkeypatch.setenv("FAKE_TRAIN", "reject")
    learner = Learner(home)
    student = host.EngineHost()
    student.start(student_spec(finetune=True), {})
    auto = make_auto(make_cfg(), learner, student, gpu_info=gpu.GpuInfo("g", 8, 4))
    try:
        assert auto.retrain() == "rejected"
        assert student.ready.is_set() and student.spec.id == "student"  # reloaded after training
        assert auto.cfg.model == "student" and not versions.entries(learner)
        run = learner.get_state("last_retrain")
        assert run["status"] == "rejected" and not run["gate1"]["passed"]
        assert learner.get_state("retrain_mark") is not None  # counts as a run
    finally:
        student.stop()


def test_keydown_kills_training_and_reloads_student(home, fake_training, monkeypatch):
    monkeypatch.setenv("FAKE_TRAIN", "sleep")
    learner = Learner(home)
    student = host.EngineHost()
    student.start(student_spec(finetune=True), {})
    auto = make_auto(make_cfg(), learner, student, gpu_info=gpu.GpuInfo("g", 8, 4))
    result = []
    t = threading.Thread(target=lambda: result.append(auto.retrain()))
    t.start()
    try:
        assert wait_for(lambda: fake_training.exists())  # the trainer is running
        assert not student.ready.is_set()
        auto.key_down()
        assert wait_for(student.ready.is_set, timeout=10)
        t.join(10)
        assert result == ["dictation"]
        assert learner.get_state("retrain_mark") is None  # a killed run doesn't count
        assert learner.get_state("last_retrain") is None
    finally:
        student.stop()


def test_retrain_needs_a_trainable_student_and_a_gpu(home, fake_training):
    learner = Learner(home)
    student = host.EngineHost()
    student.spec = student_spec(finetune=False)
    assert make_auto(make_cfg(), learner, student, gpu_info=gpu.GpuInfo("g", 8, 4)).retrain() == "not trainable"
    student.spec = student_spec(finetune=True)
    assert make_auto(make_cfg(), learner, student, gpu_info=None).retrain() == "no gpu"


# --- versions and rollback --------------------------------------------------------------


def _version(home, i):
    path = home / "models" / f"whisper-ft-2026102{i}-0300"
    hf = home / "models" / f"hf-2026102{i}-0300"
    for d in (path, hf):
        d.mkdir(parents=True)
    (path / "model.bin").write_bytes(b"")
    return {"name": path.name, "path": str(path), "hf": str(hf), "base": "whisper-large-v3-turbo", "ts": i}


def test_keeps_last_three_versions(home):
    learner = Learner(home)
    made = [_version(home, i) for i in range(1, 5)]
    for v in made[:3]:
        assert versions.record(learner, v, keep=3, active=v["path"]) == []
    assert versions.record(learner, made[3], keep=3, active=made[3]["path"]) == [made[0]["name"]]
    assert not Path(made[0]["path"]).exists() and not Path(made[0]["hf"]).exists()
    assert [v["name"] for v in versions.entries(learner)] == [v["name"] for v in made[1:]]


def test_active_version_is_never_pruned(home):
    learner = Learner(home)
    made = [_version(home, i) for i in range(1, 5)]
    for v in made:
        versions.record(learner, v, keep=1, active=made[0]["path"])
    assert Path(made[0]["path"]).exists()
    # After a rollback to the oldest, a new version doesn't delete it.
    newest = _version(home, 5)
    versions.record(learner, newest, keep=2, active=[newest["path"], made[0]["path"]])
    assert Path(made[0]["path"]).exists() and Path(newest["path"]).exists()


def test_rollback_steps_back_to_the_base_model(home):
    learner = Learner(home)
    made = [_version(home, i) for i in range(1, 4)]
    for v in made:
        versions.record(learner, v, keep=3, active=v["path"])
    cfg = config.Config.load()
    cfg.model = made[2]["path"]
    config.set_value("model", cfg.model)

    assert versions.rollback(learner, cfg)[0] == made[1]["path"]
    assert config.Config.load().model == made[1]["path"]
    assert versions.rollback(learner, cfg)[0] == made[0]["path"]
    assert versions.rollback(learner, cfg)[0] == "whisper-large-v3-turbo"
    with pytest.raises(versions.RollbackError):
        versions.rollback(learner, cfg)


def test_rollback_skips_deleted_versions(home):
    learner = Learner(home)
    made = [_version(home, i) for i in range(1, 4)]
    for v in made:
        versions.record(learner, v, keep=3, active=v["path"])
    (Path(made[1]["path"]) / "model.bin").unlink()
    assert versions.rollback_target(learner, made[2]["path"])[0] == made[0]["path"]


def test_rollback_with_nothing_trained(home):
    with pytest.raises(versions.RollbackError, match="nothing to roll back"):
        versions.rollback_target(Learner(home), "whisper-large-v3-turbo")


def test_cli_rollback_and_status(home, monkeypatch, capsys):
    from speechify import __main__ as cli

    learner = Learner(home)
    made = [_version(home, i) for i in range(1, 3)]
    for v in made:
        versions.record(learner, v, keep=3, active=v["path"])
    learner._db.close()
    config.Config.load()
    config.set_value("model", made[1]["path"])
    monkeypatch.setattr(sys, "argv", ["speechify", "models", "--rollback"])
    cli.main()
    assert "previous version" in capsys.readouterr().out
    assert config.Config.load().model == made[0]["path"]

    monkeypatch.setattr(sys, "argv", ["speechify", "learn", "status"])
    cli.main()
    out = capsys.readouterr().out
    assert "labelled" in out and "pending" in out and "skipped" in out
    assert "Last teacher pass: never" in out and "<- in use" in out


# --- the two gates (scripts/finetune.py) ------------------------------------------------


@pytest.fixture(scope="module")
def finetune():
    spec = importlib.util.spec_from_file_location("finetune", TESTS.parent / "scripts" / "finetune.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_gate1_must_beat_the_current_model(finetune):
    assert finetune.gate1(current=0.10, candidate=0.09, held_out=20, source="corrected", base=0.12)["passed"]
    # Beating the base isn't enough when the current model is better still.
    assert not finetune.gate1(current=0.08, candidate=0.09, held_out=20, source="corrected", base=0.12)["passed"]
    assert not finetune.gate1(current=0.09, candidate=0.09, held_out=20, source="teacher", base=0.09)["passed"]


def test_gate2_allows_half_a_point_against_the_base(finetune):
    assert finetune.gate2(base=0.030, candidate=0.035, max_points=0.5, clips=50)["passed"]
    assert not finetune.gate2(base=0.030, candidate=0.0351, max_points=0.5, clips=50)["passed"]
    assert finetune.gate2(base=0.030, candidate=0.020, max_points=0.5, clips=50)["passed"]


def test_finetune_never_trains_on_pasted_text(home, finetune):
    learner = Learner(home)
    add_clips(learner, 3)
    learner.set_teacher(1, "teacher one", "t", 0.9, None)
    learner.set_teacher(2, "teacher two", "t", 0.1, None)  # not confident
    learner.edit_sample(3, "my own fix")
    train, held, _ = finetune.load_samples(home, 0.7, corrected_only=False)
    texts = sorted(t for _, t in train + held)
    assert texts == ["my own fix", "teacher one"]
    train, held, _ = finetune.load_samples(home, 0.7, corrected_only=True)
    assert [t for _, t in train + held] == ["my own fix"]
