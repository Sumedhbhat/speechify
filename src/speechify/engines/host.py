"""App side of the engines: per-backend environments and the worker process.

Layout under the data directory:

    engines/
      _shim/speechify/engines/*.py   worker code, importable from any env
      whisper/ onnx/ transformers/ qwen/ ggml/   one virtualenv per backend
      models/                         ONNX and GGUF weights (the rest use the HF cache)
      downloaded.json                 models whose weights are on disk
"""

from __future__ import annotations

import dataclasses
import hashlib
import json
import os
import shutil
import subprocess
import sys
import threading
from pathlib import Path
from typing import Callable

import numpy as np

from ..config import data_dir, has_cuda
from .catalog import BACKENDS, GGML_DEVICES, ModelSpec, placed

PYTHON = "3.12"
MIN_PACKAGE_AGE = "14 days"  # same rule as Dependabot's cooldown: never install brand-new releases

Log = Callable[[str], None]


class EngineError(RuntimeError):
    pass


class WorkerCrashed(EngineError):
    def __init__(self, message: str, spec: ModelSpec, opts: dict):
        super().__init__(message)
        self.spec, self.opts = spec, opts


def _root() -> Path:
    # The self-test keeps its data separate but reuses installed engines.
    path = Path(os.environ["SPEECHIFY_ENGINES"]) if os.environ.get("SPEECHIFY_ENGINES") else data_dir() / "engines"
    path.mkdir(parents=True, exist_ok=True)
    return path


def env_dir(backend: str) -> Path:
    return _root() / backend


def env_python(backend: str) -> Path:
    d = env_dir(backend)
    return d / "Scripts" / "python.exe" if sys.platform == "win32" else d / "bin" / "python"


def requirements(backend: str) -> tuple[str, ...]:
    return BACKENDS[backend].requirements_for(has_cuda())


def _requirements_hash(backend: str) -> str:
    # Covers the CUDA/CPU choice too, so adding a GPU later reinstalls the
    # environment with the GPU packages.
    return hashlib.sha256("\n".join(requirements(backend)).encode()).hexdigest()[:16]


def _marker(backend: str) -> Path:
    return env_dir(backend) / ".speechify-installed"


def backend_installed(backend: str) -> bool:
    m = _marker(backend)
    return env_python(backend).exists() and m.exists() and m.read_text().strip() == _requirements_hash(backend)


def _downloaded() -> set[str]:
    path = _root() / "downloaded.json"
    return set(json.loads(path.read_text())) if path.exists() else set()


def _set_downloaded(model_id: str, value: bool) -> None:
    ids = _downloaded()
    if value:
        ids.add(model_id)
    else:
        ids.discard(model_id)
    (_root() / "downloaded.json").write_text(json.dumps(sorted(ids)))


def _download_key(spec: ModelSpec) -> str:
    # A model's ONNX/PyTorch weights and its GGUF are separate downloads.
    return f"{spec.id}@ggml" if spec.backend == "ggml" else spec.id


def is_ready(spec: ModelSpec, device: str | None = None) -> bool:
    """Installed and downloaded: selecting it loads straight from disk.

    `device` (a resolved config device) picks the build that runs there; see catalog.placed.
    """
    spec = placed(spec, device)
    local = spec.id.startswith("ft:")
    return backend_installed(spec.backend) and (local or _download_key(spec) in _downloaded())


def status(spec: ModelSpec, device: str | None = None) -> str:
    spec = placed(spec, device)
    if is_ready(spec):
        return "ready"
    if backend_installed(spec.backend):
        return "not downloaded"
    return "not installed"


def find_uv() -> str:
    exe = "uv.exe" if sys.platform == "win32" else "uv"
    candidates = [os.environ.get("UV")]
    if getattr(sys, "frozen", False):  # the installed app ships its own uv
        candidates.append(str(Path(sys._MEIPASS) / exe))
    candidates.append(shutil.which("uv"))
    home = Path.home()
    candidates += [str(home / ".local" / "bin" / exe), str(home / ".cargo" / "bin" / exe)]
    for c in candidates:
        if c and Path(c).exists():
            return c
    hint = ("winget install astral-sh.uv" if sys.platform == "win32"
            else "curl -LsSf https://astral.sh/uv/install.sh | sh")
    raise EngineError(f"uv not found. Install it with: {hint}")


