#!/usr/bin/env python3
import json
import urllib.request
import pathlib

root = pathlib.Path(__file__).resolve().parents[1]
models = {
    "distil-whisper-large-v3": "Systran/faster-distil-whisper-large-v3",
    "kyutai-stt-2.6b": "kyutai/stt-2.6b-en-trfs",
    "kyutai-stt-1b": "kyutai/stt-1b-en_fr-trfs",
}
result = {}
for name, repo in models.items():
    commits = json.load(
        urllib.request.urlopen(f"https://huggingface.co/api/models/{repo}/commits/main")
    )
    commit = next(c for c in commits if c["date"] < "2026-10-11")["id"]
    files = json.load(
        urllib.request.urlopen(
            f"https://huggingface.co/api/models/{repo}/tree/{commit}?recursive=true&expand=false"
        )
    )
    files = [
        f
        for f in files
        if f["type"] == "file"
        and pathlib.Path(f["path"]).suffix
        in [".json", ".safetensors", ".bin", ".model", ".txt", ".tiktoken"]
        and not f["path"].startswith("onnx/")
    ]
    result[name] = {
        "repo": repo,
        "revision": commit,
        "files": [
            {
                "path": f["path"],
                "size": f["size"],
                "sha256": f.get("lfs", {}).get("oid"),
                "git_blob": f["oid"],
            }
            for f in files
        ],
    }
(root / "src-tauri/resources/exception-models.json").write_text(json.dumps(result, indent=2) + "\n")
