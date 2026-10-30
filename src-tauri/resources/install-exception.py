"""Explicit optional installation, supervised by Rust. No hooks or application lifecycle.
Speechify-owned files only; hash-locked environments and pinned model revisions.
A killed installation leaves no completion marker and is safely retryable.
"""

import argparse
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys
import urllib.request


def emit(stage, **fields):
    print(json.dumps(dict(stage=stage, **fields)), flush=True)


def install(root, family, model=None):
    if family == "training" and sys.platform == "darwin":
        raise ValueError(
            "Fine-tuning requires NVIDIA CUDA on Windows or Linux; macOS training is unavailable"
        )
    source = pathlib.Path(__file__).resolve().parent
    env = root / "environments" / family
    marker = env / "candidate-ready"
    uv_env = {
        **os.environ,
        "UV_CACHE_DIR": str(root / "cache/uv"),
        "UV_PYTHON_INSTALL_DIR": str(root / "environments/python"),
    }
    receipt = "cuda-cu126-v1\n" if family == "training" else "complete\n"
    if not marker.exists() or (family == "training" and marker.read_text() != receipt):
        emit("Preparing Python 3.12 environment")
        # Retry repairs an interrupted environment. uv owns only this environment path.
        subprocess.run(
            ["uv", "venv", "--clear", "--python", "3.12", str(env)],
            env=uv_env,
            check=True,
            stdout=subprocess.DEVNULL,
        )
        python = env / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
        emit("Installing hash-locked dependencies")
        backend = ["--torch-backend", "cu126"] if family == "training" else []
        subprocess.run(
            [
                "uv",
                "pip",
                "sync",
                *backend,
                "--python",
                str(python),
                "--require-hashes",
                str(source / "requirements" / f"{family}.lock"),
            ],
            env=uv_env,
            check=True,
            stdout=subprocess.DEVNULL,
        )
        marker.write_text(receipt)
    if not model:
        return
    spec = json.loads((source / "exception-models.json").read_text())[model]
    dest = root / "models" / model
    partial = dest.with_name(dest.name + ".partial")
    if (dest / "candidate-provenance.json").exists():
        return
    if dest.exists():
        raise ValueError("Incomplete model directory exists; preserve it and use a clean directory")
    if partial.exists():
        shutil.rmtree(partial)
    partial.mkdir(parents=True)
    total = sum(f["size"] for f in spec["files"])
    done = 0
    try:
        for f in spec["files"]:
            rel = pathlib.PurePosixPath(f["path"])
            if rel.is_absolute() or ".." in rel.parts:
                raise ValueError("Unsafe model filename")
            dst = partial / rel
            dst.parent.mkdir(parents=True, exist_ok=True)
            digest = (
                hashlib.sha256()
                if f["sha256"]
                else hashlib.sha1(b"blob " + str(f["size"]).encode() + b"\0")
            )
            size = 0
            with (
                urllib.request.urlopen(
                    f"https://huggingface.co/{spec['repo']}/resolve/{spec['revision']}/{f['path']}",
                    timeout=30,
                ) as response,
                dst.open("wb") as out,
            ):
                while data := response.read(1024 * 1024):
                    out.write(data)
                    digest.update(data)
                    size += len(data)
                    if size > f["size"]:
                        raise ValueError("Unexpected model size")
                    emit("Downloading", bytes=done + size, total=total, file=f["path"])
            emit("Verifying model file", bytes=done + size, total=total)
            if size != f["size"] or digest.hexdigest() != (f["sha256"] or f["git_blob"]):
                raise ValueError("Model checksum mismatch: " + f["path"])
            done += size
        (partial / "candidate-provenance.json").write_text(json.dumps(spec, indent=2) + "\n")
        partial.rename(dest)
    except BaseException:
        shutil.rmtree(partial, ignore_errors=True)
        raise


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True, type=pathlib.Path)
    parser.add_argument("--family", required=True, choices=["whisper", "transformers", "training"])
    parser.add_argument(
        "--model", choices=["distil-whisper-large-v3", "kyutai-stt-1b", "kyutai-stt-2.6b"]
    )
    args = parser.parse_args()
    try:
        install(args.root, args.family, args.model)
        emit("Installed")
    except Exception as error:
        emit("Failed", error=str(error))
        raise SystemExit(1)