def _shim() -> Path:
    """Copy the worker code somewhere importable without the app's own packages.

    Putting the app's site-packages on the worker's path would mix two
    environments' numpy/torch versions, so the worker gets just these files.
    """
    src = Path(__file__).resolve().parent
    dst = _root() / "_shim" / "speechify" / "engines"
    dst.mkdir(parents=True, exist_ok=True)
    (dst.parent / "__init__.py").write_text("")
    for f in ("__init__.py", "backends.py", "worker.py"):
        data = (src / f).read_bytes()
        target = dst / f
        if not target.exists() or target.read_bytes() != data:
            target.write_bytes(data)
    return dst.parent.parent


def _run(cmd: list[str], log: Log, env: dict | None = None) -> None:
    proc = subprocess.Popen(
        cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        encoding="utf-8", errors="replace", env=env, creationflags=_NO_WINDOW,
    )
    for line in proc.stdout:
        line = line.rstrip()
        if line and not line.startswith('{"event"'):  # the worker's own protocol message
            log(line)
    if proc.wait() != 0:
        raise EngineError(f"command failed ({proc.returncode}): {' '.join(cmd[:4])} ...")


_NO_WINDOW = subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0


def _worker_env(spec: ModelSpec, opts: dict, download_only: bool, offline: bool) -> dict:
    env = dict(os.environ)
    env["PYTHONPATH"] = str(_shim())
    env["PYTHONUNBUFFERED"] = "1"
    opts = {**opts, "models_dir": str(_root() / "models")}
    if spec.gguf is not None and spec.backend == "ggml":
        opts["gguf"] = dataclasses.asdict(spec.gguf)
    elif opts.get("device") in GGML_DEVICES:
        # A model without a transcribe.cpp build, on an AMD, Intel or Apple
        # GPU: its own backend only knows CUDA and the CPU.
        opts.update(device="cpu", compute_type="int8")
    env["SPEECHIFY_WORKER"] = json.dumps({
        "backend": spec.backend, "repo": spec.repo, "download_only": download_only, "opts": opts,
    })
    if offline:
        # Skips Hugging Face's "is there a newer revision" requests: faster startup, works offline.
        env["HF_HUB_OFFLINE"] = "1"
    if sys.platform.startswith("linux"):
        # pip's CUDA and cuDNN libraries (site-packages/nvidia/*/lib) aren't on
        # the loader path, and CTranslate2 won't find them by itself. On
        # Windows, backends.add_nvidia_dll_dirs does the same job.
        libs = [str(d) for d in sorted(env_dir(spec.backend).glob("lib/python3*/site-packages/nvidia/*/lib"))]
        if libs:
            env["LD_LIBRARY_PATH"] = os.pathsep.join(libs + [env.get("LD_LIBRARY_PATH", "")]).rstrip(os.pathsep)
    return env


