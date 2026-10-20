"""Learning from your corrections.

When you fix a dictation, three things happen:

1. Word-level differences ("get hub" -> "GitHub") become replacement rules.
   Once you've made the same fix `autocorrect_after` times, it's applied to
   every future dictation automatically.
2. The corrected words go into a vocabulary that's passed to Whisper as its
   initial prompt, which makes it much more likely to hear them right the
   first time. This takes effect on the very next dictation.
3. The audio and corrected text are saved as a training pair for
   scripts/finetune.py, which LoRA fine-tunes Whisper on your voice.
"""

from __future__ import annotations

import difflib
import re
import sqlite3
import string
import threading
import time
import uuid
from dataclasses import dataclass
from pathlib import Path

import numpy as np

MAX_SPAN_WORDS = 4  # longer edits are rewrites, not mis-hearings
MAX_PROMPT_CHARS = 600  # Whisper's prompt window is ~224 tokens

_PUNCT = string.punctuation + "“”‘’…"

SCHEMA = """
CREATE TABLE IF NOT EXISTS samples (
    id INTEGER PRIMARY KEY,
    ts REAL NOT NULL,
    audio TEXT,
    raw TEXT NOT NULL,       -- what Whisper produced
    pasted TEXT NOT NULL,    -- what we typed, after rules / LLM cleanup
    corrected TEXT,          -- what you said it should have been
    excluded INTEGER NOT NULL DEFAULT 0  -- 1 = keep, but don't train on it
);
CREATE TABLE IF NOT EXISTS rules (
    wrong TEXT NOT NULL,     -- lowercase, punctuation-stripped
    right TEXT NOT NULL,
    count INTEGER NOT NULL,
    PRIMARY KEY (wrong, right)
);
CREATE TABLE IF NOT EXISTS vocab (
    term TEXT PRIMARY KEY,
    count INTEGER NOT NULL
);
"""


def _strip(token: str) -> str:
    return token.strip(_PUNCT)


def extract_edits(before: str, after: str) -> list[tuple[str, str]]:
    """Return (wrong, right) pairs for the words that changed.

    `wrong` is lowercased so rules match regardless of case; `right` keeps
    your capitalization. Pure insertions/deletions and long rewrites are
    ignored since they don't tell us anything about what Whisper misheard.
    """
    a = before.split()
    b = after.split()
    a_norm = [_strip(t).lower() for t in a]
    b_norm = [_strip(t).lower() for t in b]
    edits: list[tuple[str, str]] = []
    matcher = difflib.SequenceMatcher(None, a_norm, b_norm, autojunk=False)
    for op, i1, i2, j1, j2 in matcher.get_opcodes():
        if op == "equal":
            # Same words, but the casing may have been fixed: "github" -> "GitHub".
            for ta, tb in zip(a[i1:i2], b[j1:j2]):
                sa, sb = _strip(ta), _strip(tb)
                if sa and sa != sb:
                    edits.append((sa.lower(), sb))
        elif op == "replace":
            if i2 - i1 > MAX_SPAN_WORDS or j2 - j1 > MAX_SPAN_WORDS:
                continue
            wrong = " ".join(t for t in a_norm[i1:i2] if t)
            right = " ".join(s for s in (_strip(t) for t in b[j1:j2]) if s)
            if wrong and right and wrong != right:
                edits.append((wrong, right))
    return edits


def build_replacer(rules: dict[str, str]):
    """Compile rules into a single-pass replacer.

    One combined regex (longest phrases first) means a replacement can never
    be re-matched by another rule, so rules can't chain into each other.
    """
    if not rules:
        return lambda text: text
    keys = sorted(rules, key=len, reverse=True)
    alternation = "|".join(r"\s+".join(map(re.escape, k.split())) for k in keys)
    pattern = re.compile(rf"(?<![\w'])(?:{alternation})(?![\w'])", re.IGNORECASE)

    def replace(match: re.Match) -> str:
        found = match.group(0)
        right = rules[" ".join(found.lower().split())]
        # Keep sentence-initial capitals: "get hub is" -> "GitHub is", but
        # also "There" -> "Their" when the rule is "there" -> "their".
        if found[:1].isupper() and right[:1].islower():
            right = right[:1].upper() + right[1:]
        return right

    return lambda text: pattern.sub(replace, text)


