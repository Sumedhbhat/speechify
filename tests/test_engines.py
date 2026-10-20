import os
import sys
import tomllib
from pathlib import Path

import numpy as np
import pytest

from speechify import config
from speechify.engines import backends, catalog, host


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
