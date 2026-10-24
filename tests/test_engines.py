import os
import sys
import tomllib
from pathlib import Path
from types import SimpleNamespace

import numpy as np
import pytest

from speechify import config
from speechify.engines import backends, catalog, host


@pytest.fixture
def onnx_runtime(monkeypatch):
    """Exercise provider and thread setup without loading real model weights."""
    calls = []
    pools = []

    class Options:
        intra_op_num_threads = 0
        inter_op_num_threads = 0
        use_per_session_threads = True

    ort = SimpleNamespace(
        SessionOptions=Options,
        get_available_providers=lambda: ["CPUExecutionProvider"],
        set_global_thread_pool_sizes=lambda **kwargs: pools.append(kwargs),
    )
    model = SimpleNamespace(recognize=lambda *args, **kwargs: "transcript")

    def load(*args, **kwargs):
        calls.append(kwargs)
        return model

    monkeypatch.setitem(sys.modules, "onnxruntime", ort)
    monkeypatch.setitem(sys.modules, "onnx_asr", SimpleNamespace(load_model=load))
    monkeypatch.setattr(os, "sched_getaffinity", lambda pid: set(range(8)), raising=False)
    return ort, calls, pools


def test_parakeet_shares_bounded_cpu_pool(onnx_runtime):
    _, calls, pools = onnx_runtime
    engine = backends.OnnxEngine("nemo-parakeet-tdt-0.6b-v3", {
        "device": "cpu", "compute_type": "int8", "language": "en",
    })
    assert pools == [{"intra_op_num_threads": 4, "inter_op_num_threads": 1}]
    options = calls[0]["sess_options"]
    assert not options.use_per_session_threads
    assert calls[0]["quantization"] == "int8"
    assert calls[0]["providers"] == ["CPUExecutionProvider"]
    assert engine.transcribe(np.zeros(10, dtype=np.float32), None) == "transcript"


def test_onnx_threads_respects_process_affinity(onnx_runtime, monkeypatch):
    ort, _, pools = onnx_runtime
    monkeypatch.setattr(os, "sched_getaffinity", lambda pid: {2, 3})
    options = backends._onnx_cpu_options(ort, 4)
    assert not options.use_per_session_threads
    assert pools[0]["intra_op_num_threads"] == 2


def test_onnx_threads_without_affinity_api(onnx_runtime, monkeypatch):
    ort, _, pools = onnx_runtime
    monkeypatch.delattr(os, "sched_getaffinity")
    monkeypatch.setattr(os, "cpu_count", lambda: None)
    backends._onnx_cpu_options(ort, 4)
    assert pools[0]["intra_op_num_threads"] == 1


def test_zero_onnx_threads_restores_runtime_defaults(onnx_runtime):
    ort, _, pools = onnx_runtime
    options = backends._onnx_cpu_options(ort, 0)
    assert options.intra_op_num_threads == options.inter_op_num_threads == 0
    assert options.use_per_session_threads and not pools


@pytest.mark.parametrize("supported", [False, True], ids=["older-runtime", "initialized-environment"])
def test_onnx_pool_fallback(onnx_runtime, supported):
    ort, _, _ = onnx_runtime
    if supported:
        def initialized(**kwargs):
            raise RuntimeError("environment already initialized")
        ort.set_global_thread_pool_sizes = initialized
    else:
        del ort.set_global_thread_pool_sizes
    options = backends._onnx_cpu_options(ort, 4)
    assert options.use_per_session_threads
    assert options.intra_op_num_threads == 4


def test_accelerated_onnx_keeps_runtime_options(onnx_runtime):
    ort, calls, pools = onnx_runtime
    ort.get_available_providers = lambda: ["CUDAExecutionProvider", "CPUExecutionProvider"]
    backends.OnnxEngine("nemo-parakeet-tdt-0.6b-v3", {
        "device": "cuda", "compute_type": "float16", "language": "en",
    })
    assert calls[0]["providers"][0] == "CUDAExecutionProvider"
    assert "sess_options" not in calls[0] and not pools


def test_failed_gpu_load_uses_cpu_thread_options(onnx_runtime, monkeypatch):
    ort, calls, pools = onnx_runtime
    ort.get_available_providers = lambda: ["CUDAExecutionProvider", "CPUExecutionProvider"]
    original = sys.modules["onnx_asr"].load_model

    def load(*args, **kwargs):
        if kwargs["providers"][0] == "CUDAExecutionProvider":
            raise RuntimeError("driver unavailable")
        return original(*args, **kwargs)

    monkeypatch.setattr(sys.modules["onnx_asr"], "load_model", load)
    engine = backends.OnnxEngine("nemo-parakeet-tdt-0.6b-v3", {
        "device": "cuda", "compute_type": "float16", "language": "en",
    })
    assert engine.device == "CPUExecutionProvider"
    assert calls[0]["providers"] == ["CPUExecutionProvider"]
    assert not calls[0]["sess_options"].use_per_session_threads and pools


def test_other_onnx_models_keep_thread_defaults(onnx_runtime):
    _, calls, pools = onnx_runtime
    backends.OnnxEngine("nemo-canary-1b-v2", {"device": "cpu", "language": "en"})
    assert "sess_options" not in calls[0] and not pools


