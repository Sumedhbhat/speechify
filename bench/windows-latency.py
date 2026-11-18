"""Run the cleanup benchmark on a Windows Actions runner."""
import contextlib
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import urllib.error
import urllib.request
import zipfile

ROOT = Path(os.environ["RUNNER_TEMP"]) / "speechify-latency"
RESULTS = Path("benchmark-results")
PROBE = Path("target/release/examples/windows-latency.exe").resolve()
OLLAMA = "http://127.0.0.1:11434"
DIRECT = "http://127.0.0.1:18080"
OLLAMA_VERSION = "0.40.16"


def install(repository, tag, asset, digest, name):
    archive = ROOT / asset
    url = f"https://github.com/{repository}/releases/download/{tag}/{asset}"
    subprocess.run(["curl.exe", "-fL", "-sS", "--retry", "3", "-o", archive, url], check=True)
    with archive.open("rb") as stream:
        if hashlib.file_digest(stream, "sha256").hexdigest() != digest:
            raise ValueError(f"Unexpected checksum for {asset}")
    with zipfile.ZipFile(archive) as package:
        package.extractall(ROOT / name)
    archive.unlink()
    return ROOT / name


def request(url, body=None, timeout=90):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as response:
        return json.load(response)


@contextlib.contextmanager
def serving(command, endpoint, name, env):
    with (RESULTS / f"{name}.log").open("wb") as log:
        proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, env=env)
        try:
            for _ in range(90):
                if proc.poll() is not None:
                    raise RuntimeError(f"{name} exited during startup")
                try:
                    request(endpoint, timeout=2)
                    break
                except (urllib.error.URLError, TimeoutError):
                    time.sleep(1)
            else:
                raise RuntimeError(f"{name} did not become ready")
            yield
        finally:
            subprocess.run(["taskkill", "/pid", str(proc.pid), "/t", "/f"], capture_output=True)
            proc.wait(timeout=15)


def probe(backend, url, model, timeout=6):
    name = f"{backend}-{model.replace(':', '-')}"
    with (RESULTS / f"{name}.jsonl").open("wb") as output:
        subprocess.run([PROBE, backend, url, model, str(timeout)], stdout=output, check=True)


def main():
    RESULTS.mkdir(exist_ok=True)
    ROOT.mkdir(exist_ok=True)
    probe("rules", OLLAMA, "rules")
    folder = install(
        "ollama/ollama", f"v{OLLAMA_VERSION}", "ollama-windows-amd64.zip",
        "50fcf5ad54277f2fa5d173ebfc93bcb017adbc0c9ef2314daabc96ca9e6aa64e", "ollama",
    )
    ollama = next(folder.rglob("ollama.exe"))
    weights = ROOT / "weights"
    env = dict(os.environ, OLLAMA_HOST="127.0.0.1:11434", OLLAMA_MODELS=str(weights),
               OLLAMA_NUM_PARALLEL="1", OLLAMA_MAX_LOADED_MODELS="1",
               OLLAMA_CONTEXT_LENGTH="4096", OLLAMA_VULKAN="0", CUDA_VISIBLE_DEVICES="-1")
    with serving([ollama, "serve"], f"{OLLAMA}/api/tags", "ollama", env):
        for model in ("gemma3:4b", "llama3.2:1b", "llama3.2:3b"):
            subprocess.run([ollama, "pull", model], env=env, check=True)
            request(f"{OLLAMA}/api/generate", {"model": model, "keep_alive": "30m"})
            state = request(f"{OLLAMA}/api/ps")
            (RESULTS / f"loaded-{model.replace(':', '-')}.json").write_text(json.dumps(state))
            probe("ollama", OLLAMA, model)
            request(f"{OLLAMA}/api/generate", {"model": model, "keep_alive": 0})
        (RESULTS / "tags.json").write_text(json.dumps(request(f"{OLLAMA}/api/tags")))

    manifest = json.loads((weights / "manifests/registry.ollama.ai/library/llama3.2/3b").read_text())
    digest = next(layer["digest"] for layer in manifest["layers"] if layer["mediaType"].endswith("image.model"))
    gguf = weights / "blobs" / digest.replace(":", "-")
    servers = list(folder.rglob("llama-server.exe"))
    if not servers:
        folder = install(
            "ggml-org/llama.cpp", "b12521", "llama-b12521-bin-win-cpu-x64.zip",
            "bd4e093bce292452248c318d4b176fbfebe1ca43a7f3779ebb3846fe7753653e", "llama",
        )
        servers = list(folder.rglob("llama-server.exe"))
    server = servers[0]
    with serving(
        [server, "-m", gguf, "--alias", "llama3.2:3b", "-c", "4096", "-ngl", "0",
         "--parallel", "1", "-t", "4", "--host", "127.0.0.1", "--port", "18080"],
        f"{DIRECT}/health", "llama-server", env,
    ):
        request(f"{DIRECT}/v1/chat/completions", {
            "model": "llama3.2:3b", "messages": [{"role": "user", "content": "Reply OK."}], "max_tokens": 4,
        })
        probe("llama-server", f"{DIRECT}/v1", "llama3.2:3b")


if __name__ == "__main__":
    try:
        main()
    finally:
        shutil.rmtree(ROOT, ignore_errors=True)