def install(spec: ModelSpec, opts: dict, log: Log = print) -> None:
    """Create the backend environment if needed, then download the model."""
    spec = placed(spec, opts.get("device"))
    backend = BACKENDS[spec.backend]
    if not backend_installed(spec.backend):
        uv = find_uv()
        log(f"Setting up the {backend.label} environment (one time)...")
        if _marker(spec.backend).exists():
            # Installed with other requirements (say, the CPU build before a
            # GPU was added). Installing on top would leave both onnxruntime
            # and onnxruntime-gpu in one environment, so start clean.
            shutil.rmtree(env_dir(spec.backend), ignore_errors=True)
        _run([uv, "venv", "--python", PYTHON, "--allow-existing", str(env_dir(spec.backend))], log)
        # A requirements file, so pinned packages can carry their hashes
        # (uv checks every hash it's given, and fails on a mismatch).
        reqs = env_dir(spec.backend) / "speechify-requirements.txt"
        reqs.write_text("\n".join(requirements(spec.backend)) + "\n", encoding="utf-8")
        cmd = [uv, "pip", "install", "--python", str(env_python(spec.backend)),
               "--exclude-newer", MIN_PACKAGE_AGE, "-r", str(reqs)]
        if backend.needs_torch:
            cmd += ["--torch-backend", "auto"]  # picks the CUDA build that matches your driver
        _run(cmd, log)
        _marker(spec.backend).write_text(_requirements_hash(spec.backend))
    if not is_ready(spec):
        log(f"Downloading {spec.name} (about {spec.download_gb:g} GB)...")
        _run([str(env_python(spec.backend)), "-m", "speechify.engines.worker"], log,
             env=_worker_env(spec, opts, download_only=True, offline=False))
        _set_downloaded(_download_key(spec), True)
    log(f"{spec.name} is ready.")


def uninstall_backend(backend: str) -> None:
    shutil.rmtree(env_dir(backend), ignore_errors=True)


def falls_back_to_cpu(spec: ModelSpec, opts: dict) -> bool:
    """Whether a worker that dies on the GPU is restarted on the CPU.

    transcribe.cpp models only: their GGUF runs on the CPU as it is. The
    others need different packages for the CPU, and CUDA is reliable enough.
    """
    return spec.backend == "ggml" and opts.get("device") != "cpu"