@pytest.mark.parametrize("threads", [-1, True, 1.5, "4"])
def test_invalid_onnx_thread_count(onnx_runtime, threads):
    with pytest.raises(ValueError, match="onnx_threads"):
        backends.OnnxEngine("nemo-parakeet-tdt-0.6b-v3", {
            "device": "cpu", "language": "en", "onnx_threads": threads,
        })


def test_saved_onnx_threads_reach_worker(data):
    config.Config.load()
    config.set_value("onnx_threads", 0)
    assert config.Config.load().engine_opts()["onnx_threads"] == 0


@pytest.fixture
def data(tmp_path, monkeypatch):
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    monkeypatch.setenv("APPDATA", str(tmp_path))
    return config.data_dir()


def test_every_model_has_a_backend():
    for spec in catalog.MODELS.values():
        assert spec.backend in catalog.BACKENDS
        assert spec.backend in backends.BACKEND_ENGINES
        assert 0 < spec.accuracy <= 5 and 0 < spec.speed <= 5


def test_resolve(data):
    assert catalog.resolve("large-v3-turbo", data).id == "whisper-large-v3-turbo"
    assert catalog.resolve("parakeet-tdt-0.6b-v3", data).backend == "onnx"
    with pytest.raises(KeyError):
        catalog.resolve("nope", data)

    ft = data / "models" / "whisper-ft-20261021-0900"
    ft.mkdir(parents=True)
    (ft / "model.bin").write_bytes(b"")
    spec = catalog.resolve(ft.as_posix(), data)
    assert spec.id == "ft:whisper-ft-20261021-0900" and spec.backend == "whisper"
    assert catalog.all_models(data)[0].id == spec.id


def test_stars():
    assert catalog.stars(3.5) == "★★★½" and catalog.stars(5) == "★★★★★"


def test_set_value_keeps_comments(data):
    config.Config.load()
    config.set_value("model", r"C:\Users\me\models\whisper-ft")
    config.set_value("keep_all_audio", False)
    config.set_value("brand_new", 3)
    text = (data / "config.toml").read_text(encoding="utf-8")
    assert "# Hold this key to talk" in text
    parsed = tomllib.loads(text)
    assert parsed["model"] == r"C:\Users\me\models\whisper-ft"
    assert parsed["keep_all_audio"] is False and parsed["brand_new"] == 3


def test_status(data):
    spec = catalog.MODELS["parakeet-tdt-0.6b-v3"]
    assert host.status(spec) == "not installed"


def test_resample():
    out = backends.resample(np.ones(16_000, dtype=np.float32), 24_000)
    assert out.size == 24_000 and out.dtype == np.float32


@pytest.fixture
def fake_worker(data, monkeypatch):
    """Run the real worker process, with this interpreter and a fake engine."""
    monkeypatch.setattr(host, "env_python", lambda backend: Path(sys.executable))
    monkeypatch.setattr(host, "is_ready", lambda spec: True)
    real_env = host._worker_env

    def env(*args, **kwargs):
        e = real_env(*args, **kwargs)
        e["PYTHONPATH"] += os.pathsep + str(Path(__file__).parent)
        return e

    monkeypatch.setattr(host, "_worker_env", env)

    def spec(repo="ok"):
        return catalog.ModelSpec("fake", "Fake", "fake_engine:Echo", repo, "any", 1, 1, 0, 0)

    return spec


def test_worker_protocol(fake_worker):
    engine = host.EngineHost()
    info = engine.start(fake_worker(), {"device": "cpu"})
    assert info["event"] == "ready" and info["device"] == "test"
    try:
        text, ms = engine.transcribe(np.full(16_000, 0.5, dtype=np.float32), "Glossary: x.")
        assert text == "16000 samples, sum 8000.0, prompt Glossary: x." and ms >= 0
        # An error in one clip is reported, and the worker keeps serving.
        with pytest.raises(host.EngineError, match="bad clip"):
            engine.transcribe(np.zeros(3, dtype=np.float32), None)
        assert engine.transcribe(np.zeros(10, dtype=np.float32), None)[0].startswith("10 samples")
    finally:
        engine.stop()
    assert not engine.ready.is_set()


def test_worker_load_failure(fake_worker):
    engine = host.EngineHost()
    with pytest.raises(host.EngineError, match="could not load"):
        engine.start(fake_worker("fail"), {"device": "cpu"})


def test_whisper_prompt_always_has_punctuated_text_in_english():
    assert backends.whisper_prompt(None, "en") == backends.WHISPER_STYLE
    assert backends.whisper_prompt("", "en") == backends.WHISPER_STYLE
    assert backends.whisper_prompt("Glossary: Sumedh.", "en") == f"{backends.WHISPER_STYLE} Glossary: Sumedh."
    # Other languages and auto-detect: an English sentence could pull them towards English.
    assert backends.whisper_prompt(None, None) is None
    assert backends.whisper_prompt("Glossary: Sumedh.", "de") == "Glossary: Sumedh."


def test_whisper_prompt_fits_with_the_longest_glossary(tmp_path):
    from speechify.learning import MAX_PROMPT_CHARS, Learner

    learner = Learner(tmp_path, autocorrect_after=1)
    for i in range(200):
        learner._db.execute("INSERT INTO vocab (term, count) VALUES (?, 1)", (f"Term{i:03d}",))
    learner._reload()
    assert len(backends.whisper_prompt(learner.prompt(), "en")) <= MAX_PROMPT_CHARS
