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

from dataclasses import dataclass, replace
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
class Gguf:
    """A model's transcribe.cpp build: one GGUF file in a Hugging Face repo,
    pinned to a revision and checked against its SHA-256 after download."""

    repo: str
    file: str
    revision: str
    sha256: str
    size: int  # bytes

    @property
    def size_gb(self) -> float:
        return round(self.size / 1e9, 1)


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
    # The same model for transcribe.cpp, used on GPUs other than NVIDIA's (see placed()).
    gguf: Gguf | None = None
    # For a model placed on the ggml backend, the backend the catalog lists it under.
    origin: str = ""


# transcribe.cpp, the ggml engine Handy uses: Vulkan, Metal, CUDA and CPU
# builds of the same models. Pinned with hashes (14-day rule: 0.3.1 is the
# newest release at least 14 days old, see the dependency-updates policy).
# The bindings and the default native build (CPU + Vulkan on Windows and
# Linux, CPU + Metal on Apple Silicon) come from PyPI; the CUDA build is too
# big for PyPI and comes from the GitHub release.
TRANSCRIBE_CPP_VERSION = "0.3.1"
TRANSCRIBE_CPP = (
    "transcribe-cpp==0.3.1"
    " --hash=sha256:af815ea64a996e51b6f72884606bbdb400fa4bd4e6a667caef16954b86d44f34",
    "transcribe-cpp-native==0.3.1"
    " --hash=sha256:8b5fe9f76fcb88d64981c55837f74bd0ccba013ac7f1487216a59a1ef8dadc82"  # macOS arm64
    " --hash=sha256:a03335e8512b0f8a35ed6b3409beca6530161060dbf5f6d31615cca51edf3abd"  # macOS x86_64
    " --hash=sha256:cffc38c389fb5933270c6b7e282c18dcb5b88798005c021a4f6644372d3f9ffa"  # Linux aarch64
    " --hash=sha256:9647bff7a7486810c2778fb9b86a6ac0302688620bfad8f31b560697423641aa"  # Linux x86_64
    " --hash=sha256:8bb3e962cf9405054129a8ca9ecf9c9df7f4f044b9d6467bf94d5823ff1c9533",  # Windows
)
_RELEASE = "https://github.com/handy-computer/transcribe.cpp/releases/download/v0.3.1/"
TRANSCRIBE_CPP_CUDA = (
    f"transcribe-cpp-native-cu12 @ {_RELEASE}transcribe_cpp_native_cu12-0.3.1-py3-none-win_amd64.whl"
    "#sha256=beda0455e5ad2093debf29a6afaeb8a164d58bff5358db84a14c3cbe95b9247a ; sys_platform == 'win32'",
    f"transcribe-cpp-native-cu12 @ {_RELEASE}transcribe_cpp_native_cu12-0.3.1-py3-none-"
    "manylinux_2_27_x86_64.manylinux_2_28_x86_64.whl"
    "#sha256=9d8bccc22fb3928c2296758a5fba55054c09b010e56aa347762177dfbe6ee881 ; sys_platform == 'linux'",
)