class EngineHost:
    """Owns one running worker. Thread-safe.

    The app has one for the dictation model, and autolearn.py may start a
    second for the teacher.
    """

    def __init__(self):
        self._proc: subprocess.Popen | None = None
        self._lock = threading.Lock()  # one request at a time
        self._load_lock = threading.Lock()
        self._loading: subprocess.Popen | None = None
        self._generation = 0  # bumped by kill(), so a cancelled load can't finish
        self.ready = threading.Event()
        self.spec: ModelSpec | None = None
        self.opts: dict = {}
        self.info: dict = {}  # the worker's "ready" message; info["device"] is what it runs on

    def start(self, spec: ModelSpec, opts: dict) -> dict:
        """Stop the current model and load `spec`. Blocks until it's ready.

        `spec` is placed on opts["device"] (catalog.placed), so self.spec may
        name the transcribe.cpp build of the model that was asked for.
        """
        self.stop()
        spec = placed(spec, opts.get("device"))
        if not is_ready(spec):
            raise EngineError(f"{spec.name} isn't installed yet")
        generation = self._generation
        attempts = [(opts, not spec.id.startswith("ft:")),
                    (opts, False)]  # maybe the cache is incomplete; let it reach the network once
        if falls_back_to_cpu(spec, opts):
            # The worker falls back to the CPU itself when the GPU reports an
            # error. This covers a driver that takes the whole process down.
            attempts.append(({**opts, "device": "cpu"}, False))
        for i, (attempt, offline) in enumerate(attempts):
            try:
                proc, msg = self._spawn(spec, attempt, offline=offline, generation=generation)
                break
            except EngineError as exc:
                if self._generation != generation or i == len(attempts) - 1:
                    raise
                if attempts[i + 1][0] is not opts:
                    print(f"{spec.name} didn't load on {opts.get('device')} ({exc}); "
                          "trying the CPU instead.", file=sys.stderr)
        with self._lock:
            if self._generation != generation:  # killed after it loaded
                proc.kill()
                raise EngineError(f"loading {spec.name} was cancelled")
            self._proc, self.spec, self.opts, self.info = proc, spec, attempt, msg
        self.ready.set()
        return msg

    def _spawn(self, spec: ModelSpec, opts: dict, offline: bool, generation: int):
        with self._load_lock:
            if self._generation != generation:
                raise EngineError(f"loading {spec.name} was cancelled")
            proc = subprocess.Popen(
                [str(env_python(spec.backend)), "-m", "speechify.engines.worker"],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=None,
                env=_worker_env(spec, opts, download_only=False, offline=offline),
                creationflags=_NO_WINDOW,
            )
            self._loading = proc
        try:
            line = proc.stdout.readline()
        finally:
            with self._load_lock:
                if self._loading is proc:
                    self._loading = None
        if self._generation != generation:
            proc.kill()
            raise EngineError(f"loading {spec.name} was cancelled")
        msg = json.loads(line) if line else {"event": "error", "error": "worker exited while loading"}
        if msg.get("event") != "ready":
            proc.kill()
            raise EngineError(f"{spec.name} failed to load: {msg.get('error')} (details in the console)")
        return proc, msg

    def transcribe(self, audio: np.ndarray, prompt: str | None, timeout: float = 900) -> tuple[str, int]:
        msg = self._request_or_fall_back(audio, prompt, False, timeout)
        return msg["text"], msg["ms"]

    def transcribe_scored(self, audio: np.ndarray, prompt: str | None,
                          timeout: float = 900) -> tuple[str, float | None]:
        """Text and mean token log-probability (None if the backend has none)."""
        msg = self._request_or_fall_back(audio, prompt, True, timeout)
        return msg["text"], msg.get("logprob")

    def _request_or_fall_back(self, audio, prompt, score: bool, timeout: float) -> dict:
        try:
            return self._request(audio, prompt, score, timeout)
        except WorkerCrashed as exc:
            spec, opts = exc.spec, exc.opts
            if not falls_back_to_cpu(spec, opts):
                raise
        # A GPU driver crash took the worker down mid-run. Rather than lose
        # the dictation, reload on the CPU and run it again.
        print(f"{spec.name} crashed on {self.info.get('device')}; reloading it on the CPU "
              "and transcribing again.", file=sys.stderr)
        self.start(spec, {**opts, "device": "cpu"})
        return self._request(audio, prompt, score, timeout)

    def _request(self, audio: np.ndarray, prompt: str | None, score: bool, timeout: float) -> dict:
        if not self.ready.wait(timeout):
            raise EngineError("no model is loaded")
        with self._lock:
            proc = self._proc
            if proc is None:
                raise EngineError("no model is loaded")
            pcm = np.ascontiguousarray(audio, dtype=np.float32)
            request = {"samples": int(pcm.size), "prompt": prompt}
            if score:
                request["score"] = True
            header = json.dumps(request).encode() + b"\n"
            try:
                proc.stdin.write(header)
                proc.stdin.write(pcm.tobytes())
                proc.stdin.flush()
                line = proc.stdout.readline()
            except (OSError, ValueError):
                line = b""
            if not line:
                self.ready.clear()
                if self._proc is not proc:  # kill() stopped it on purpose
                    raise EngineError("the model was stopped")
                self._proc = None
                raise WorkerCrashed("the model process crashed (details in the console)", self.spec, self.opts)
            msg = json.loads(line)
            if msg.get("device"):
                self.info["device"] = msg["device"]
        if "error" in msg:
            raise EngineError(msg["error"])
        return msg

    def kill(self) -> None:
        """Stop the model at once, even mid-load or mid-transcription.

        Doesn't wait for anything: it's called the moment you press the
        dictation key. A transcription in progress fails with EngineError.
        """
        self.ready.clear()
        with self._load_lock:
            self._generation += 1
            procs = [self._loading, self._proc]
            self._loading = None
        # Not under self._lock: a transcription may be holding it.
        self._proc, self.spec = None, None
        for proc in procs:
            if proc is not None:
                try:
                    proc.kill()
                except OSError:
                    pass

    def stop(self) -> None:
        self.ready.clear()
        with self._lock:
            proc, self._proc, self.spec = self._proc, None, None
        if proc is None:
            return
        try:
            proc.stdin.close()
            proc.wait(timeout=5)
        except Exception:
            proc.kill()
