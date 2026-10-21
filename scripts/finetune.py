# /// script
# requires-python = ">=3.11,<3.13"
# dependencies = [
#     "numpy",
#     "torch>=2.4",
#     "transformers>=4.45,<5",  # the CTranslate2 converter targets transformers 4.x
#     "peft>=0.13",
#     "ctranslate2>=4.5",
# ]
#
# [tool.uv]
# exclude-newer = "14 days"
#
# # PyPI only has CPU builds of torch for Windows.
# [[tool.uv.index]]
# name = "pytorch-cu126"
# url = "https://download.pytorch.org/whl/cu126"
# explicit = true
#
# [tool.uv.sources]
# torch = { index = "pytorch-cu126" }
# ///
"""LoRA fine-tune Whisper on your own dictations, then convert it for faster-whisper.

Trains on every saved recording that isn't excluded in the History window:
ones you edited use your text, the rest use what was pasted. Each run
starts again from the base model and trains on everything, so mistakes from an
earlier run can't compound.

A held-out slice of your samples is transcribed before and after training.
The new model is only written if it gets a lower word error rate on that
slice than the base model.

    uv run scripts/finetune.py            # train and report
    uv run scripts/finetune.py --apply    # also switch Speechify to the new model

uv reads the dependencies above and builds a separate environment for them,
so training libraries never touch the app's own environment.
"""

from __future__ import annotations

import argparse
import random
import sqlite3
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from speechify.audio import SAMPLE_RATE, load_wav  # noqa: E402
from speechify.config import Config, data_dir, set_value  # noqa: E402
from speechify.metrics import wer  # noqa: E402

BASE_MODELS = {
    "whisper-large-v3-turbo": "openai/whisper-large-v3-turbo",
    "whisper-large-v3": "openai/whisper-large-v3",
    # Legacy config names.
    "large-v3-turbo": "openai/whisper-large-v3-turbo",
    "large-v3": "openai/whisper-large-v3",
}


def load_samples(root: Path, include_accepted: bool) -> list[tuple[Path, str]]:
    db = sqlite3.connect(root / "learning.db")
    rows = db.execute(
        "SELECT audio, coalesce(corrected, pasted), corrected IS NOT NULL "
        "FROM samples WHERE audio IS NOT NULL AND excluded = 0"
    ).fetchall()
    samples = []
    for name, text, corrected in rows:
        path = root / "audio" / name
        if (corrected or include_accepted) and text.strip() and path.exists():
            samples.append((path, text.strip()))
    return samples


