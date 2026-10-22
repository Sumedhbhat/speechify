# /// script
# requires-python = ">=3.11,<3.13"
# dependencies = [
#     "numpy",
#     "torch>=2.4",
#     "transformers>=4.45,<5",  # the CTranslate2 converter targets transformers 4.x
#     "peft>=0.13",
#     "ctranslate2>=4.5",
#     "soundfile",  # decodes the LibriSpeech regression clips (FLAC), once
# ]
#
# [tool.uv]
# exclude-newer = "14 days"
#
# # PyPI only has CPU builds of torch for Windows. The CUDA index has no Mac
# # builds, and Macs can't fine-tune anyway, so they get PyPI's and a clear error.
# [[tool.uv.index]]
# name = "pytorch-cu126"
# url = "https://download.pytorch.org/whl/cu126"
# explicit = true
#
# [tool.uv.sources]
# torch = { index = "pytorch-cu126", marker = "sys_platform != 'darwin'" }
# ///
"""LoRA fine-tune Whisper on your own dictations, then convert it for faster-whisper.

Trains on every saved recording that has a training label (see
src/speechify/labels.py): your correction where you made one, otherwise the
teacher model's transcript if it was confident. What Speechify pasted is
never used, so the model can't learn its own mistakes. Each run starts again
from the base model and trains on everything.

A new model is only saved if it passes two gates:

1. Your audio: lower word error rate than the model you use now (not just
   the base model) on a held-out slice of your recordings. Recordings you
   corrected yourself are the held-out set once there are 20 of them; until
   then, confident teacher labels are.
2. LibriSpeech: on 50 fixed test-clean clips (downloaded once), no more
   than --max-regression WER points worse than the base model, so it can't
   get better at your voice by forgetting general English.

    uv run scripts/finetune.py            # train and report
    uv run scripts/finetune.py --apply    # also switch Speechify to the new model

Speechify runs this by itself while you're away (src/speechify/autolearn.py,
with --auto and --report). uv reads the dependencies above and builds a
separate environment for them, so training libraries never touch the app's
own environment.
"""

from __future__ import annotations

import argparse
import io
import json
import random
import sys
import tarfile
import time
import traceback
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from speechify.audio import SAMPLE_RATE, load_wav, save_wav  # noqa: E402
from speechify.config import Config, data_dir, set_value  # noqa: E402
from speechify.labels import labelled_from_db, split  # noqa: E402
from speechify.metrics import wer  # noqa: E402

BASE_MODELS = {
    "whisper-large-v3-turbo": "openai/whisper-large-v3-turbo",
    "whisper-large-v3": "openai/whisper-large-v3",
    # Legacy config names.
    "large-v3-turbo": "openai/whisper-large-v3-turbo",
    "large-v3": "openai/whisper-large-v3",
}
VERSION_FILE = "speechify-version.json"  # students.VERSION_FILE
MAX_SECONDS = 30  # Whisper sees 30 s windows; longer clips would train on cut-off audio

LIBRISPEECH_URLS = [
    "https://www.openslr.org/resources/12/test-clean.tar.gz",
    "https://us.openslr.org/resources/12/test-clean.tar.gz",
    "https://openslr.trmal.net/resources/12/test-clean.tar.gz",
]
REGRESSION_CLIPS = 50


class GateError(Exception):
    pass


def gate1(current: float, candidate: float, held_out: int, source: str, base: float) -> dict:
    """Your audio: the new model must beat the model in use now, strictly."""
    return {"current": current, "base": base, "candidate": candidate, "held_out": held_out,
            "source": source, "passed": candidate < current}


def gate2(base: float, candidate: float, max_points: float, clips: int) -> dict:
    """LibriSpeech: at most `max_points` WER points worse than the base model."""
    allowed = max_points / 100
    return {"base": base, "candidate": candidate, "max_increase": allowed, "clips": clips,
            "passed": candidate <= base + allowed + 1e-9}


