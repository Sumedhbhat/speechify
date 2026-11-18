import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
from urllib.error import URLError
from urllib.request import Request, urlopen

MODEL = "Llama-3.2-3B-Instruct-Q4_K_M.gguf"
RELEASE = "https://github.com/ggml-org/llama.cpp/releases/download/b12521"
WEIGHTS = "https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/5ab33fa94d1d04e903623ae72c95d1696f09f9e8"
BASE = "http://127.0.0.1:18080/v1"

def download(url, path, expected):
    subprocess.run(["curl", "-fsSL", "--retry", "3", "-o", path, url], check=True)
    with path.open("rb") as stream:
        assert hashlib.file_digest(stream, "sha256").hexdigest() == expected

def request(path, data=None, timeout=5):
    req = Request(BASE + path, data=None if data is None else json.dumps(data).encode(),
                  headers={"Content-Type": "application/json"})
    with urlopen(req, timeout=timeout) as r:
        return json.load(r)

with tempfile.TemporaryDirectory() as tmp:
    d = Path(tmp)
    archive = d / os.environ["PACKAGE"]
    download(f"{RELEASE}/llama-b12521-bin-{archive.name}", archive, os.environ["EXPECTED_SHA"])
    if archive.suffix == ".zip":
        subprocess.run(["python3", "-m", "zipfile", "-e", archive, d], check=True)
    else:
        subprocess.run(["tar", "-xzf", archive, "-C", d], check=True)
    binary = next(d.rglob("llama-server.exe" if os.name == "nt" else "llama-server"))
    model = binary.parent / MODEL
    download(f"{WEIGHTS}/{MODEL}?download=true", model,
             "6c1a2b41161032677be168d354123594c0e6e67d2b9227c84f296ad037c728ff")
    with subprocess.Popen([binary, "-m", MODEL, "--alias", "llama3.2:3b", "-c", "4096",
                           "--host", "127.0.0.1", "--port", "18080", "--log-disable"],
                          cwd=binary.parent) as server:
        try:
            for _ in range(90):
                assert server.poll() is None, f"server exited: {server.returncode}"
                try:
                    assert "llama3.2:3b" in {m["id"] for m in request("/models")["data"]}
                    break
                except (URLError, TimeoutError):
                    time.sleep(1)
            else:
                raise RuntimeError("Server failed readiness check")
            reply = request("/chat/completions", {
                "model": "llama3.2:3b", "temperature": 0, "stream": False, "max_tokens": 32,
                "messages": [{"role": "system", "content": "Clean up filler words. Output only the edited text."},
                             {"role": "user", "content": "Um, please send the invoice tomorrow."}],
            }, timeout=60)
            answer = reply["choices"][0]["message"]["content"]
            assert "invoice tomorrow" in answer.lower(), answer
            print("Model discovered and cleanup completed:", answer)
        finally:
            server.terminate()
            server.wait(timeout=15)