def main() -> None:
    cfg = Config.load()
    default_base = BASE_MODELS.get(cfg.model, "openai/whisper-large-v3-turbo")
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--base", default=default_base, help="Hugging Face model to start from")
    p.add_argument("--epochs", type=int, default=3)
    p.add_argument("--lr", type=float, default=3e-4)
    p.add_argument("--batch-size", type=int, default=4)
    p.add_argument("--rank", type=int, default=32, help="LoRA rank")
    p.add_argument("--min-samples", type=int, default=50)
    p.add_argument("--corrected-only", action="store_true", help="ignore accepted dictations")
    p.add_argument("--force", action="store_true", help="save even if WER didn't improve")
    p.add_argument("--apply", action="store_true", help="update config.toml to use the new model")
    args = p.parse_args()

    import torch
    from peft import LoraConfig, get_peft_model
    from transformers import (
        AutoTokenizer,
        WhisperForConditionalGeneration,
        WhisperProcessor,
        get_linear_schedule_with_warmup,
    )

    root = data_dir()
    samples = load_samples(root, include_accepted=not args.corrected_only)
    if len(samples) < args.min_samples:
        sys.exit(
            f"Only {len(samples)} samples saved; need at least {args.min_samples}. Keep dictating "
            "and fixing mistakes (keep_all_audio = true collects them fastest)."
        )
    random.Random(0).shuffle(samples)
    n_eval = max(5, len(samples) // 10)
    eval_set, train_set = samples[:n_eval], samples[n_eval:]
    print(f"{len(train_set)} training samples, {len(eval_set)} held out for evaluation")

    if not torch.cuda.is_available():
        sys.exit("CUDA isn't available to PyTorch. Install a CUDA build: https://pytorch.org/get-started/locally/")
    device = "cuda"
    dtype = torch.bfloat16 if torch.cuda.is_bf16_supported() else torch.float16
    language = cfg.language or "en"

    processor = WhisperProcessor.from_pretrained(args.base, language=language, task="transcribe")
    model = WhisperForConditionalGeneration.from_pretrained(args.base, torch_dtype=dtype).to(device)
    model.generation_config.forced_decoder_ids = None
    start_token = model.config.decoder_start_token_id

    def features(batch: list[tuple[Path, str]]) -> torch.Tensor:
        audio = [load_wav(path) for path, _ in batch]
        feats = processor.feature_extractor(audio, sampling_rate=SAMPLE_RATE, return_tensors="pt")
        return feats.input_features.to(device, dtype)

    def labels(batch: list[tuple[Path, str]]) -> torch.Tensor:
        ids = [processor.tokenizer(text).input_ids for _, text in batch]
        # The model prepends the start token itself when shifting labels right.
        ids = [x[1:] if x and x[0] == start_token else x for x in ids]
        width = max(len(x) for x in ids)
        return torch.tensor([x + [-100] * (width - len(x)) for x in ids], device=device)

    @torch.no_grad()
    def evaluate(m) -> float:
        m.eval()
        hyps = []
        for i in range(0, len(eval_set), args.batch_size):
            batch = eval_set[i : i + args.batch_size]
            out = m.generate(input_features=features(batch), language=language,
                             task="transcribe", max_new_tokens=220)
            hyps += processor.batch_decode(out, skip_special_tokens=True)
        return wer([t for _, t in eval_set], hyps)

    base_wer = evaluate(model)
    print(f"Base model WER on your held-out audio: {base_wer:.1%}")

    model = get_peft_model(model, LoraConfig(
        r=args.rank,
        lora_alpha=args.rank * 2,
        lora_dropout=0.05,
        target_modules=["q_proj", "k_proj", "v_proj", "out_proj"],
    ))
    model.print_trainable_parameters()

    params = [p for p in model.parameters() if p.requires_grad]
    optim = torch.optim.AdamW(params, lr=args.lr, weight_decay=0.01)
    steps = args.epochs * ((len(train_set) + args.batch_size - 1) // args.batch_size)
    sched = get_linear_schedule_with_warmup(optim, max(1, steps // 10), steps)
    scaler = torch.amp.GradScaler(enabled=dtype == torch.float16)

    t0 = time.time()
    for epoch in range(args.epochs):
        model.train()
        random.Random(epoch).shuffle(train_set)
        total = 0.0
        for i in range(0, len(train_set), args.batch_size):
            batch = train_set[i : i + args.batch_size]
            with torch.autocast(device, dtype=dtype):
                loss = model(input_features=features(batch), labels=labels(batch)).loss
            scaler.scale(loss).backward()
            scaler.unscale_(optim)
            torch.nn.utils.clip_grad_norm_(params, 1.0)
            scaler.step(optim)
            scaler.update()
            optim.zero_grad(set_to_none=True)
            sched.step()
            total += loss.item() * len(batch)
        print(f"epoch {epoch + 1}/{args.epochs}  loss {total / len(train_set):.4f}  "
              f"({time.time() - t0:.0f}s)")

    tuned_wer = evaluate(model)
    print(f"Fine-tuned WER: {tuned_wer:.1%}  (base {base_wer:.1%})")
    if tuned_wer >= base_wer and not args.force:
        sys.exit("No improvement, so the model wasn't saved. Collect more samples, or pass --force.")

    stamp = time.strftime("%Y%m%d-%H%M")
    hf_dir = root / "models" / f"hf-{stamp}"
    out_dir = root / "models" / f"whisper-ft-{stamp}"
    merged = model.merge_and_unload()
    merged.save_pretrained(hf_dir)
    processor.save_pretrained(hf_dir)
    AutoTokenizer.from_pretrained(args.base).save_pretrained(hf_dir)  # fast tokenizer writes tokenizer.json

    from ctranslate2.converters import TransformersConverter

    TransformersConverter(
        str(hf_dir), copy_files=["tokenizer.json", "preprocessor_config.json"]
    ).convert(str(out_dir), quantization="float16", force=True)
    print(f"Saved faster-whisper model to {out_dir}")

    model_path = out_dir.as_posix()
    if args.apply:
        set_value("model", model_path)
        print("Switched Speechify to it. Restart Speechify, or pick it with Ctrl+Alt+M.")
    else:
        print("It's listed in the model window (Ctrl+Alt+M), or rerun with --apply to switch to it.")

if __name__ == "__main__":
    main()
