"""Model loading and inference. Runs inside a worker process (see worker.py).

Each backend imports its libraries inside __init__, so this module can be
imported by any environment and only needs numpy.
"""

from __future__ import annotations

import os
import sys
import tempfile
import wave
from pathlib import Path

import numpy as np

SAMPLE_RATE = 16_000

# Qwen3-ASR takes language names, not codes.
LANGUAGE_NAMES = {
    "en": "English", "zh": "Chinese", "fr": "French", "de": "German", "es": "Spanish",
    "it": "Italian", "pt": "Portuguese", "ja": "Japanese", "ko": "Korean", "ru": "Russian",
    "ar": "Arabic", "nl": "Dutch", "hi": "Hindi", "vi": "Vietnamese", "pl": "Polish",
}


def add_nvidia_dll_dirs() -> None:
    """Expose pip-installed CUDA/cuDNN DLLs (site-packages/nvidia/*/bin) on Windows."""
    if sys.platform != "win32":
        return
    for entry in sys.path:
        nvidia = Path(entry) / "nvidia"
        if not nvidia.is_dir():
            continue
        for bin_dir in nvidia.glob("*/bin"):
            os.add_dll_directory(str(bin_dir))
            os.environ["PATH"] = f"{bin_dir}{os.pathsep}{os.environ['PATH']}"


def resample(audio: np.ndarray, rate: int) -> np.ndarray:
    if rate == SAMPLE_RATE:
        return audio
    n = int(round(audio.size * rate / SAMPLE_RATE))
    return np.interp(np.linspace(0, audio.size - 1, n), np.arange(audio.size), audio).astype(np.float32)


class Engine:
    """opts: device ("cuda"/"cpu"), compute_type, language ("" = auto), beam_size."""

    device: str

    def __init__(self, repo: str, opts: dict):
        raise NotImplementedError

    def transcribe(self, audio: np.ndarray, prompt: str | None) -> str:
        raise NotImplementedError


class WhisperEngine(Engine):
    def __init__(self, repo, opts):
        device, compute_type = opts["device"], opts["compute_type"]
        add_nvidia_dll_dirs()
        from faster_whisper import WhisperModel

        try:
            self.model = WhisperModel(repo, device=device, compute_type=compute_type)
            self.device = device
        except Exception as exc:
            if device != "cuda":
                raise
            print(f"CUDA unavailable ({exc}); falling back to CPU int8.", file=sys.stderr)
            self.model = WhisperModel(repo, device="cpu", compute_type="int8")
            self.device = "cpu"
        self.language = opts["language"] or None
        self.beam_size = opts["beam_size"]

    def transcribe(self, audio, prompt):
        segments, _ = self.model.transcribe(
            audio,
            language=self.language,
            beam_size=self.beam_size,
            initial_prompt=prompt or None,
            condition_on_previous_text=False,
            without_timestamps=True,
            vad_filter=True,
            vad_parameters={"min_silence_duration_ms": 500, "speech_pad_ms": 200},
        )
        return " ".join(s.text.strip() for s in segments)


