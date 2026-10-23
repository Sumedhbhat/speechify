"""GPU choice, transcribe.cpp (ggml) placement, installs and CPU fallback.

None of this needs a GPU: the native library and the network are faked.
"""

import hashlib
import json
import sys
import types
from pathlib import Path

import numpy as np
import pytest

from speechify import config
from speechify.engines import backends, catalog, host


@pytest.fixture
def machine(monkeypatch, tmp_path):
    """A fake machine: set .platform, .arch, files under .system32, .render_node, .vulkan."""
    for var in ("SPEECHIFY_GPU", "SPEECHIFY_CUDA"):
        monkeypatch.delenv(var, raising=False)
    system32 = tmp_path / "System32"
    dri = tmp_path / "dri"
    system32.mkdir()
    dri.mkdir()
    state = types.SimpleNamespace(system32=system32, dri=dri, vulkan=None)
    monkeypatch.setattr(config, "_system32", lambda: system32)
    monkeypatch.setattr(config, "DRI", dri)
    monkeypatch.setattr(config, "_find_library", lambda name: state.vulkan if name == "vulkan" else None)
    monkeypatch.setattr(config.Path, "exists", _no_nvidia_proc(config.Path.exists))
    config._probe_gpu.cache_clear()
    yield state, monkeypatch
    config._probe_gpu.cache_clear()


def _no_nvidia_proc(real):
    def exists(self):
        return False if str(self) == "/proc/driver/nvidia/version" else real(self)
    return exists


def accel(state, monkeypatch, platform, arch="x86_64"):
    monkeypatch.setattr(sys, "platform", platform)
    monkeypatch.setattr(config.platform, "machine", lambda: arch)
    config._probe_gpu.cache_clear()
    return config.accelerator()


def test_nvidia_keeps_cuda(machine):
    state, mp = machine
    (state.system32 / "nvcuda.dll").write_bytes(b"")
    (state.system32 / "vulkan-1.dll").write_bytes(b"")
    assert accel(state, mp, "win32") == "cuda"


def test_windows_amd_or_intel_uses_vulkan(machine):
    state, mp = machine
    assert accel(state, mp, "win32") == "cpu"
    (state.system32 / "vulkan-1.dll").write_bytes(b"")
    assert accel(state, mp, "win32") == "vulkan"
    assert not config.has_cuda()


def test_linux_needs_a_render_node_and_the_vulkan_loader(machine):
    state, mp = machine
    assert accel(state, mp, "linux") == "cpu"
    (state.dri / "renderD128").write_bytes(b"")
    assert accel(state, mp, "linux") == "cpu"  # no libvulkan.so.1
    state.vulkan = "libvulkan.so.1"
    assert accel(state, mp, "linux") == "vulkan"


def test_macs(machine):
    state, mp = machine
    assert accel(state, mp, "darwin", "arm64") == "metal"
    assert accel(state, mp, "darwin", "x86_64") == "cpu"  # transcribe.cpp has no Metal build for Intel


def test_overrides(machine):
    state, mp = machine
    mp.setenv("SPEECHIFY_GPU", "cpu")
    (state.system32 / "vulkan-1.dll").write_bytes(b"")
    assert accel(state, mp, "win32") == "cpu"
    mp.setenv("SPEECHIFY_GPU", "bogus")
    mp.setenv("SPEECHIFY_CUDA", "1")
    assert accel(state, mp, "win32") == "cuda"


def test_auto_device_resolves(monkeypatch):
    monkeypatch.setattr(config, "accelerator", lambda: "vulkan")
    cfg = config.Config(device="auto")
    assert cfg.engine_opts()["device"] == "vulkan"
    assert config.Config(device="cpu").engine_opts()["device"] == "cpu"
    assert config.Config(device="nonsense").resolved_device() == "vulkan"


@pytest.mark.parametrize("old", config._OLD_DEVICE_LINES)
def test_untouched_old_device_line_becomes_auto(monkeypatch, tmp_path, old):
    monkeypatch.setenv("SPEECHIFY_HOME", str(tmp_path))
    (tmp_path / "config.toml").write_text(f'model = "moonshine-base"\n{old}\nbeam_size = 1\n', encoding="utf-8")
    cfg = config.Config.load()
    assert cfg.device == "auto" and cfg.model == "moonshine-base" and cfg.beam_size == 1
    assert 'device = "auto"' in (tmp_path / "config.toml").read_text(encoding="utf-8")


def test_a_chosen_device_is_kept(monkeypatch, tmp_path):
    monkeypatch.setenv("SPEECHIFY_HOME", str(tmp_path))
    (tmp_path / "config.toml").write_text('device = "cpu"  # I want the CPU\n', encoding="utf-8")
    assert config.Config.load().device == "cpu"


# --- placement -------------------------------------------------------------------