def _gguf(name: str, revision: str, sha256: str, size: int) -> Gguf:
    # Q8_0, the quantisation Handy uses. Revisions are the newest that are at
    # least 14 days old and match transcribe.cpp 0.3.1's model catalog.
    return Gguf(f"handy-computer/{name}-gguf", f"{name}-Q8_0.gguf", revision, sha256, size)


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
        Backend(
            "ggml",
            "transcribe.cpp (ggml)",
            ("numpy", "huggingface_hub", *TRANSCRIBE_CPP),
            # The CUDA build, for `speechify bench` on NVIDIA machines. It
            # includes the Vulkan and CPU builds too.
            cuda=TRANSCRIBE_CPP_CUDA,
        ),
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
            gguf=_gguf("whisper-large-v3-turbo", "ceea6c8a94a21ab85be244d311e874a39344dbf5",
                       "b2e30cc286bc9f3aba4db9099fc7403543497c05ce7100d0d83091ddfd25a183", 886381760),
        ),
        ModelSpec(
            "whisper-large-v3", "Whisper large-v3", "whisper", "large-v3",
            "99 languages", accuracy=3.5, speed=2.5, vram_gb=4, download_gb=3.1,
            prompt=True, finetune=True,
            gguf=_gguf("whisper-large-v3", "b33a05f1459f33b0c876f014a57d51618b77d754",
                       "2fa1a5f179f8a511a53e2108db270aa4af3ce08cd976af4180e2854666bb4ba3", 1668741440),
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
            gguf=_gguf("parakeet-tdt-0.6b-v3", "90f082450fcbacdb54e5900c44ef697c9ea59622",
                       "5859f77944efcd8eafa23a6350731960b2b55b2203df51f319665c807d802cc7", 739508576),
        ),
        ModelSpec(
            "parakeet-tdt-0.6b-v2", "NVIDIA Parakeet TDT 0.6B v2", "onnx", "nemo-parakeet-tdt-0.6b-v2",
            "English", accuracy=4, speed=5, vram_gb=2, download_gb=2.5,
            notes="English-only predecessor of v3; slightly better on English.",
            gguf=_gguf("parakeet-tdt-0.6b-v2", "382796e405a9fe37145938dccbabfb25e93e4286",
                       "f0d0e99cebb6d3b83f1f7069b82b5d3c2e39a54545b0da039cb4bafd9c4e5caa", 729574912),
        ),
        ModelSpec(
            "canary-1b-v2", "NVIDIA Canary 1B v2", "onnx", "nemo-canary-1b-v2",
            "25 European languages", accuracy=4, speed=4, vram_gb=3, download_gb=4,
            gguf=_gguf("canary-1b-v2", "e2d8e6d7f2accc1259dc5497b517b4083047e44b",
                       "224f83d1bc487b3303b495a7d6874912fdece93de19d1a04b550829c30a5d289", 1144290016),
        ),
        # --- LLM-based recognizers: most accurate, a bit slower --------------
        ModelSpec(
            "granite-speech-4.1-2b", "IBM Granite Speech 4.1 2B", "transformers",
            "ibm-granite/granite-speech-4.1-2b",
            "en, fr, de, es, pt, ja", accuracy=5, speed=3, vram_gb=6, download_gb=4.5,
            notes="#1 on the Open ASR Leaderboard (5.33% WER).",
            gguf=_gguf("granite-speech-4.1-2b", "8636dae5eebb011a3d9b56215944ed0925a0d035",
                       "8c0b2dce2861feb1dba91e9d59f2ab4a9c23ad808bff86465151576e096ec7a6", 2559878848),
        ),
        ModelSpec(
            "cohere-transcribe", "Cohere Transcribe (03-2026)", "transformers",
            "CohereLabs/cohere-transcribe-03-2026",
            "14 languages", accuracy=4.5, speed=3, vram_gb=5, download_gb=4.2, gated=True,
            notes="Gated: accept the terms on its Hugging Face page and run `hf auth login` first.",
            gguf=_gguf("cohere-transcribe-03-2026", "ab667acedcb5d8d56837ef3ed3568c3ad50dde47",
                       "931916663432fd895423a4291a8400221802b288967ca2d435fc5e3141c9e71e", 2410655232),
        ),
        ModelSpec(
            "qwen3-asr-1.7b", "Qwen3-ASR 1.7B", "qwen", "Qwen/Qwen3-ASR-1.7B",
            "52 languages and dialects", accuracy=4.5, speed=3.5, vram_gb=5, download_gb=4.4,
            gguf=_gguf("Qwen3-ASR-1.7B", "3555bd238a8572bbace3ebf60d23b036dc0a5dbe",
                       "9a0d81792dfea2d5f278b8a63deb3ea6e02139ce42c2301f32ea19c4f77526b7", 2185030624),
        ),
        ModelSpec(
            "qwen3-asr-0.6b", "Qwen3-ASR 0.6B", "qwen", "Qwen/Qwen3-ASR-0.6B",
            "52 languages and dialects", accuracy=4, speed=4, vram_gb=2.5, download_gb=1.9,
            gguf=_gguf("Qwen3-ASR-0.6B", "28c66fba077f1359a42b52c55b618a24cdb6e0e7",
                       "f081b2d5e23bd669d92cc331d722a8a0681943b8e6f34b48996fd5c319b5acd8", 850423456),
        ),
        ModelSpec(
            "voxtral-mini-3b", "Mistral Voxtral Mini 3B", "transformers", "mistralai/Voxtral-Mini-3B-2507",
            "8 languages", accuracy=4, speed=2.5, vram_gb=9, download_gb=9.4,
            notes="Needs ~9 GB of VRAM.",
            gguf=_gguf("Voxtral-Mini-3B-2507", "b9e655d7a09a24394edf457e08651163ad22732d",
                       "06b4dd04e32953ccab0d417125bdd728ec031431596ed5e80123c3c66f29ddc2", 5000084128),
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
            gguf=_gguf("moonshine-base", "cb703342553d960a95e2b4b0f48b8142648ddb22",
                       "7f0027dfd857d310b63a85ef57cadf183da712cc374f85a648f8bc18aaa2efc8", 77476480),
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


# Devices that run models through transcribe.cpp when it has them (config.DEVICES).
GGML_DEVICES = ("vulkan", "metal")


def placed(spec: ModelSpec, device: str | None) -> ModelSpec:
    """`spec` as it runs on `device` (a resolved config device).

    On Vulkan and Metal, models with a GGUF build run on transcribe.cpp;
    everything else, and every model on CUDA and the CPU, keeps its own
    backend. Same id either way, so the model you picked stays picked.
    """
    if spec.gguf is not None and device in GGML_DEVICES:
        return as_ggml(spec)
    return spec


def as_ggml(spec: ModelSpec) -> ModelSpec:
    """The transcribe.cpp build of `spec`, whatever the device (`speechify bench`)."""
    if spec.backend == "ggml":
        return spec
    if spec.gguf is None:
        raise KeyError(f"{spec.name} has no transcribe.cpp build")
    # The GGUF repos aren't gated, so Cohere needs no Hugging Face login here.
    return replace(spec, backend="ggml", repo=spec.gguf.repo, origin=spec.backend,
                   download_gb=spec.gguf.size_gb, gated=False)


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