class OnnxEngine(Engine):
    def __init__(self, repo, opts):
        device = opts["device"]
        import onnxruntime as ort

        if hasattr(ort, "preload_dlls"):
            # Loads the CUDA/cuDNN DLLs shipped by onnxruntime-gpu[cuda,cudnn].
            ort.preload_dlls()
        import onnx_asr

        available = ort.get_available_providers()
        accelerated = []
        if device == "cuda" and "CUDAExecutionProvider" in available:
            accelerated = ["CUDAExecutionProvider"]
        elif "DmlExecutionProvider" in available:
            accelerated = ["DmlExecutionProvider"]
        # A plain folder of real files: Hugging Face's cache stores weights as
        # links into a shared blob folder, and ONNX Runtime refuses external
        # weight files that resolve outside the model's own directory.
        path = Path(opts["models_dir"]) / repo if opts.get("models_dir") else None
        # Without a GPU, the int8 weights are a quarter of the download and
        # several times faster on the CPU. compute_type is "int8" by default
        # on machines without CUDA (config.platform_defaults).
        quantization = "int8" if not accelerated and opts.get("compute_type") == "int8" else None
        if quantization and path is not None and path.exists() and not any(path.glob("*.int8.onnx")):
            # onnx-asr never downloads into a folder that exists, so a model
            # first downloaded at full precision stays that way.
            print("Using the full-precision weights already downloaded; delete "
                  f"{path} to get the faster int8 ones.", file=sys.stderr)
            quantization = None
        try:
            self.model = onnx_asr.load_model(repo, path, quantization=quantization,
                                             providers=accelerated + ["CPUExecutionProvider"])
            self.device = (accelerated or ["CPUExecutionProvider"])[0]
            if quantization:
                self.device += " (int8)"
        except Exception as exc:
            if not accelerated:
                raise
            # ONNX Runtime lists CUDA whenever onnxruntime-gpu is installed, then
            # fails to create the session if the GPU or driver can't be used.
            print(f"{accelerated[0]} unavailable ({str(exc)[:200]}); using the CPU.", file=sys.stderr)
            self.model = onnx_asr.load_model(repo, path, providers=["CPUExecutionProvider"])
            self.device = "CPUExecutionProvider"
        self.language = opts["language"] or None
        self.multilingual = "canary" in repo

    def transcribe(self, audio, prompt):
        if self.multilingual and self.language:
            return self.model.recognize(audio, sample_rate=SAMPLE_RATE, language=self.language)
        return self.model.recognize(audio, sample_rate=SAMPLE_RATE)