def load_samples(root: Path, min_confidence: float, corrected_only: bool):
    """(train, held_out, held_out_source), as (wav path, text) pairs."""
    found = []
    for label in labelled_from_db(root / "learning.db", min_confidence):
        if corrected_only and label.source != "corrected":
            continue
        path = root / "audio" / label.clip.audio
        if path.exists() and path.stat().st_size <= 44 + MAX_SECONDS * SAMPLE_RATE * 2:
            found.append(label)
    train, held_out, source = split(found)
    pairs = lambda labels: [(root / "audio" / x.clip.audio, x.text) for x in labels]  # noqa: E731
    return pairs(train), pairs(held_out), source


def regression_set(root: Path, log=print) -> list[tuple[Path, str]]:
    """50 LibriSpeech test-clean clips, cached as WAVs after the first run.

    Streams the start of the archive and stops once it has enough, so it
    reads ~10 MB of the 346 MB file.
    """
    folder = root / "regression" / "librispeech-test-clean"
    index = folder / "transcripts.tsv"
    if index.exists():
        rows = [line.split("\t", 1) for line in index.read_text(encoding="utf-8").splitlines() if line]
        if len(rows) >= REGRESSION_CLIPS:
            return [(folder / f"{uid}.wav", text) for uid, text in rows]
    import numpy as np
    import soundfile

    folder.mkdir(parents=True, exist_ok=True)
    last_error = None
    for url in LIBRISPEECH_URLS:
        try:
            log(f"Downloading {REGRESSION_CLIPS} LibriSpeech test-clean clips from {url} (once)...")
            texts: dict[str, str] = {}
            flacs: dict[str, bytes] = {}
            chosen: list[tuple[str, str]] = []
            with urllib.request.urlopen(url, timeout=60) as resp, tarfile.open(fileobj=resp, mode="r|gz") as tar:
                for member in tar:
                    if member.name.endswith(".trans.txt"):
                        for line in tar.extractfile(member).read().decode().splitlines():
                            uid, _, text = line.partition(" ")
                            texts[uid] = text.strip()
                    elif member.name.endswith(".flac"):
                        flacs[Path(member.name).stem] = tar.extractfile(member).read()
                    for uid in [u for u in flacs if u in texts]:
                        audio, rate = soundfile.read(io.BytesIO(flacs.pop(uid)), dtype="float32")
                        if rate == SAMPLE_RATE and audio.ndim == 1 and audio.size <= 20 * SAMPLE_RATE:
                            save_wav(np.asarray(audio), folder / f"{uid}.wav")
                            chosen.append((uid, texts[uid]))
                    if len(chosen) >= REGRESSION_CLIPS:
                        break
            if len(chosen) < REGRESSION_CLIPS:
                raise OSError(f"only found {len(chosen)} clips")
            chosen = chosen[:REGRESSION_CLIPS]
            index.write_text("".join(f"{u}\t{t}\n" for u, t in chosen), encoding="utf-8")
            return [(folder / f"{uid}.wav", text) for uid, text in chosen]
        except Exception as exc:  # try the next mirror
            last_error = exc
            log(f"  failed: {exc}")
    raise GateError(f"couldn't download the LibriSpeech regression set: {last_error}")


def default_current(cfg: Config, base: str) -> str:
    """The model Speechify uses now, as something transformers can load."""
    path = Path(cfg.model)
    if path.name.startswith("whisper-ft-"):
        hf = path.parent / ("hf-" + path.name.removeprefix("whisper-ft-"))
        if (hf / "config.json").exists():
            return str(hf)
    return BASE_MODELS.get(cfg.model, base)