def test_models_with_a_gguf_run_on_ggml_on_vulkan_and_metal():
    parakeet = catalog.MODELS["parakeet-tdt-0.6b-v3"]
    for device in ("vulkan", "metal"):
        p = catalog.placed(parakeet, device)
        assert p.backend == "ggml" and p.origin == "onnx" and p.id == parakeet.id
        assert p.repo == "handy-computer/parakeet-tdt-0.6b-v3-gguf" and p.download_gb == 0.7
    for device in ("cuda", "cpu", None):
        assert catalog.placed(parakeet, device) is parakeet
    kyutai = catalog.MODELS["kyutai-stt-1b"]  # no GGUF: stays on its own backend (on the CPU)
    assert catalog.placed(kyutai, "vulkan") is kyutai
    with pytest.raises(KeyError):
        catalog.as_ggml(kyutai)
    ggml = catalog.placed(parakeet, "vulkan")
    assert catalog.placed(ggml, "vulkan") == ggml and catalog.placed(ggml, "cpu") == ggml


def test_requested_models_have_pinned_q8_ggufs():
    wanted = {"parakeet-tdt-0.6b-v3", "parakeet-tdt-0.6b-v2", "canary-1b-v2", "whisper-large-v3-turbo",
              "qwen3-asr-0.6b", "qwen3-asr-1.7b", "cohere-transcribe", "moonshine-base"}
    have = {m.id for m in catalog.MODELS.values() if m.gguf}
    assert wanted <= have
    for spec in catalog.MODELS.values():
        g = spec.gguf
        if g:
            assert g.repo.startswith("handy-computer/") and g.file.endswith("-Q8_0.gguf")
            assert len(g.revision) == 40 and len(g.sha256) == 64 and g.size > 10_000_000


def test_gated_cohere_needs_no_login_on_ggml():
    assert catalog.MODELS["cohere-transcribe"].gated
    assert not catalog.as_ggml(catalog.MODELS["cohere-transcribe"]).gated


def test_whisper_on_ggml_still_trains_like_whisper():
    from speechify import students

    spec = catalog.placed(catalog.MODELS["whisper-large-v3-turbo"], "vulkan")
    assert students.trainer_for(spec) is students.TRAINERS["whisper"]


# --- installs --------------------------------------------------------------------


def test_transcribe_cpp_is_pinned_with_hashes():
    ggml = catalog.BACKENDS["ggml"]
    cpu = ggml.requirements_for(cuda=False)
    pinned = [r for r in cpu if r.startswith("transcribe-cpp")]
    assert len(pinned) == 2 and all("==0.3.1 --hash=sha256:" in r for r in pinned)
    assert not any("cu12" in r for r in cpu)
    cuda = [r for r in ggml.requirements_for(cuda=True) if "cu12" in r]
    assert {r.rsplit("sys_platform == ", 1)[1] for r in cuda} == {"'win32'", "'linux'"}
    assert all("releases/download/v0.3.1/" in r and "#sha256=" in r for r in cuda)


def test_install_passes_hashes_in_a_requirements_file(monkeypatch, tmp_path):
    monkeypatch.setenv("SPEECHIFY_ENGINES", str(tmp_path))
    monkeypatch.setattr(host, "has_cuda", lambda: False)
    monkeypatch.setattr(host, "find_uv", lambda: "uv")
    commands = []

    def run(cmd, log, env=None):
        commands.append((cmd, env))
        if cmd[1] == "venv":
            host.env_python("ggml").parent.mkdir(parents=True, exist_ok=True)
            host.env_python("ggml").write_bytes(b"")

    monkeypatch.setattr(host, "_run", run)
    spec = catalog.MODELS["parakeet-tdt-0.6b-v3"]
    host.install(spec, {"device": "vulkan", "language": "en"}, log=lambda line: None)
    venv, pip, download = commands
    assert pip[0][pip[0].index("--exclude-newer") + 1] == host.MIN_PACKAGE_AGE
    reqs = Path(pip[0][pip[0].index("-r") + 1]).read_text(encoding="utf-8")
    assert "transcribe-cpp==0.3.1 --hash=sha256:" in reqs and "huggingface_hub" in reqs
    worker = json.loads(download[1]["SPEECHIFY_WORKER"])
    assert worker["backend"] == "ggml" and worker["opts"]["gguf"]["sha256"] == spec.gguf.sha256
    # The GGUF and the ONNX weights are separate downloads.
    assert host.is_ready(spec, "vulkan") and not host.is_ready(spec, "cpu")
    assert host.status(spec, "vulkan") == "ready" and host.status(spec, "cpu") == "not installed"