class TransformersEngine(Engine):
    """Granite Speech, Cohere Transcribe, Voxtral, Kyutai STT and Moonshine."""

    def __init__(self, repo, opts):
        import torch

        self.torch = torch
        self.repo = repo
        self.language = opts["language"] or None
        self.device = "cuda" if opts["device"] == "cuda" and torch.cuda.is_available() else "cpu"
        if self.device == "cuda":
            self.dtype = torch.bfloat16 if torch.cuda.is_bf16_supported() else torch.float16
        else:
            self.dtype = torch.float32
        family = repo.lower()
        if "granite" in family:
            self.kind = "granite"
        elif "cohere" in family:
            self.kind = "cohere"
        elif "voxtral" in family:
            self.kind = "voxtral"
        elif "kyutai" in family:
            self.kind = "kyutai"
        elif "moonshine" in family:
            self.kind = "moonshine"
        else:
            raise ValueError(f"no transformers recipe for {repo}")
        getattr(self, f"_load_{self.kind}")()

    # --- loading -------------------------------------------------------------

    def _load_granite(self):
        from transformers import AutoModelForSpeechSeq2Seq, AutoProcessor

        self.processor = AutoProcessor.from_pretrained(self.repo)
        self.model = AutoModelForSpeechSeq2Seq.from_pretrained(
            self.repo, device_map=self.device, dtype=self.dtype
        )
        chat = [{"role": "user", "content": "<|audio|>transcribe the speech with proper punctuation and capitalization."}]
        self.prompt = self.processor.tokenizer.apply_chat_template(
            chat, tokenize=False, add_generation_prompt=True
        )

    def _load_cohere(self):
        from transformers import AutoProcessor, CohereAsrForConditionalGeneration

        self.processor = AutoProcessor.from_pretrained(self.repo)
        self.model = CohereAsrForConditionalGeneration.from_pretrained(
            self.repo, device_map=self.device, dtype=self.dtype
        )

    def _load_voxtral(self):
        from transformers import AutoProcessor, VoxtralForConditionalGeneration

        self.processor = AutoProcessor.from_pretrained(self.repo)
        self.model = VoxtralForConditionalGeneration.from_pretrained(
            self.repo, device_map=self.device, dtype=self.dtype
        )

    def _load_kyutai(self):
        from transformers import (
            KyutaiSpeechToTextForConditionalGeneration,
            KyutaiSpeechToTextProcessor,
        )

        self.processor = KyutaiSpeechToTextProcessor.from_pretrained(self.repo)
        self.model = KyutaiSpeechToTextForConditionalGeneration.from_pretrained(
            self.repo, device_map=self.device, dtype=self.dtype
        )

    def _load_moonshine(self):
        from transformers import AutoProcessor, MoonshineForConditionalGeneration

        self.processor = AutoProcessor.from_pretrained(self.repo)
        self.model = MoonshineForConditionalGeneration.from_pretrained(self.repo, dtype=self.dtype).to(self.device)

    # --- inference -----------------------------------------------------------

    def transcribe(self, audio, prompt):
        with self.torch.inference_mode():
            return getattr(self, f"_run_{self.kind}")(audio)

    def _run_granite(self, audio):
        wav = self.torch.from_numpy(audio).unsqueeze(0)
        inputs = self.processor(self.prompt, wav, device=self.device, return_tensors="pt").to(self.device)
        out = self.model.generate(**inputs, max_new_tokens=200, do_sample=False, num_beams=1)
        new_tokens = out[:, inputs["input_ids"].shape[-1]:]
        return self.processor.tokenizer.batch_decode(new_tokens, skip_special_tokens=True)[0]

    def _run_cohere(self, audio):
        inputs = self.processor(audio, sampling_rate=SAMPLE_RATE, return_tensors="pt",
                                language=self.language or "en")
        inputs = inputs.to(self.model.device, dtype=self.model.dtype)
        out = self.model.generate(**inputs, max_new_tokens=256)
        text = self.processor.decode(out, skip_special_tokens=True)
        return text if isinstance(text, str) else " ".join(text)

    def _run_voxtral(self, audio):
        # apply_transcription_request takes a path or URL, so hand it a temp WAV.
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "clip.wav"
            with wave.open(str(path), "wb") as f:
                f.setnchannels(1)
                f.setsampwidth(2)
                f.setframerate(SAMPLE_RATE)
                f.writeframes((np.clip(audio, -1, 1) * 32767).astype(np.int16).tobytes())
            kwargs = {"language": self.language} if self.language else {}
            inputs = self.processor.apply_transcription_request(audio=str(path), model_id=self.repo, **kwargs)
        inputs = inputs.to(self.device, dtype=self.dtype)
        out = self.model.generate(**inputs, max_new_tokens=500)
        return self.processor.batch_decode(out[:, inputs.input_ids.shape[1]:], skip_special_tokens=True)[0]

    def _run_kyutai(self, audio):
        # By keyword: in transformers 5 the processor's first positional argument is `images`.
        inputs = self.processor(audio=resample(audio, 24_000), sampling_rate=24_000,
                                return_tensors="pt").to(self.device)
        out = self.model.generate(**inputs)
        return self.processor.batch_decode(out, skip_special_tokens=True)[0]

    def _run_moonshine(self, audio):
        inputs = self.processor(audio, sampling_rate=SAMPLE_RATE, return_tensors="pt")
        inputs = inputs.to(self.device, dtype=self.dtype)
        # Moonshine's model card caps output at ~6.5 tokens per second of audio.
        max_length = int(audio.size / SAMPLE_RATE * 6.5) + 10
        out = self.model.generate(**inputs, max_length=max_length)
        return self.processor.batch_decode(out, skip_special_tokens=True)[0]


class QwenEngine(Engine):
    def __init__(self, repo, opts):
        import torch
        from qwen_asr import Qwen3ASRModel

        language = opts["language"]
        cuda = opts["device"] == "cuda" and torch.cuda.is_available()
        self.device = "cuda" if cuda else "cpu"
        self.model = Qwen3ASRModel.from_pretrained(
            repo,
            dtype=torch.bfloat16 if cuda else torch.float32,
            device_map="cuda:0" if cuda else "cpu",
            max_inference_batch_size=1,
            max_new_tokens=256,
        )
        self.language = LANGUAGE_NAMES.get(language) if language else None

    def transcribe(self, audio, prompt):
        results = self.model.transcribe(audio=(audio, SAMPLE_RATE), language=self.language)
        return results[0].text


BACKEND_ENGINES: dict[str, type[Engine]] = {
    "whisper": WhisperEngine,
    "onnx": OnnxEngine,
    "transformers": TransformersEngine,
    "qwen": QwenEngine,
}