@dataclass
class Dictation:
    audio: np.ndarray
    raw: str
    pasted: str
    sample_id: int | None = None


@dataclass
class Sample:
    id: int
    ts: float
    audio: str | None
    raw: str
    pasted: str
    corrected: str | None
    excluded: int
    audio_dir: Path

    @property
    def text(self) -> str:
        """The text used for training."""
        return self.corrected if self.corrected is not None else self.pasted

    @property
    def path(self) -> Path | None:
        return self.audio_dir / self.audio if self.audio else None


class Learner:
    def __init__(self, data_dir: Path, autocorrect_after: int = 2):
        self.audio_dir = data_dir / "audio"
        self.audio_dir.mkdir(parents=True, exist_ok=True)
        self.autocorrect_after = autocorrect_after
        self._lock = threading.Lock()
        self._db = sqlite3.connect(data_dir / "learning.db", check_same_thread=False)
        self._db.executescript(SCHEMA)
        columns = {row[1] for row in self._db.execute("PRAGMA table_info(samples)")}
        if "excluded" not in columns:  # databases created before the history window
            self._db.execute("ALTER TABLE samples ADD COLUMN excluded INTEGER NOT NULL DEFAULT 0")
        self._reload()

    def _reload(self) -> None:
        rows = self._db.execute(
            "SELECT wrong, right, count FROM rules WHERE count >= ? ORDER BY count ASC",
            (self.autocorrect_after,),
        ).fetchall()
        # Ascending order so the most frequent `right` for a given `wrong` wins.
        self._replace = build_replacer({wrong: right for wrong, right, _ in rows})
        terms = [t for (t,) in self._db.execute("SELECT term FROM vocab ORDER BY count DESC, term")]
        prompt, used = "", []
        for term in terms:
            candidate = ", ".join(used + [term])
            if len(candidate) > MAX_PROMPT_CHARS:
                break
            used.append(term)
            prompt = candidate
        self._prompt = f"Glossary: {prompt}." if prompt else ""

    # --- used on every dictation -------------------------------------------

    def apply(self, text: str) -> str:
        return self._replace(text)

    def prompt(self) -> str:
        return self._prompt

    def record(self, d: Dictation, keep_audio: bool) -> None:
        if not keep_audio:
            return
        with self._lock:
            d.sample_id = self._insert(d, corrected=None)

    # --- corrections --------------------------------------------------------

    def correct(self, d: Dictation, corrected: str) -> list[tuple[str, str]]:
        """Save a correction and learn from it. Returns the edits learned."""
        corrected = corrected.strip()
        if not corrected or corrected == d.pasted:
            return []
        with self._lock:
            if d.sample_id is None:
                d.sample_id = self._insert(d, corrected=corrected)
            else:
                self._db.execute(
                    "UPDATE samples SET corrected = ? WHERE id = ?", (corrected, d.sample_id)
                )
            return self._learn(d.pasted, corrected)

    def _learn(self, before: str, after: str) -> list[tuple[str, str]]:
        edits = extract_edits(before, after)
        for wrong, right in edits:
            self._db.execute(
                "INSERT INTO rules VALUES (?, ?, 1) "
                "ON CONFLICT (wrong, right) DO UPDATE SET count = count + 1",
                (wrong, right),
            )
            self._db.execute(
                "INSERT INTO vocab VALUES (?, 1) "
                "ON CONFLICT (term) DO UPDATE SET count = count + 1",
                (right,),
            )
        self._db.commit()
        self._reload()
        return edits

    def _insert(self, d: Dictation, corrected: str | None) -> int:
        from .audio import save_wav

        name = f"{time.strftime('%Y%m%d-%H%M%S')}-{uuid.uuid4().hex[:6]}.wav"
        save_wav(d.audio, self.audio_dir / name)
        cur = self._db.execute(
            "INSERT INTO samples (ts, audio, raw, pasted, corrected) VALUES (?, ?, ?, ?, ?)",
            (time.time(), name, d.raw, d.pasted, corrected),
        )
        self._db.commit()
        return cur.lastrowid

    # --- history (every saved recording) -----------------------------------

    def samples(self, search: str = "", limit: int = 500) -> list[Sample]:
        rows = self._db.execute(
            "SELECT id, ts, audio, raw, pasted, corrected, excluded FROM samples "
            "WHERE coalesce(corrected, pasted) LIKE ? ORDER BY ts DESC LIMIT ?",
            (f"%{search}%", limit),
        ).fetchall()
        return [Sample(*row, audio_dir=self.audio_dir) for row in rows]

    def edit_sample(self, sample_id: int, text: str) -> list[tuple[str, str]]:
        """Set the training text for a recording, learning from what changed."""
        text = text.strip()
        with self._lock:
            row = self._db.execute(
                "SELECT coalesce(corrected, pasted), pasted FROM samples WHERE id = ?", (sample_id,)
            ).fetchone()
            if row is None or not text or text == row[0]:
                return []
            # Setting it back to exactly what was pasted means "this was right".
            self._db.execute(
                "UPDATE samples SET corrected = ? WHERE id = ?",
                (None if text == row[1] else text, sample_id),
            )
            return self._learn(row[0], text)

    def set_excluded(self, sample_id: int, excluded: bool) -> None:
        with self._lock:
            self._db.execute("UPDATE samples SET excluded = ? WHERE id = ?", (int(excluded), sample_id))
            self._db.commit()

    def delete_sample(self, sample_id: int) -> None:
        with self._lock:
            row = self._db.execute("SELECT audio FROM samples WHERE id = ?", (sample_id,)).fetchone()
            if row is None:
                return
            if row[0]:
                (self.audio_dir / row[0]).unlink(missing_ok=True)
            self._db.execute("DELETE FROM samples WHERE id = ?", (sample_id,))
            self._db.commit()

    def export(self, out_dir: Path) -> int:
        """Write a Hugging Face "audiofolder" dataset: wavs + metadata.csv."""
        import csv
        import shutil

        out_dir.mkdir(parents=True, exist_ok=True)
        rows = self._db.execute(
            "SELECT audio, coalesce(corrected, pasted) FROM samples "
            "WHERE audio IS NOT NULL AND excluded = 0 ORDER BY ts"
        ).fetchall()
        n = 0
        with open(out_dir / "metadata.csv", "w", newline="", encoding="utf-8") as f:
            writer = csv.writer(f)
            writer.writerow(["file_name", "transcription"])
            for name, text in rows:
                src = self.audio_dir / name
                if src.exists():
                    shutil.copy2(src, out_dir / name)
                    writer.writerow([name, text])
                    n += 1
        return n

    # --- management (CLI) ---------------------------------------------------

    def rules(self) -> list[tuple[str, str, int]]:
        return self._db.execute(
            "SELECT wrong, right, count FROM rules ORDER BY count DESC, wrong"
        ).fetchall()

    def vocab(self) -> list[tuple[str, int]]:
        return self._db.execute("SELECT term, count FROM vocab ORDER BY count DESC, term").fetchall()

    def forget(self, phrase: str) -> int:
        """Delete every rule and vocab term matching `phrase`."""
        with self._lock:
            n = self._db.execute(
                "DELETE FROM rules WHERE wrong = ? OR lower(right) = ?",
                (phrase.lower(), phrase.lower()),
            ).rowcount
            n += self._db.execute("DELETE FROM vocab WHERE lower(term) = ?", (phrase.lower(),)).rowcount
            self._db.commit()
            self._reload()
        return n

    def stats(self) -> dict[str, int]:
        q = self._db.execute
        return {
            "samples": q("SELECT count(*) FROM samples").fetchone()[0],
            "corrected": q("SELECT count(*) FROM samples WHERE corrected IS NOT NULL").fetchone()[0],
            "excluded": q("SELECT count(*) FROM samples WHERE excluded = 1").fetchone()[0],
            "rules": q("SELECT count(*) FROM rules").fetchone()[0],
            "active_rules": q(
                "SELECT count(*) FROM rules WHERE count >= ?", (self.autocorrect_after,)
            ).fetchone()[0],
            "vocab": q("SELECT count(*) FROM vocab").fetchone()[0],
        }