def test_fetch_gguf_checks_the_hash(monkeypatch, tmp_path):
    payload = b"GGUF fake weights"
    calls = []

    def download(repo, file, revision, local_dir):
        calls.append(revision)
        Path(local_dir).mkdir(parents=True, exist_ok=True)
        (Path(local_dir) / file).write_bytes(payload)

    monkeypatch.setitem(sys.modules, "huggingface_hub", types.SimpleNamespace(hf_hub_download=download))
    good = {"repo": "handy-computer/x-gguf", "file": "x-Q8_0.gguf", "revision": "r1",
            "sha256": hashlib.sha256(payload).hexdigest(), "size": len(payload)}
    path = backends.fetch_gguf(good, str(tmp_path))
    assert path.read_bytes() == payload and calls == ["r1"]
    assert backends.fetch_gguf(good, str(tmp_path)) == path and calls == ["r1"]  # checked once, no network

    bad = {**good, "file": "y-Q8_0.gguf", "sha256": "0" * 64}
    with pytest.raises(ValueError, match="checksum"):
        backends.fetch_gguf(bad, str(tmp_path))
    assert not (path.parent / "y-Q8_0.gguf").exists()


# --- GgmlEngine with a fake transcribe.cpp ---------------------------------------


class FakeTC(types.ModuleType):
    """Enough of transcribe_cpp for GgmlEngine. `fail` lists what breaks:
    "load:vulkan", "run:vulkan", or "software" (Vulkan is only a CPU renderer)."""

    class TranscribeError(Exception): ...
    class InvalidArgument(TranscribeError): ...
    class UnsupportedRequest(TranscribeError): ...
    class InputTooLong(TranscribeError): ...

    def __init__(self, fail=(), max_audio_ms=0, features=()):
        super().__init__("transcribe_cpp")
        self.fail, self.max_audio_ms, self.features = set(fail), max_audio_ms, set(features)
        self.runs = []
        tc = self

        class Device:
            def __init__(self, kind, device_type, description, memory=0):
                self.kind, self.device_type, self.description, self.memory_total = kind, device_type, description, memory

        class Session:
            def __init__(self, backend):
                self.backend = backend

            def run(self, pcm, **kwargs):
                if f"run:{self.backend}" in tc.fail:
                    raise tc.TranscribeError("vk::DeviceLostError")
                tc.runs.append((self.backend, pcm.size, kwargs))
                token = types.SimpleNamespace(p=0.5)
                return types.SimpleNamespace(text=f" {pcm.size} on {self.backend} ", tokens=[token, token])

            def close(self): ...

        class Model:
            def __init__(self, path, backend, device):
                if f"load:{backend}" in tc.fail:
                    raise tc.TranscribeError("vkCreateInstance failed")
                self.backend, self.device = backend, device or Device("cpu", "cpu", "Ryzen")
                self.capabilities = types.SimpleNamespace(languages=("en", "de"), max_audio_ms=tc.max_audio_ms)

            def session(self):
                return Session(self.backend)

            def supports(self, feature):
                return feature in tc.features

            def close(self): ...

        self.Device, self.Model = Device, Model
        self.WhisperRunOptions = lambda initial_prompt: ("whisper", initial_prompt)

    def backends(self):
        if "software" in self.fail:
            return [self.Device("vulkan", "cpu", "llvmpipe"), self.Device("cpu", "cpu", "Ryzen")]
        return [self.Device("vulkan", "igpu", "Radeon Vega 8", 4), self.Device("vulkan", "gpu", "RX 7600", 8),
                self.Device("cpu", "cpu", "Ryzen")]


def ggml_engine(monkeypatch, tmp_path, device="vulkan", **fake):
    tc = FakeTC(**fake)
    monkeypatch.setitem(sys.modules, "transcribe_cpp", tc)
    monkeypatch.setattr(backends, "fetch_gguf", lambda gguf, models_dir: tmp_path / "m.gguf")
    opts = {"device": device, "language": "en", "gguf": {}, "models_dir": str(tmp_path)}
    return backends.GgmlEngine("repo", opts), tc


def test_ggml_prefers_the_discrete_gpu(monkeypatch, tmp_path):
    engine, tc = ggml_engine(monkeypatch, tmp_path)
    assert engine.device == "Vulkan: RX 7600"
    text, logprob = engine.transcribe_scored(np.zeros(100, np.float32), None)
    assert text == "100 on vulkan" and logprob == pytest.approx(np.log(0.5))
    assert tc.runs[-1][2]["language"] == "en"


@pytest.mark.parametrize("fail", [("load:vulkan",), ("software",)])
def test_ggml_falls_back_to_the_cpu_when_the_gpu_wont_start(monkeypatch, tmp_path, capsys, fail):
    engine, _ = ggml_engine(monkeypatch, tmp_path, fail=fail)
    assert engine.device == "CPU (transcribe.cpp; vulkan failed to start)"
    assert engine.transcribe(np.zeros(5, np.float32), None) == "5 on cpu"
    assert "using the CPU instead" in capsys.readouterr().err


