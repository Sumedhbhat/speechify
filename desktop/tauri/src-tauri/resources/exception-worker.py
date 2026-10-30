"""Optional persistent inference worker: Distil-Whisper, trained CT2, Kyutai only.
No microphone, hooks, settings, history, scheduler or UI ownership.
The Rust parent supplies candidate-owned local model paths, supervises and kills it.
"""

import sys
import json
import time
import wave
import os
from pathlib import Path

path, device, config = Path(sys.argv[1]), sys.argv[2], json.loads(sys.argv[3])
# Libraries may print to stdout: preserve the private JSONL protocol separately.
protocol = sys.stdout
sys.stdout = sys.stderr
import numpy as np

start = time.perf_counter()
if (path / "model.bin").exists():
    from faster_whisper import WhisperModel

    device = "cuda" if device == "cuda" else "cpu"
    engine = WhisperModel(
        str(path),
        device=device,
        compute_type=config["compute_type"] if device == "cuda" else "int8",
        local_files_only=True,
    )

    def run(audio, prompt):
        parts, _ = engine.transcribe(
            audio,
            language=config["language"] or None,
            beam_size=config["beam_size"],
            initial_prompt="Hello. A clear, punctuated English transcript. Glossary: " + prompt,
            vad_filter=True,
        )
        return " ".join(p.text.strip() for p in parts)
else:
    if not path.name.startswith("kyutai-stt-"):
        raise RuntimeError("This worker supports Kyutai and CTranslate2 models only")
    import torch
    from transformers import KyutaiSpeechToTextProcessor, KyutaiSpeechToTextForConditionalGeneration

    device = "cuda" if device in ("cuda", "auto") and torch.cuda.is_available() else "cpu"
    processor = KyutaiSpeechToTextProcessor.from_pretrained(str(path), local_files_only=True)
    engine = (
        KyutaiSpeechToTextForConditionalGeneration.from_pretrained(str(path), local_files_only=True)
        .to(device)
        .eval()
    )

    def run(audio, prompt):
        from scipy.signal import resample_poly

        inputs = processor(
            audio=resample_poly(audio, 3, 2), sampling_rate=24000, return_tensors="pt"
        ).to(device)
        with torch.inference_mode():
            out = engine.generate(**inputs, max_new_tokens=1024)
        return processor.batch_decode(out, skip_special_tokens=True)[0]


def send(obj):
    protocol.write(json.dumps(obj) + "\n")
    protocol.flush()


send(
    {
        "ready": True,
        "device": device + " (Python exception)",
        "load_ms": (time.perf_counter() - start) * 1000,
    }
)
for line in sys.stdin:
    try:
        if len(line) > 32768:
            raise ValueError("Message too large")
        req = json.loads(line)
        with wave.open(req["wav"]) as w:
            if w.getframerate() != 16000 or w.getnchannels() != 1 or w.getsampwidth() != 2:
                raise ValueError("Expected 16kHz mono PCM16")
            if w.getnframes() > 16000 * 600:
                raise ValueError("Clip exceeds ten minutes")
            audio = (
                np.frombuffer(w.readframes(w.getnframes()), dtype="<i2").astype(np.float32) / 32768
            )
        start = time.perf_counter()
        text = run(audio, req.get("prompt", ""))
        send(
            {
                "text": text,
                "device": device + " (Python exception)",
                "inference_ms": (time.perf_counter() - start) * 1000,
            }
        )
    except Exception as e:
        send({"error": str(e)})
