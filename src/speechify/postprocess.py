"""Optional LLM cleanup through an OpenAI-compatible chat endpoint."""

from __future__ import annotations

import json
import urllib.request

SYSTEM = (
    "You fix speech-to-text output. Correct misheard words, punctuation and "
    "capitalization. Do not rephrase, summarize, answer questions, or add "
    "anything. Reply with the corrected text only."
)


def clean(text: str, url: str, model: str, glossary: str, timeout: float = 3.0) -> str:
    """Return the cleaned text, or the input unchanged if the LLM fails."""
    system = SYSTEM + (f"\n\nThe speaker often uses these terms. {glossary}" if glossary else "")
    body = json.dumps({
        "model": model,
        "temperature": 0,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": text},
        ],
    }).encode()
    req = urllib.request.Request(url, body, {"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            out = json.load(resp)["choices"][0]["message"]["content"].strip()
    except Exception as exc:
        print(f"LLM cleanup skipped: {exc}")
        return text
    # Small models sometimes go off and answer the text instead of fixing it.
    if not out or len(out) > 2 * len(text) + 20:
        return text
    return out
