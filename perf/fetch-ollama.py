"""Fetch a pinned Ollama for the performance gate, or read the digest of a pulled model.

fetch-ollama.py DIR          download the pinned Ollama release into DIR/ollama, keeping
                             only what a CPU run needs, and write DIR/env.sh
fetch-ollama.py digest DIR   print the weights digest of the cleanup model pulled into DIR/models
"""

import hashlib
import json
import shutil
import stat
import subprocess
import sys
import tarfile
import time
import urllib.request
import zipfile
from pathlib import Path, PurePosixPath

RELEASE = "v0.40.19"
# Besides the CPU libraries, the archives bundle graphics backends in folders of their own:
# CUDA, ROCm and Vulkan in lib/ollama/<backend>/ (over 2 GB on Linux and Windows), and the
# MLX engine for safetensors models on macOS. Ollama looks for those backends only in such
# folders, and the runners have no graphics card, so they are left out. Metal is built into
# the macOS binary itself.
BACKENDS = ("cuda_", "rocm", "vulkan", "mlx_")
ARCHIVES = {
    "linux": (
        "ollama-linux-amd64.tar.zst",
        "8eb99f9dd91c82f4c51a2c891415bfbdf07bd43f627d82514c47dfd613d2070f",
    ),
    "darwin": (
        "ollama-darwin.tgz",
        "c335802128e9ac33fc0599fa9da661c3d44fbe0dc0176bb6a2f4134386205942",
    ),
    "win32": (
        "ollama-windows-amd64.zip",
        "0114b9f103d783b96f14390cf7f11d0fa360040b3ee9941f370fdee503cca308",
    ),
}
DEFAULTS = Path(__file__).resolve().parent.parent / "core" / "defaults.json"
ATTEMPTS = 3


def digest(path):
    sha = hashlib.sha256()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            sha.update(block)
    return sha.hexdigest()


def retried(action, what):
    for attempt in range(1, ATTEMPTS + 1):
        try:
            return action()
        except OSError as error:
            if attempt == ATTEMPTS:
                sys.exit(f"Could not fetch {what} after {ATTEMPTS} attempts: {error}")
            print(f"Fetching {what} failed ({error}); trying again", file=sys.stderr)
            time.sleep(10 * attempt)


def fetch(url, path, expected):
    part = path.with_name(path.name + ".part")

    def download():
        with urllib.request.urlopen(urllib.request.Request(url), timeout=120) as response, open(part, "wb") as out:
            shutil.copyfileobj(response, out, 1 << 20)

    retried(download, url)
    if digest(part) != expected:
        part.unlink()
        sys.exit(f"The checksum of {url} does not match")
    part.replace(path)


def needed(name):
    """Everything except the graphics backends' folders."""
    name = name.replace("\\", "/")
    parts = PurePosixPath(name).parts
    folders = parts if name.endswith("/") else parts[:-1]
    return not any(folder.startswith(BACKENDS) for folder in folders)


def extract(archive, out):
    if archive.name.endswith(".zip"):
        with zipfile.ZipFile(archive) as bundle:
            bundle.extractall(out, [name for name in bundle.namelist() if needed(name)])
        return
    if archive.name.endswith(".zst"):
        # Python reads Zstandard only from 3.14, so stream it through the zstd tool.
        unzstd = subprocess.Popen(["zstd", "-dc", str(archive)], stdout=subprocess.PIPE)
        with tarfile.open(fileobj=unzstd.stdout, mode="r|") as bundle:
            for member in bundle:
                if needed(member.name + "/" if member.isdir() else member.name):
                    bundle.extract(member, out, filter="data")
        if unzstd.wait() != 0:
            sys.exit(f"Could not decompress {archive.name}")
        return
    with tarfile.open(archive) as bundle:
        bundle.extractall(out, [m for m in bundle.getmembers() if needed(m.name + "/" if m.isdir() else m.name)], filter="data")


def install(dest, platform):
    archive_name, archive_sha = ARCHIVES[platform]
    server_dir = dest / "ollama"
    stamp = server_dir / ".release"
    if not (stamp.exists() and stamp.read_text(encoding="utf-8") == archive_sha):
        archive = dest / archive_name
        if not (archive.exists() and digest(archive) == archive_sha):
            fetch(f"https://github.com/ollama/ollama/releases/download/{RELEASE}/{archive_name}", archive, archive_sha)
        shutil.rmtree(server_dir, ignore_errors=True)
        extract(archive, server_dir)
        archive.unlink()
        stamp.write_text(archive_sha, encoding="utf-8")
    name = "ollama.exe" if platform == "win32" else "ollama"
    server = next((path for path in server_dir.rglob(name) if path.is_file()), None)
    if server is None:
        sys.exit(f"{archive_name} has no {name}")
    server.chmod(server.stat().st_mode | stat.S_IXUSR)
    return server


def model_digest(dest, tag):
    """The digest of the weights, which stays the same when a tag is re-published unchanged."""
    name, _, version = tag.partition(":")
    if "/" not in name:
        name = f"library/{name}"
    path = dest / "models" / "manifests" / "registry.ollama.ai" / name / (version or "latest")
    layers = json.loads(path.read_text(encoding="utf-8"))["layers"]
    layer = next((item for item in layers if item["mediaType"] == "application/vnd.ollama.image.model"), None)
    if layer is None:
        sys.exit(f"{tag} has no model layer")
    return layer["digest"]


def main():
    tag = json.loads(DEFAULTS.read_text(encoding="utf-8"))["cleanup_model"]
    if sys.argv[1] == "digest":
        # No newline, so a Windows shell does not keep a carriage return.
        sys.stdout.write(model_digest(Path(sys.argv[2]), tag))
        return
    dest = Path(sys.argv[1]).resolve()
    dest.mkdir(parents=True, exist_ok=True)
    platform = "linux" if sys.platform.startswith("linux") else sys.platform
    server = install(dest, platform)
    (dest / "env.sh").write_text(
        f'export PERF_OLLAMA="{server.as_posix()}"\nexport PERF_LLM_MODEL="{tag}"\n',
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
