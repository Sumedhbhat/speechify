import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "finetune", Path(__file__).parents[1] / "scripts" / "finetune.py"
)
finetune = importlib.util.module_from_spec(spec)
spec.loader.exec_module(finetune)


def test_wer():
    assert finetune.wer(["the cat sat"], ["the cat sat"]) == 0
    assert finetune.wer(["the cat sat"], ["The cat, sat."]) == 0
    assert finetune.wer(["the cat sat"], ["the bat"]) == 2 / 3
    assert finetune.wer(["a b", "c d"], ["a b c", ""]) == 3 / 4
