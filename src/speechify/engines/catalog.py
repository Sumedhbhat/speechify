"""Every speech model Speechify can run, and what each one needs.

Models are grouped by backend. Each backend gets its own Python environment
because their dependencies conflict (qwen-asr pins transformers 4.57, Cohere
needs transformers 5.4+), and so the app itself stays small and starts fast.
Only the selected model is ever loaded.

Ratings are 1-5 for dictation on an RTX laptop, based on the Open ASR
Leaderboard and published benchmarks (see README). Speed means how quickly
text appears after you let go of the key.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Backend:
    id: str
    label: str
    requirements: tuple[str, ...]
    # Added on machines with an NVIDIA GPU (see config.has_cuda), and the
    # CPU-only builds everywhere else, macOS included.
    cuda: tuple[str, ...] = ()
    cpu: tuple[str, ...] = ()
    # PyTorch wheels come from the CUDA index matching the installed driver.
    needs_torch: bool = False

    def requirements_for(self, cuda: bool) -> tuple[str, ...]:
        return self.requirements + (self.cuda if cuda else self.cpu)


@dataclass(frozen=True)
class ModelSpec:
    id: str
    name: str
    backend: str
    repo: str  # what the backend loads: a HF repo, an onnx-asr name, or a local path
    languages: str
    accuracy: float
    speed: float
    vram_gb: float
    download_gb: float
    # Whether the model accepts a vocabulary hint (your learned glossary).
    prompt: bool = False
    finetune: bool = False
    gated: bool = False
    notes: str = ""


BACKENDS: dict[str, Backend] = {
    b.id: b
    for b in [
        Backend(
            "whisper",
            "faster-whisper (CTranslate2)",
            ("numpy", "faster-whisper>=1.1"),
            cuda=("nvidia-cublas-cu12", "nvidia-cudnn-cu12>=9,<10"),
        ),
        Backend(
            "onnx",
            "onnx-asr (ONNX Runtime)",
            ("numpy", "onnx-asr[hub]>=0.12"),
            cuda=("onnxruntime-gpu[cuda,cudnn]",),
            cpu=("onnxruntime",),
        ),
        Backend(
            "transformers",
            "Hugging Face transformers",
            (
                "numpy",
                "torch",
                "torchaudio",  # Granite Speech's feature extractor
                "transformers>=5.4",
                "accelerate",
                "soundfile",
                "librosa",
                "sentencepiece",
                "protobuf",
                # Voxtral's tokenizer. transformers ignores mistral-common below
                # 1.11.5 and also needs Pillow before it will use it.
                "mistral-common[audio]>=1.11.5",
                "pillow",
            ),
            needs_torch=True,
        ),
        Backend("qwen", "qwen-asr", ("numpy", "torch", "qwen-asr"), needs_torch=True),
    ]
}

MODELS: dict[str, ModelSpec] = {
    m.id: m
    for m in [
        # --- Whisper family: vocabulary hints + fine-tuning on your voice ----
        ModelSpec(
            "whisper-large-v3-turbo", "Whisper large-v3-turbo", "whisper", "large-v3-turbo",
            "99 languages", accuracy=3.5, speed=4, vram_gb=2, download_gb=1.6,
            prompt=True, finetune=True,
            notes="Default. Learns your vocabulary and can be fine-tuned on your recordings.",
        ),
        ModelSpec(
            "whisper-large-v3", "Whisper large-v3", "whisper", "large-v3",
            "99 languages", accuracy=3.5, speed=2.5, vram_gb=4, download_gb=3.1,
            prompt=True, finetune=True,
        ),
        ModelSpec(
            "distil-whisper-large-v3", "Distil-Whisper large-v3", "whisper", "distil-large-v3",
            "English", accuracy=3, speed=4.5, vram_gb=2, download_gb=1.5, prompt=True,
        ),
        # --- NVIDIA NeMo models, exported to ONNX: the fastest --------------
        ModelSpec(
            "parakeet-tdt-0.6b-v3", "NVIDIA Parakeet TDT 0.6B v3", "onnx", "nemo-parakeet-tdt-0.6b-v3",
            "25 European languages", accuracy=4, speed=5, vram_gb=2, download_gb=2.5,
            notes="Best speed/accuracy balance, and the fastest model on a CPU.",
        ),
        ModelSpec(
            "parakeet-tdt-0.6b-v2", "NVIDIA Parakeet TDT 0.6B v2", "onnx", "nemo-parakeet-tdt-0.6b-v2",
            "English", accuracy=4, speed=5, vram_gb=2, download_gb=2.5,
            notes="English-only predecessor of v3; slightly better on English.",
        ),
        ModelSpec(
            "canary-1b-v2", "NVIDIA Canary 1B v2", "onnx", "nemo-canary-1b-v2",
            "25 European languages", accuracy=4, speed=4, vram_gb=3, download_gb=4,
        ),
        # --- LLM-based recognizers: most accurate, a bit slower --------------
        ModelSpec(
            "granite-speech-4.1-2b", "IBM Granite Speech 4.1 2B", "transformers",
            "ibm-granite/granite-speech-4.1-2b",
            "en, fr, de, es, pt, ja", accuracy=5, speed=3, vram_gb=6, download_gb=4.5,
            notes="#1 on the Open ASR Leaderboard (5.33% WER).",
        ),
        ModelSpec(
            "cohere-transcribe", "Cohere Transcribe (03-2026)", "transformers",
            "CohereLabs/cohere-transcribe-03-2026",
            "14 languages", accuracy=4.5, speed=3, vram_gb=5, download_gb=4.2, gated=True,
            notes="Gated: accept the terms on its Hugging Face page and run `hf auth login` first.",
        ),
        ModelSpec(
            "qwen3-asr-1.7b", "Qwen3-ASR 1.7B", "qwen", "Qwen/Qwen3-ASR-1.7B",
            "52 languages and dialects", accuracy=4.5, speed=3.5, vram_gb=5, download_gb=4.4,
        ),
        ModelSpec(
            "qwen3-asr-0.6b", "Qwen3-ASR 0.6B", "qwen", "Qwen/Qwen3-ASR-0.6B",
            "52 languages and dialects", accuracy=4, speed=4, vram_gb=2.5, download_gb=1.9,
        ),
        ModelSpec(
            "voxtral-mini-3b", "Mistral Voxtral Mini 3B", "transformers", "mistralai/Voxtral-Mini-3B-2507",
            "8 languages", accuracy=4, speed=2.5, vram_gb=9, download_gb=9.4,
            notes="Needs ~9 GB of VRAM.",
        ),
        ModelSpec(
            "kyutai-stt-2.6b", "Kyutai STT 2.6B", "transformers", "kyutai/stt-2.6b-en-trfs",
            "English", accuracy=4, speed=3, vram_gb=6, download_gb=5.6,
        ),
        ModelSpec(
            "kyutai-stt-1b", "Kyutai STT 1B", "transformers", "kyutai/stt-1b-en_fr-trfs",
            "English, French", accuracy=3.5, speed=4, vram_gb=3, download_gb=2.2,
        ),
        ModelSpec(
            "moonshine-base", "Moonshine base", "transformers", "moonshine-ai/moonshine-base",
            "English", accuracy=2, speed=5, vram_gb=1, download_gb=0.25,
            notes="Tiny; for weak hardware. Noticeably less accurate.",
        ),
    ]
}

# Models that exist but aren't offered, shown so it's clear why.
UNSUPPORTED = {
    "Canary-Qwen 2.5B": "needs NVIDIA NeMo, which doesn't support Windows or macOS",
    "Voxtral Mini 4B Realtime": "needs vLLM, which doesn't support Windows or macOS",
}

# Names used before models had catalog ids.
ALIASES = {
    "large-v3-turbo": "whisper-large-v3-turbo",
    "turbo": "whisper-large-v3-turbo",
    "large-v3": "whisper-large-v3",
    "distil-large-v3": "distil-whisper-large-v3",
}


def finetuned_models(data_dir: Path) -> list[ModelSpec]:
    """Whisper models produced by scripts/finetune.py."""
    out = []
    for path in sorted((data_dir / "models").glob("whisper-ft-*"), reverse=True):
        if (path / "model.bin").exists():
            out.append(ModelSpec(
                f"ft:{path.name}", f"Your fine-tuned Whisper ({path.name[11:]})", "whisper",
                str(path), "as trained", accuracy=0, speed=4, vram_gb=2, download_gb=0,
                prompt=True, finetune=True, notes="Trained on your recordings.",
            ))
    return out


def all_models(data_dir: Path) -> list[ModelSpec]:
    return finetuned_models(data_dir) + list(MODELS.values())


def resolve(name: str, data_dir: Path) -> ModelSpec:
    """Find a model by catalog id, legacy name, or path to a fine-tuned model."""
    name = ALIASES.get(name, name)
    if name in MODELS:
        return MODELS[name]
    for spec in finetuned_models(data_dir):
        if name in (spec.id, spec.repo) or Path(name).name == Path(spec.repo).name:
            return spec
    path = Path(name)
    if path.is_dir() and (path / "model.bin").exists():
        return ModelSpec(f"ft:{path.name}", f"Whisper ({path.name})", "whisper", str(path),
                         "as trained", 0, 4, 2, 0, prompt=True, finetune=True)
    raise KeyError(f"unknown model {name!r}; run `speechify models` to see the options")


def stars(rating: float) -> str:
    if rating <= 0:
        return "yours"
    return "★" * int(rating) + ("½" if rating % 1 else "")
