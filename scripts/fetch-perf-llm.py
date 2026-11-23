import hashlib
import shutil
import stat
import sys
import tarfile
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
MODEL = (
    "gemma-3-4b-it-Q4_K_M.gguf",
    "https://huggingface.co/ggml-org/gemma-3-4b-it-GGUF/resolve/"
    "d0976223747697cb51e056d85c532013931fe52e/gemma-3-4b-it-Q4_K_M.gguf",
    "882e8d2db44dc554fb0ea5077cb7e4bc49e7342a1f0da57901c0802ea21a0863",
)


def digest(path):
    sha = hashlib.sha256()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            sha.update(block)
    return sha.hexdigest()


def fetch(url, path, expected):
    if path.exists() and digest(path) == expected:
        return
    part = path.with_name(path.name + ".part")
    with urllib.request.urlopen(url, timeout=120) as response, open(part, "wb") as out:
        shutil.copyfileobj(response, out, 1 << 20)
    if digest(part) != expected:
        part.unlink()
        sys.exit(f"The checksum of {url} does not match")
    part.replace(path)


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
    server = next(server_dir.rglob(name))
    server.chmod(server.stat().st_mode | stat.S_IXUSR)
    model_name, model_url, model_sha = MODEL
    model = dest / model_name
    fetch(model_url, model, model_sha)
    (dest / "env.sh").write_text(
        f'export PERF_LLAMA_SERVER="{server.as_posix()}"\n'
        f'export PERF_LLM_FILE="{model.as_posix()}"\n'
        f'export PERF_LLM_MODEL="{model_name.removesuffix(".gguf")}"\n',
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
