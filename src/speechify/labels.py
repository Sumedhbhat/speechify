"""Training labels: which text each saved recording is trained on.

Imported by the app and by scripts/finetune.py (in its own environment), so
it only needs the standard library and numpy.

Each clip's label, in order of preference:

1. your own correction;
2. otherwise the teacher's transcript, if it passed the hallucination checks
   and its confidence is at least `min_confidence`;
3. otherwise nothing: the clip is skipped.

What the student pasted is never used as a label, so it can't learn from its
own mistakes. Clips where the teacher and the student disagree are kept like
any other: they're the ones that teach it something.
"""

from __future__ import annotations

import hashlib
import math
import re
import sqlite3
import zlib
from dataclasses import dataclass
from pathlib import Path

import numpy as np

SAMPLE_RATE = 16_000

# Corrected clips become the held-out set once there are this many.
MIN_CORRECTED_HELDOUT = 20

# Hallucination limits.
MAX_COMPRESSION_RATIO = 2.4  # Whisper's own threshold for repetitive output
MAX_WORDS_PER_SECOND = 7.0   # fast speech is ~4-5 words per second of voice
MIN_WORDS_PER_SECOND = 0.3   # checked once there are 3+ seconds of voice

# Without token probabilities, a clean-looking transcript scores this much,
# never more, so the heuristic can't outrank a real probability.
HEURISTIC_CEILING = 0.8

_WORD = re.compile(r"[^\W_]+(?:['’][^\W_]+)*")


def words(text: str) -> list[str]:
    return _WORD.findall(text.lower())


# --- voiced duration --------------------------------------------------------


def voiced_seconds(audio: np.ndarray) -> float:
    """Seconds of audio that look like speech, from frame energy.

    Rough on purpose: it only has to tell a 2-second clip from a 20-second
    one so the transcript length can be sanity-checked.
    """
    frame = 480  # 30 ms
    n = audio.size // frame
    if n == 0:
        return 0.0
    rms = np.sqrt(np.mean(audio[: n * frame].reshape(n, frame).astype(np.float64) ** 2, axis=1))
    # Three times the noise floor, but never above half the median: in a
    # clip that's speech from end to end the "floor" is speech too.
    floor = float(np.percentile(rms, 10))
    threshold = max(0.004, min(floor * 3, float(np.median(rms)) * 0.5))
    voiced = rms > threshold
    return float(voiced.sum()) * frame / SAMPLE_RATE


# --- hallucination checks ---------------------------------------------------


def _longest_repeat(tokens: list[str], max_n: int = 6) -> tuple[int, int]:
    """(n, times): the n-gram repeated back to back the most times."""
    best = (1, 1)
    for n in range(1, max_n + 1):
        for start in range(len(tokens) - 2 * n + 1):
            gram = tokens[start : start + n]
            times = 1
            i = start + n
            while tokens[i : i + n] == gram:
                times += 1
                i += n
            if times > best[1]:
                best = (n, times)
    return best


def compression_ratio(text: str) -> float:
    data = text.encode("utf-8")
    return len(data) / len(zlib.compress(data)) if data else 0.0


def hallucination(text: str, voiced_s: float) -> str | None:
    """Why `text` looks like a hallucination for a clip with `voiced_s`
    seconds of speech, or None if it looks fine."""
    tokens = words(text)
    if not tokens:
        return "empty"
    n, times = _longest_repeat(tokens)
    if times >= (5 if n == 1 else 3):
        return "repeated phrase"
    if len(text) >= 60 and compression_ratio(text) > MAX_COMPRESSION_RATIO:
        return "repeated phrase"
    if len(tokens) > MAX_WORDS_PER_SECOND * voiced_s + 3:
        return "too long for the clip"
    if voiced_s >= 3 and len(tokens) < MIN_WORDS_PER_SECOND * voiced_s:
        return "too short for the clip"
    return None


# --- confidence ---------------------------------------------------------------


def confidence_from_logprob(mean_logprob: float) -> float:
    """exp(mean token log-probability): the geometric mean of the teacher's
    per-token probabilities, between 0 and 1."""
    return float(min(1.0, max(0.0, math.exp(mean_logprob))))


def heuristic_confidence(text: str, voiced_s: float) -> float:
    """For teachers that don't expose token probabilities.

    Starts at HEURISTIC_CEILING and loses points for things that are legal
    but suspicious: a speaking rate outside 1-4.5 words per second of voice,
    repetitive text (compression ratio above 1.8), and lots of non-word
    characters. Agreement with the student is deliberately not a factor.
    """
    tokens = words(text)
    if not tokens:
        return 0.0
    score = HEURISTIC_CEILING
    if voiced_s >= 1:
        rate = len(tokens) / voiced_s
        if rate > 4.5:
            score -= min(0.4, (rate - 4.5) * 0.15)
        elif rate < 1.0:
            score -= min(0.4, (1.0 - rate) * 0.5)
    ratio = compression_ratio(text) if len(text) >= 60 else 0.0
    if ratio > 1.8:
        score -= min(0.4, (ratio - 1.8) / 0.6 * 0.4)
    letters = sum(len(t) for t in tokens)
    other = len(re.sub(r"\s", "", text)) - letters
    if letters and other / (letters + other) > 0.3:
        score -= 0.2
    return round(max(0.0, score), 3)