def main() -> None:
    cfg = Config.load()
    default_base = BASE_MODELS.get(cfg.model, "openai/whisper-large-v3-turbo")
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--base", default=default_base, help="Hugging Face model to start from")
    p.add_argument("--base-id", help="Speechify's id for the base model (recorded with the new model)")
    p.add_argument("--current", help="the model to beat: a Hugging Face repo or a saved hf-* folder "
                                     "(default: the model Speechify uses now)")
    p.add_argument("--epochs", type=int, default=3)
    p.add_argument("--lr", type=float, default=3e-4)
    p.add_argument("--batch-size", type=int, default=4)
    p.add_argument("--rank", type=int, default=32, help="LoRA rank")
    p.add_argument("--min-samples", type=int, default=50)
    p.add_argument("--min-confidence", type=float, default=cfg.teacher_min_confidence,
                   help="teacher labels below this confidence aren't used")
    p.add_argument("--max-regression", type=float, default=cfg.max_regression_wer,
                   help="WER points the new model may lose on LibriSpeech against the base model")
    p.add_argument("--no-regression", action="store_true", help="skip the LibriSpeech gate")
    p.add_argument("--corrected-only", action="store_true", help="only train on recordings you corrected")
    p.add_argument("--force", action="store_true", help="save even if a gate fails")
    p.add_argument("--apply", action="store_true", help="update config.toml to use the new model")
    p.add_argument("--auto", action="store_true", help=argparse.SUPPRESS)  # started by Speechify itself
    p.add_argument("--report", type=Path, help="write the outcome here as JSON")
    args = p.parse_args()

    report: dict = {"status": "error", "started": time.time(), "base": args.base}

    def finish(status: str, message: str, code: int = 0) -> None:
        report.update(status=status, finished=time.time())
        if status == "error":
            report["error"] = message
        if args.report:
            args.report.write_text(json.dumps(report, indent=2), encoding="utf-8")
        print(message)
        sys.exit(code)

    try:
        train(args, cfg, report, finish)
    except SystemExit:
        raise
    except BaseException as exc:
        traceback.print_exc()
        finish("error", f"{type(exc).__name__}: {exc}", 1)


