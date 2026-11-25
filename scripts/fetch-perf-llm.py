import hashlib
import json
import shutil
import stat
import sys
import tarfile
import time
import urllib.request
import zipfile
from pathlib import Path

RELEASE = "b12211"
SERVERS = {
    "linux": (
        f"llama-{RELEASE}-bin-ubuntu-x64.tar.gz",
        "962924b5431bc1237a9e5d26ba072d37b6fe05f79b495206e7ff6134887624c5",
    ),
    "darwin": (
        f"llama-{RELEASE}-bin-macos-arm64.tar.gz",
        "4da4edb437358d31d55938ad849d262ca31b0e0ffb35e9caedb727a0cd5a8f0f",
    ),
    "win32": (
        f"llama-{RELEASE}-bin-win-cpu-x64.zip",
        "23729000408c6968faf5e322afc99319d3b8c576ba872ef7663d9b89810090f3",
    ),
}
REGISTRY = "https://registry.ollama.ai/v2"
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
    if path.exists() and digest(path) == expected:
        return
    part = path.with_name(path.name + ".part")

    def download():
        with urllib.request.urlopen(urllib.request.Request(url), timeout=120) as response, open(part, "wb") as out:
            shutil.copyfileobj(response, out, 1 << 20)

    retried(download, url)
    if digest(part) != expected:
        part.unlink()
        sys.exit(f"The checksum of {url} does not match")
    part.replace(path)


def manifest(name, version):
    request = urllib.request.Request(
        f"{REGISTRY}/{name}/manifests/{version or 'latest'}",
        headers={"Accept": "application/vnd.docker.distribution.manifest.v2+json"},
    )

    def read():
        with urllib.request.urlopen(request, timeout=60) as response:
            return json.load(response)

    return retried(read, f"the manifest of {name}:{version}")


def main():
    dest = Path(sys.argv[1]).resolve()
    dest.mkdir(parents=True, exist_ok=True)
    platform = "linux" if sys.platform.startswith("linux") else sys.platform
    archive_name, archive_sha = SERVERS[platform]
    archive = dest / archive_name
    fetch(
        f"https://github.com/ggml-org/llama.cpp/releases/download/{RELEASE}/{archive_name}",
        archive,
        archive_sha,
    )
    server_dir = dest / "server"
    shutil.rmtree(server_dir, ignore_errors=True)
    if archive_name.endswith(".zip"):
        with zipfile.ZipFile(archive) as bundle:
            bundle.extractall(server_dir)
    else:
        with tarfile.open(archive) as bundle:
            bundle.extractall(server_dir, filter="data")
    name = "llama-server.exe" if platform == "win32" else "llama-server"
    server = next(server_dir.rglob(name), None)
    if server is None:
        sys.exit(f"{archive_name} has no {name}")
    server.chmod(server.stat().st_mode | stat.S_IXUSR)
    tag = json.loads(DEFAULTS.read_text(encoding="utf-8"))["cleanup_model"]
    name, _, version = tag.partition(":")
    if "/" not in name:
        name = f"library/{name}"
    layer = next(
        (item for item in manifest(name, version)["layers"] if item["mediaType"] == "application/vnd.ollama.image.model"),
        None,
    )
    if layer is None:
        sys.exit(f"{tag} has no model layer in the Ollama registry")
    model_sha = layer["digest"].removeprefix("sha256:")
    model = dest / f"{model_sha}.gguf"
    fetch(f"{REGISTRY}/{name}/blobs/{layer['digest']}", model, model_sha)
    (dest / "env.sh").write_text(
        f'export PERF_LLAMA_SERVER="{server.as_posix()}"\n'
        f'export PERF_LLM_FILE="{model.as_posix()}"\n'
        f'export PERF_LLM_MODEL="{tag}"\n'
        f'export PERF_LLM_DIGEST="{layer["digest"]}"\n',
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
