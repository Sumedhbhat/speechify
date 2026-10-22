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


def mean_logprob(torch, steps, tokens) -> float | None:
    """Mean log-probability of the generated `tokens`, given the logits
    `steps` that generate() returned for them (one tensor per step)."""
    values = []
    for logits, token in zip(steps, tokens):
        lp = torch.log_softmax(logits[0].float(), dim=-1)[int(token)].item()
        if lp == lp and lp != float("-inf"):  # skip NaN / -inf
            values.append(lp)
    return sum(values) / len(values) if values else None


def keywords(prompt: str | None) -> str:
    """Learner.prompt() ("Glossary: a, b.") as a plain list: "a, b"."""
    if not prompt:
        return ""
    text = prompt.strip()
    if text.startswith("Glossary:"):
        text = text[len("Glossary:"):].strip().rstrip(".")
    return text


class Engine:
    """opts: device ("cuda"/"cpu"), compute_type, language ("" = auto), beam_size."""

    device: str

    def __init__(self, repo: str, opts: dict):
        raise NotImplementedError

    def transcribe(self, audio: np.ndarray, prompt: str | None) -> str:
        raise NotImplementedError

    def transcribe_scored(self, audio: np.ndarray, prompt: str | None) -> tuple[str, float | None]:
        """Text plus the mean token log-probability, for teacher labels.
        None when the backend doesn't expose token probabilities."""
        return self.transcribe(audio, prompt), None

    @classmethod
    def fetch(cls, repo: str, opts: dict) -> bool:
        """Download the weights without loading the model. False means this
        backend can't, and the worker loads the model once instead."""
        return False


def hf_fetch(repo: str) -> bool:
    from huggingface_hub import snapshot_download

    snapshot_download(repo)
    return True


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
        # nearly twice as fast on the CPU, with the same transcripts. compute_type is "int8" by default
        # on machines without CUDA (config.platform_defaults).
        wanted = "int8" if not accelerated and opts.get("compute_type") == "int8" else None
        quantization = wanted
        if path is not None and path.exists():
            # onnx-asr never downloads into a folder that exists, so use the
            # weights that are there, whichever precision was fetched first.
            has_int8 = any(path.glob("*.int8.onnx"))
            has_full = any(".int8." not in f.name for f in path.glob("*.onnx"))
            if quantization and not has_int8:
                quantization = None
            elif not quantization and has_int8 and not has_full:
                quantization = "int8"
            if quantization != wanted:
                kind = "int8" if quantization else "full-precision"
                print(f"Using the {kind} weights already downloaded; delete {path} "
                      "to download the other kind.", file=sys.stderr)
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
            self.model = onnx_asr.load_model(repo, path, quantization=quantization,
                                             providers=["CPUExecutionProvider"])
            self.device = "CPUExecutionProvider" + (" (int8)" if quantization else "")
        self.language = opts["language"] or None
        self.multilingual = "canary" in repo

    def transcribe(self, audio, prompt):
        if self.multilingual and self.language:
            return self.model.recognize(audio, sample_rate=SAMPLE_RATE, language=self.language)
        return self.model.recognize(audio, sample_rate=SAMPLE_RATE)


class TransformersEngine(Engine):
    """Granite Speech, Cohere Transcribe, Voxtral, Kyutai STT and Moonshine."""

    fetch = classmethod(lambda cls, repo, opts: hf_fetch(repo))

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
        self.prompt = self._granite_prompt(None)

    def _granite_prompt(self, vocabulary: str | None) -> str:
        instruction = "<|audio|>transcribe the speech with proper punctuation and capitalization."
        words = keywords(vocabulary)
        if words:  # Granite 4.1's keyword biasing format, from its model card
            instruction += f" Keywords: {words}"
        chat = [{"role": "user", "content": instruction}]
        return self.processor.tokenizer.apply_chat_template(chat, tokenize=False, add_generation_prompt=True)

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

    def transcribe_scored(self, audio, prompt):
        if self.kind != "granite":
            return self.transcribe(audio, prompt), None
        with self.torch.inference_mode():
            return self._granite(audio, self._granite_prompt(prompt), scored=True)

    def _run_granite(self, audio):
        return self._granite(audio, self.prompt, scored=False)[0]

    def _granite(self, audio, prompt, scored):
        wav = self.torch.from_numpy(audio).unsqueeze(0)
        inputs = self.processor(prompt, wav, device=self.device, return_tensors="pt").to(self.device)
        # ~3 tokens a word and up to ~5 words a second; 200 covers a normal dictation.
        max_new = max(200, int(audio.size / SAMPLE_RATE * 15) + 20)
        out = self.model.generate(**inputs, max_new_tokens=max_new, do_sample=False, num_beams=1,
                                  return_dict_in_generate=True, output_logits=scored)
        new_tokens = out.sequences[:, inputs["input_ids"].shape[-1]:]
        text = self.processor.tokenizer.batch_decode(new_tokens, skip_special_tokens=True)[0]
        score = mean_logprob(self.torch, out.logits, new_tokens[0]) if scored and out.logits else None
        return text, score

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
    fetch = classmethod(lambda cls, repo, opts: hf_fetch(repo))

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
        results = self.model.transcribe(audio=(audio, SAMPLE_RATE), context=prompt or "",
                                        language=self.language)
        return results[0].text

    def transcribe_scored(self, audio, prompt):
        """Same as transcribe(), plus token probabilities.

        qwen-asr's transcribe() doesn't return them, so this repeats its
        single-chunk path with logits switched on. It leans on qwen-asr
        internals; if they change, it falls back to plain text, and the
        label gets the heuristic confidence instead.
        """
        import torch

        try:
            from qwen_asr.inference.utils import parse_asr_output

            m = self.model
            text_prompt = m._build_text_prompt(context=prompt or "", force_language=self.language)
            inputs = m.processor(text=[text_prompt], audio=[audio], return_tensors="pt", padding=True)
            inputs = inputs.to(m.model.device).to(m.model.dtype)
            max_new = max(256, int(audio.size / SAMPLE_RATE * 15) + 20)
            with torch.inference_mode():
                out = m.model.generate(**inputs, max_new_tokens=max_new, output_logits=True)
            new_tokens = out.sequences[:, inputs["input_ids"].shape[1]:]
            raw = m.processor.batch_decode(new_tokens, skip_special_tokens=True,
                                           clean_up_tokenization_spaces=False)[0]
            _, text = parse_asr_output(raw, user_language=self.language)
            score = mean_logprob(torch, out.logits, new_tokens[0]) if out.logits else None
            return text, score
        except Exception as exc:
            print(f"qwen-asr scoring unavailable ({type(exc).__name__}: {exc}); using plain transcribe",
                  file=sys.stderr)
            return self.transcribe(audio, prompt), None


BACKEND_ENGINES: dict[str, type[Engine]] = {
    "whisper": WhisperEngine,
    "onnx": OnnxEngine,
    "transformers": TransformersEngine,
    "qwen": QwenEngine,
}