def train(args, cfg: Config, report: dict, finish) -> None:
    root = data_dir()
    train_set, eval_set, eval_source = load_samples(root, args.min_confidence, args.corrected_only)
    total = len(train_set) + len(eval_set)
    if total < args.min_samples:
        finish("error", f"Only {total} labelled recordings; need at least {args.min_samples}. Keep "
               "dictating: recordings you don't correct get labels from the teacher model while you're "
               "away, and the ones you correct are labelled already.", 1)
    if len(eval_set) < 5:
        finish("error", f"Only {len(eval_set)} recordings to hold out for evaluation; need at least 5.", 1)
    print(f"{len(train_set)} training recordings, {len(eval_set)} held out ({eval_source} labels)")
    report["train"] = {"recordings": len(train_set), "held_out": len(eval_set), "held_out_source": eval_source}

    import torch

    if not torch.cuda.is_available():
        finish("error", "Fine-tuning needs an NVIDIA GPU, and CUDA isn't available to PyTorch here. "
               "Run it on a PC with one and copy the model folder over; any OS can use it.", 1)

    from peft import LoraConfig, get_peft_model
    from transformers import (
        AutoTokenizer,
        WhisperForConditionalGeneration,
        WhisperProcessor,
        get_linear_schedule_with_warmup,
    )

    device = "cuda"
    dtype = torch.bfloat16 if torch.cuda.is_bf16_supported() else torch.float16
    language = cfg.language or "en"
    current = args.current or default_current(cfg, args.base)
    report["current"] = current

    processor = WhisperProcessor.from_pretrained(args.base, language=language, task="transcribe")

    def load(name: str):
        m = WhisperForConditionalGeneration.from_pretrained(name, torch_dtype=dtype).to(device)
        m.generation_config.forced_decoder_ids = None
        return m

    def features(batch: list[tuple[Path, str]]) -> torch.Tensor:
        audio = [load_wav(path) for path, _ in batch]
        feats = processor.feature_extractor(audio, sampling_rate=SAMPLE_RATE, return_tensors="pt")
        return feats.input_features.to(device, dtype)

    @torch.no_grad()
    def evaluate(m, clips: list[tuple[Path, str]]) -> float:
        m.eval()
        hyps = []
        for i in range(0, len(clips), args.batch_size):
            batch = clips[i : i + args.batch_size]
            out = m.generate(input_features=features(batch), language=language,
                             task="transcribe", max_new_tokens=220)
            hyps += processor.batch_decode(out, skip_special_tokens=True)
        return wer([t for _, t in clips], hyps)

    regression = []
    if not args.no_regression:
        try:
            regression = regression_set(root)
        except GateError as exc:
            if not args.force:
                finish("error", str(exc), 1)
            print(f"{exc}; continuing without the LibriSpeech gate (--force)")

    model = load(args.base)
    start_token = model.config.decoder_start_token_id
    base_wer = evaluate(model, eval_set)
    base_ls = evaluate(model, regression) if regression else None
    print(f"Base model WER on your held-out audio: {base_wer:.1%}"
          + (f", on LibriSpeech: {base_ls:.1%}" if base_ls is not None else ""))
    if Path(current).name == Path(args.base).name or current == args.base:
        current_wer = base_wer
    else:
        other = load(current)
        current_wer = evaluate(other, eval_set)
        del other
        torch.cuda.empty_cache()
    print(f"Current model ({current}) WER on your held-out audio: {current_wer:.1%}")

    def labels(batch: list[tuple[Path, str]]) -> torch.Tensor:
        ids = [processor.tokenizer(text).input_ids for _, text in batch]
        # The model prepends the start token itself when shifting labels right.
        ids = [x[1:] if x and x[0] == start_token else x for x in ids]
        width = max(len(x) for x in ids)
        return torch.tensor([x + [-100] * (width - len(x)) for x in ids], device=device)

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
        total_loss = 0.0
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
            total_loss += loss.item() * len(batch)
        print(f"epoch {epoch + 1}/{args.epochs}  loss {total_loss / len(train_set):.4f}  "
              f"({time.time() - t0:.0f}s)")

    tuned_wer = evaluate(model, eval_set)
    g1 = report["gate1"] = gate1(current_wer, tuned_wer, len(eval_set), eval_source, base_wer)
    print(f"Gate 1, your audio: {tuned_wer:.1%} vs {current_wer:.1%} for the current model "
          f"({'pass' if g1['passed'] else 'fail'})")
    passed = g1["passed"]
    if regression:
        g2 = report["gate2"] = gate2(base_ls, evaluate(model, regression), args.max_regression, len(regression))
        print(f"Gate 2, LibriSpeech: {g2['candidate']:.1%} vs {base_ls:.1%} for the base model, "
              f"{args.max_regression:g} points allowed ({'pass' if g2['passed'] else 'fail'})")
        passed = passed and g2["passed"]
    if not passed and not args.force:
        finish("rejected", "The new model didn't pass, so it wasn't saved.")

    stamp = time.strftime("%Y%m%d-%H%M%S")
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
    info = {"base_repo": args.base, "base_id": args.base_id or next(
                (k for k, v in BASE_MODELS.items() if v == args.base), args.base),
            "created": time.time(), "train": report["train"],
            "gate1": report.get("gate1"), "gate2": report.get("gate2")}
    for d in (hf_dir, out_dir):
        (d / VERSION_FILE).write_text(json.dumps(info, indent=2), encoding="utf-8")
    print(f"Saved faster-whisper model to {out_dir}")
    report.update(model=out_dir.as_posix(), hf=hf_dir.as_posix())

    if args.apply:
        set_value("model", out_dir.as_posix())
        print("Switched Speechify to it. Restart Speechify, or pick it with Ctrl+Alt+M.")
    elif not args.auto:
        print("It's listed in the model window (Ctrl+Alt+M), or rerun with --apply to switch to it.")
    finish("accepted", "Done.")


if __name__ == "__main__":
    main()