def test_ggml_reruns_on_the_cpu_when_a_gpu_run_fails(monkeypatch, tmp_path, capsys):
    engine, tc = ggml_engine(monkeypatch, tmp_path, fail=("run:vulkan",))
    assert engine.transcribe(np.zeros(7, np.float32), None) == "7 on cpu"
    assert engine.device == "CPU (transcribe.cpp; vulkan failed during a run)"
    assert "switching to the CPU" in capsys.readouterr().err
    assert engine.transcribe(np.zeros(8, np.float32), None) == "8 on cpu"  # stays there


def test_ggml_request_errors_are_not_gpu_errors(monkeypatch, tmp_path):
    engine, tc = ggml_engine(monkeypatch, tmp_path)

    def bad(pcm, **kwargs):
        raise tc.InvalidArgument("language 'xx' unsupported")

    engine.session.run = bad
    with pytest.raises(tc.InvalidArgument):
        engine.transcribe(np.zeros(5, np.float32), None)
    assert engine.backend == "vulkan"


def test_ggml_cpu_device_never_touches_the_gpu(monkeypatch, tmp_path):
    engine, tc = ggml_engine(monkeypatch, tmp_path, device="cpu", fail=("load:vulkan",))
    assert engine.device == "CPU (transcribe.cpp)"


@pytest.mark.parametrize("features,expect", [
    ({"vocabulary"}, ("vocabulary", ["Speechify", "Sumedh"])),
    ({"initial_prompt"}, ("family", ("whisper", "Glossary: Speechify, Sumedh."))),
    ({"context_prompt"}, ("prompt", "Glossary: Speechify, Sumedh.")),
    (set(), None),
])
def test_ggml_vocabulary_hints(monkeypatch, tmp_path, features, expect):
    engine, tc = ggml_engine(monkeypatch, tmp_path, features=features)
    engine.transcribe(np.zeros(5, np.float32), "Glossary: Speechify, Sumedh.")
    kwargs = tc.runs[-1][2]
    if expect:
        assert kwargs[expect[0]] == expect[1]
    assert not ({"vocabulary", "family", "prompt"} - ({expect[0]} if expect else set())) & set(kwargs)


def test_ggml_splits_audio_longer_than_the_model_takes(monkeypatch, tmp_path):
    engine, tc = ggml_engine(monkeypatch, tmp_path, max_audio_ms=10_000)
    audio = np.random.default_rng(0).normal(0, 0.1, 16_000 * 25).astype(np.float32)
    audio[16_000 * 9:16_000 * 9 + 1600] = 0  # a pause where the first cut should go
    engine.transcribe(audio, None)
    sizes = [n for _, n, _ in tc.runs]
    assert sum(sizes) == audio.size and max(sizes) <= 160_000 and len(sizes) == 3
    assert 16_000 * 9 <= sizes[0] <= 16_000 * 9 + 1600


# --- the host restarts a crashed GPU worker on the CPU --------------------------


@pytest.fixture
def crashy(tmp_path, monkeypatch):
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    monkeypatch.setenv("APPDATA", str(tmp_path))
    monkeypatch.setattr(host, "env_python", lambda backend: Path(sys.executable))
    monkeypatch.setattr(host, "is_ready", lambda spec, device=None: True)
    monkeypatch.setattr(host, "falls_back_to_cpu", lambda spec, opts: opts["device"] != "cpu")
    real_env = host._worker_env

    def env(*args, **kwargs):
        e = real_env(*args, **kwargs)
        e["PYTHONPATH"] += __import__("os").pathsep + str(Path(__file__).parent)
        return e

    monkeypatch.setattr(host, "_worker_env", env)
    return lambda repo: catalog.ModelSpec("crashy", "Crashy", "fake_engine:GpuCrash", repo, "any", 1, 1, 0, 0)


def test_worker_that_dies_loading_on_the_gpu_restarts_on_the_cpu(crashy, capsys):
    engine = host.EngineHost()
    try:
        info = engine.start(crashy("load"), {"device": "vulkan"})
        assert info["device"] == "cpu" and engine.opts["device"] == "cpu"
        assert "trying the CPU instead" in capsys.readouterr().err
    finally:
        engine.stop()


def test_worker_that_dies_mid_dictation_reruns_it_on_the_cpu(crashy, capsys):
    engine = host.EngineHost()
    try:
        assert engine.start(crashy("run"), {"device": "vulkan"})["device"] == "vulkan"
        text, _ = engine.transcribe(np.zeros(20, np.float32), None)
        assert text == "20 samples on cpu" and engine.info["device"] == "cpu"
        assert "reloading it on the CPU" in capsys.readouterr().err
    finally:
        engine.stop()
