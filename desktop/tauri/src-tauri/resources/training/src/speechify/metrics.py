"""Word error rate and formatting scores, shared by the self-test, `speechify
bench` and the fine-tune script.

WER ignores case and punctuation on purpose, so it can't tell "nor is mr
quilter's manner" from "Nor is Mr. Quilter's manner." formatting() scores
that part: capital letters and punctuation, against a punctuated reference.
"""

from __future__ import annotations

import re
import string
from dataclasses import dataclass

_DROP = str.maketrans("", "", string.punctuation)


# Spellings of the same word: LibriSpeech writes MISTER, Whisper "Mr.",
# Parakeet "mister".
_SAME = {"mister": "mr", "missus": "mrs"}


def words(text: str) -> list[str]:
    return [_SAME.get(w, w) for w in text.lower().replace("-", " ").translate(_DROP).split()]


def _edit_table(r: list[str], h: list[str]) -> list[list[int]]:
    table = [list(range(len(h) + 1))]
    for i, rw in enumerate(r, 1):
        row = [i]
        for j, hw in enumerate(h, 1):
            row.append(min(table[i - 1][j] + 1, row[j - 1] + 1, table[i - 1][j - 1] + (rw != hw)))
        table.append(row)
    return table


def wer(refs: list[str], hyps: list[str]) -> float:
    """Word-level edit distance over all pairs, divided by the reference word count."""
    errors = total = 0
    for ref, hyp in zip(refs, hyps):
        r, h = words(ref), words(hyp)
        errors += _edit_table(r, h)[-1][-1]
        total += len(r)
    return errors / max(total, 1)


# --- formatting ------------------------------------------------------------------

# A full stop after these is part of the word, not the end of a sentence.
_ABBREVIATIONS = {"mr", "mrs", "ms", "dr", "st", "jr", "sr", "vs", "etc", "prof"}
_END, _PAUSE = set(".?!"), set(",;:")


@dataclass(frozen=True)
class _Token:
    word: str  # lowercased, without punctuation, as words() has it
    capital: bool  # starts with a capital letter
    mark: str  # punctuation after it: "" (none), "," (pause) or "." (sentence end)


def _tokens(text: str) -> list[_Token]:
    out: list[_Token] = []
    for piece in re.split(r"[\s\-–—]+", text):
        word = piece.lower().translate(_DROP)
        tail = piece[len(piece.rstrip(string.punctuation + "”’»")) :]
        mark = "." if _END & set(tail) else "," if _PAUSE & set(tail) else ""
        if mark == "." and word in _ABBREVIATIONS and tail.startswith("."):
            mark = "." if _END & set(tail[1:]) else ""
        if not word:  # a dash or quote on its own: its mark belongs to the word before
            if out and mark and not out[-1].mark:
                out[-1] = _Token(out[-1].word, out[-1].capital, mark)
            continue
        letters = [c for c in piece if c.isalpha()]
        out.append(_Token(word, bool(letters) and letters[0].isupper(), mark))
    return out


def _aligned(ref: list[_Token], hyp: list[_Token]) -> list[tuple[_Token, _Token]]:
    """Pairs of reference and transcript tokens for the same spoken word."""
    r, h = [t.word for t in ref], [t.word for t in hyp]
    table = _edit_table(r, h)
    pairs, i, j = [], len(r), len(h)
    while i and j:
        if r[i - 1] == h[j - 1] and table[i][j] == table[i - 1][j - 1]:
            pairs.append((ref[i - 1], hyp[j - 1]))
            i, j = i - 1, j - 1
        elif table[i][j] == table[i - 1][j - 1] + 1:
            i, j = (
                i - 1,
                j - 1,
            )  # a different word: neither its case nor its punctuation is comparable
        elif table[i][j] == table[i - 1][j] + 1:
            i -= 1
        else:
            j -= 1
    return pairs[::-1]


def _f1(tp: int, fp: int, fn: int) -> float:
    return 1.0 if tp + fp + fn == 0 else 2 * tp / (2 * tp + fp + fn)


@dataclass(frozen=True)
class Formatting:
    """F1 scores from 0 to 1; 1 is perfect. Only words the transcript got
    right are compared, so misheard words count against WER, not these.

    case:  which words start with a capital letter.
    punct: where commas and sentence ends go ("," ";" ":" are one kind,
           "." "?" "!" the other). An "Mr." doesn't count as a full stop.
    """

    case: float
    punct: float

    def __str__(self) -> str:
        return f"capitals {self.case:.0%}, punctuation {self.punct:.0%}"


def formatting(refs: list[str], hyps: list[str]) -> Formatting:
    """Capitalisation and punctuation of `hyps`, against punctuated `refs`.

    F1 rather than plain accuracy: most words have no punctuation after
    them, so an all-lowercase, unpunctuated transcript would still score
    ~90% "accurate". Here it scores 0.
    """
    case, punct = [0, 0, 0], [0, 0, 0]  # true positives, false positives, false negatives
    for ref, hyp in zip(refs, hyps):
        for r, h in _aligned(_tokens(ref), _tokens(hyp)):
            if r.capital and h.capital:
                case[0] += 1
            elif h.capital:
                case[1] += 1
            elif r.capital:
                case[2] += 1
            if r.mark and r.mark == h.mark:
                punct[0] += 1
            else:
                punct[1] += bool(h.mark)
                punct[2] += bool(r.mark)
    return Formatting(_f1(*case), _f1(*punct))
