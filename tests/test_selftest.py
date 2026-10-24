import json

from speechify import selftest
from speechify.audio import load_wav
from speechify.metrics import wer


def test_sample_clip_is_packaged():
    audio = load_wav(selftest.SAMPLE)
    assert 9 < audio.size / 16_000 < 12


def test_reference_matches_a_good_transcript():
    heard = ("He hoped there would be stew for dinner, turnips and carrots and bruised potatoes "
             "and fat mutton pieces to be ladled out in thick peppered flour fattened sauce.")
    assert wer([selftest.REFERENCE], [heard]) <= selftest.MAX_WER


def test_report_records_failures(tmp_path):
    report = selftest.Report(tmp_path)
    with report.step("works"):
        pass
    with report.step("breaks") as s:
        s["detail"] = "context"
        raise AssertionError("nope")
    report.write()
    data = json.loads((tmp_path / "report.json").read_text())
    assert data["ok"] is False
    assert [s["ok"] for s in data["steps"]] == [True, False]
    assert "context | AssertionError: nope" in data["steps"][1]["detail"]
    assert "❌" in (tmp_path / "summary.md").read_text(encoding="utf-8")


def test_formatting_check_fails_lowercase_unless_the_model_never_promised(tmp_path):
    import pytest

    from speechify.engines.catalog import MODELS

    report = selftest.Report(tmp_path)
    refs = [selftest.REFERENCE, selftest.FORMAT_REFERENCE]
    lowercase = [r.lower().replace(",", "").replace(".", "") for r in refs]
    assert "capitals 100%" in selftest.check_formatting(MODELS["whisper-large-v3-turbo"], refs, refs, report)
    with pytest.raises(AssertionError, match="needs capitals"):
        selftest.check_formatting(MODELS["whisper-large-v3-turbo"], refs, lowercase, report)
    detail = selftest.check_formatting(MODELS["distil-whisper-large-v3"], refs, lowercase, report)
    assert "not checked" in detail
