"""Word error rate, shared by the self-test and the fine-tune script."""

from __future__ import annotations

import string

_DROP = str.maketrans("", "", string.punctuation)


def words(text: str) -> list[str]:
    return text.lower().replace("-", " ").translate(_DROP).split()


def wer(refs: list[str], hyps: list[str]) -> float:
    """Word-level edit distance over all pairs, divided by the reference word count."""
    errors = total = 0
    for ref, hyp in zip(refs, hyps):
        r, h = words(ref), words(hyp)
        row = list(range(len(h) + 1))
        for i, rw in enumerate(r, 1):
            prev, row[0] = row[0], i
            for j, hw in enumerate(h, 1):
                prev, row[j] = row[j], min(row[j] + 1, row[j - 1] + 1, prev + (rw != hw))
        errors += row[-1]
        total += len(r)
    return errors / max(total, 1)
