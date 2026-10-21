"""App side of the engines: per-backend environments and the worker process.

Layout under the data directory:

    engines/
      _shim/speechify/engines/*.py   worker code, importable from any env
      whisper/ onnx/ transformers/ qwen/   one virtualenv per backend
      downloaded.json                 models whose weights are on disk
"""

from __future__ import annotations

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

from ..config import data_dir
from .catalog import BACKENDS, ModelSpec

PYTHON = "3.12"
MIN_PACKAGE_AGE = "14 days"  # same rule as Dependabot's cooldown: never install brand-new releases

Log = Callable[[str], None]


class EngineError(RuntimeError):
    pass


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


def _requirements_hash(backend: str) -> str:
    return hashlib.sha256("\n".join(BACKENDS[backend].requirements).encode()).hexdigest()[:16]


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


def is_ready(spec: ModelSpec) -> bool:
    """Installed and downloaded: selecting it loads straight from disk."""
    local = spec.id.startswith("ft:")
    return backend_installed(spec.backend) and (local or spec.id in _downloaded())


def status(spec: ModelSpec) -> str:
    if is_ready(spec):
        return "ready"
    if backend_installed(spec.backend):
        return "not downloaded"
    return "not installed"


def find_uv() -> str:
    candidates = [os.environ.get("UV"), shutil.which("uv")]
    home = Path.home()
    exe = "uv.exe" if sys.platform == "win32" else "uv"
    candidates += [str(home / ".local" / "bin" / exe), str(home / ".cargo" / "bin" / exe)]
    for c in candidates:
        if c and Path(c).exists():
            return c
    raise EngineError("uv not found. Install it with: winget install astral-sh.uv")


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
    env["SPEECHIFY_WORKER"] = json.dumps({
        "backend": spec.backend, "repo": spec.repo, "opts": opts, "download_only": download_only,
    })
    if offline:
        # Skips Hugging Face's "is there a newer revision" requests: faster startup, works offline.
        env["HF_HUB_OFFLINE"] = "1"
    return env


def install(spec: ModelSpec, opts: dict, log: Log = print) -> None:
    """Create the backend environment if needed, then download the model."""
    backend = BACKENDS[spec.backend]
    if not backend_installed(spec.backend):
        uv = find_uv()
        log(f"Setting up the {backend.label} environment (one time)...")
        _run([uv, "venv", "--python", PYTHON, "--allow-existing", str(env_dir(spec.backend))], log)
        cmd = [uv, "pip", "install", "--python", str(env_python(spec.backend)),
               "--exclude-newer", MIN_PACKAGE_AGE, *backend.requirements]
        if backend.needs_torch:
            cmd += ["--torch-backend", "auto"]  # picks the CUDA build that matches your driver
        _run(cmd, log)
        _marker(spec.backend).write_text(_requirements_hash(spec.backend))
    if not is_ready(spec):
        log(f"Downloading {spec.name} (about {spec.download_gb:g} GB)...")
        _run([str(env_python(spec.backend)), "-m", "speechify.engines.worker"], log,
             env=_worker_env(spec, opts, download_only=True, offline=False))
        _set_downloaded(spec.id, True)
    log(f"{spec.name} is ready.")


def uninstall_backend(backend: str) -> None:
    shutil.rmtree(env_dir(backend), ignore_errors=True)


class EngineHost:
    """Owns the single running worker. Thread-safe."""

    def __init__(self):
        self._proc: subprocess.Popen | None = None
        self._lock = threading.Lock()
        self.ready = threading.Event()
        self.spec: ModelSpec | None = None
        self.info: dict = {}

    def start(self, spec: ModelSpec, opts: dict) -> dict:
        """Stop the current model and load `spec`. Blocks until it's ready."""
        self.stop()
        if not is_ready(spec):
            raise EngineError(f"{spec.name} isn't installed yet")
        try:
            proc, msg = self._spawn(spec, opts, offline=not spec.id.startswith("ft:"))
        except EngineError:
            # Maybe the cache is incomplete; let it reach the network once.
            proc, msg = self._spawn(spec, opts, offline=False)
        with self._lock:
            self._proc, self.spec, self.info = proc, spec, msg
        self.ready.set()
        return msg

    def _spawn(self, spec: ModelSpec, opts: dict, offline: bool):
        proc = subprocess.Popen(
            [str(env_python(spec.backend)), "-m", "speechify.engines.worker"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=None,
            env=_worker_env(spec, opts, download_only=False, offline=offline),
            creationflags=_NO_WINDOW,
        )
        line = proc.stdout.readline()
        msg = json.loads(line) if line else {"event": "error", "error": "worker exited while loading"}
        if msg.get("event") != "ready":
            proc.kill()
            raise EngineError(f"{spec.name} failed to load: {msg.get('error')} (details in the console)")
        return proc, msg

    def transcribe(self, audio: np.ndarray, prompt: str | None, timeout: float = 900) -> tuple[str, int]:
        if not self.ready.wait(timeout):
            raise EngineError("no model is loaded")
        with self._lock:
            proc = self._proc
            if proc is None:
                raise EngineError("no model is loaded")
            pcm = np.ascontiguousarray(audio, dtype=np.float32)
            header = json.dumps({"samples": int(pcm.size), "prompt": prompt}).encode() + b"\n"
            try:
                proc.stdin.write(header)
                proc.stdin.write(pcm.tobytes())
                proc.stdin.flush()
                line = proc.stdout.readline()
            except OSError:
                line = b""
            if not line:
                self._proc = None
                self.ready.clear()
                raise EngineError("the model process crashed (details in the console)")
        msg = json.loads(line)
        if "error" in msg:
            raise EngineError(msg["error"])
        return msg["text"], msg["ms"]

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