@dataclass(frozen=True)
class TeacherResult:
    text: str | None        # None when rejected
    confidence: float
    note: str | None        # why it was rejected


def judge(text: str, audio: np.ndarray, mean_logprob: float | None) -> TeacherResult:
    """Check a teacher transcript and give it a confidence score."""
    text = " ".join(text.split())
    voiced = voiced_seconds(audio)
    why = hallucination(text, voiced)
    if why:
        return TeacherResult(None, 0.0, why)
    if mean_logprob is not None and math.isfinite(mean_logprob):
        return TeacherResult(text, round(confidence_from_logprob(mean_logprob), 3), None)
    return TeacherResult(text, heuristic_confidence(text, voiced), None)


# --- label selection ------------------------------------------------------------


@dataclass(frozen=True)
class Clip:
    id: int
    audio: str | None
    corrected: str | None
    excluded: int
    teacher_text: str | None
    teacher_confidence: float | None
    teacher_ts: float | None
    teacher_note: str | None
    teacher_model: str | None = None


@dataclass(frozen=True)
class Label:
    clip: Clip
    text: str
    source: str  # "corrected" or "teacher"


def classify(clip: Clip, min_confidence: float) -> tuple[str | None, str]:
    """(label text or None, why): why is "corrected", "teacher", "excluded",
    "no audio", "pending", "rejected" or "low confidence"."""
    if clip.excluded:
        return None, "excluded"
    if not clip.audio:
        return None, "no audio"
    if clip.corrected is not None and clip.corrected.strip():
        return clip.corrected.strip(), "corrected"
    if clip.teacher_ts is None:
        return None, "pending"
    if clip.teacher_note or not clip.teacher_text:
        return None, "rejected"
    if (clip.teacher_confidence or 0.0) < min_confidence:
        return None, "low confidence"
    return clip.teacher_text.strip(), "teacher"


CLIP_COLUMNS = (
    "id, audio, corrected, excluded, teacher_text, teacher_confidence, teacher_ts, teacher_note, teacher_model"
)


def load_clips(db: sqlite3.Connection) -> list[Clip]:
    return [Clip(*row) for row in db.execute(f"SELECT {CLIP_COLUMNS} FROM samples ORDER BY ts")]


def labels(clips: list[Clip], min_confidence: float) -> list[Label]:
    out = []
    for clip in clips:
        text, source = classify(clip, min_confidence)
        if text:
            out.append(Label(clip, text, source))
    return out


def _fraction(label: Label) -> float:
    """A fixed number in [0, 1) per recording, from its file name (not its
    row id, so it doesn't depend on insert order)."""
    digest = hashlib.sha1((label.clip.audio or str(label.clip.id)).encode()).hexdigest()
    return int(digest[:8], 16) / 2**32


def split(all_labels: list[Label]) -> tuple[list[Label], list[Label], str]:
    """(train, held_out, held_out_source).

    Once you've corrected at least MIN_CORRECTED_HELDOUT clips, the held-out
    set is drawn from them: every one whose hash falls in the lowest fifth,
    topped up to 20 with the next lowest. Until then it's drawn from
    confident teacher labels the same way: a tenth, topped up to 20, at most
    half of them.

    Because membership follows a fixed hash, a recording that was trained on
    never moves into the held-out set as more arrive (in the corrected pool),
    so an earlier model isn't scored on clips it saw. Within one run the two
    sets never overlap.
    """
    corrected = [x for x in all_labels if x.source == "corrected"]
    if len(corrected) >= MIN_CORRECTED_HELDOUT:
        pool, source, fraction, cap = corrected, "corrected", 0.2, len(corrected)
    else:
        pool, source, fraction = [x for x in all_labels if x.source == "teacher"], "teacher", 0.1
        cap = len(pool) // 2
    ranked = sorted(pool, key=_fraction)
    below = sum(1 for x in ranked if _fraction(x) < fraction)
    n = min(cap, max(below, MIN_CORRECTED_HELDOUT))
    held = set(id(x) for x in ranked[:n])
    train = [x for x in all_labels if id(x) not in held]
    held_out = [x for x in all_labels if id(x) in held]
    return train, held_out, source


def labelled_from_db(db_path: Path, min_confidence: float) -> list[Label]:
    db = sqlite3.connect(db_path)
    try:
        return labels(load_clips(db), min_confidence)
    finally:
        db.close()
