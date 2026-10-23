import numpy as np

from speechify import selftest
from speechify.engines.host import WARMUP_WAV
from speechify.engines.worker import warmup_audio
from speechify.metrics import formatting, wer

QUILTER = "Nor is Mr. Quilter's manner less interesting than his matter."


def test_wer_ignores_case_and_punctuation():
    assert wer([QUILTER], ["nor is mr quilter's manner less interesting than his matter"]) == 0
    assert wer(["a b c d"], ["a x c"]) == 0.5


def test_formatting_is_perfect_for_the_reference():
    score = formatting([selftest.REFERENCE, QUILTER], [selftest.REFERENCE, QUILTER])
    assert (score.case, score.punct) == (1, 1)


def test_formatting_scores_lowercase_unpunctuated_text_zero():
    # Whisper turbo's output in the 2026-10-22 benchmark: WER can't see the difference.
    score = formatting([QUILTER], ["nor is mr quilter's manner less interesting than his matter"])
    assert (score.case, score.punct) == (0, 0)


def test_formatting_mr_is_not_a_sentence_end():
    assert formatting(["Ask Mr. Smith."], ["Ask Mr Smith."]).punct == 1
    assert formatting(["Ask Mr. Smith."], ["Ask Mr. Smith"]).punct == 0


def test_formatting_only_compares_words_the_transcript_got_right():
    # "Mister" for "Mr." and the extra word are WER's business, not formatting's.
    score = formatting([QUILTER], ["Nor is Mister Quilter's manner really less interesting than his matter."])
    assert (score.case, score.punct) == (1, 1)


def test_formatting_counts_a_missing_comma_and_a_wrong_mark():
    score = formatting(["Hello, world. Bye."], ["Hello world, bye."])
    # Capitals: Hello right, Bye missed. Marks: "." right; "," missed; "," for "." wrong.
    assert round(score.case, 2) == 0.67
    assert round(score.punct, 2) == 0.4


def test_selftest_thresholds_pass_a_good_transcript_and_fail_lowercase():
    refs = [selftest.REFERENCE, selftest.FORMAT_REFERENCE]
    good = ["He hoped there would be stew for dinner, turnips and carrots and bruised potatoes "
            "and fat mutton pieces to be ladled out in thick peppered flour fattened sauce.",
            "Nor is Mr. Quilter's manner less interesting than his matter."]
    score = formatting(refs, good)
    assert score.case >= selftest.MIN_CASE and score.punct >= selftest.MIN_PUNCT
    score = formatting(refs, [t.lower().replace(",", "").replace(".", "") for t in good])
    assert score.case < selftest.MIN_CASE and score.punct < selftest.MIN_PUNCT


def test_worker_warms_up_on_speech():
    audio = warmup_audio(str(WARMUP_WAV))
    assert audio.dtype == np.float32 and audio.size == 3 * 16_000
    assert np.sqrt((audio ** 2).mean()) > 0.02  # not silence, which the VAD filter would drop
    assert not warmup_audio(None).any()  # missing clip: silence, as before


def test_worker_tidies_only_what_is_safe():
    from speechify.engines.worker import tidy

    assert tidy("mister Quilter is the apostle.") == "Mister Quilter is the apostle."
    assert tidy("he said it again") == "He said it again."
    assert tidy("Already fine.") == "Already fine."
    assert tidy("iPhone sales rose") == "iPhone sales rose."
    assert tidy("macOS works fine?") == "macOS works fine?"
    assert tidy('he said "stop"') == 'He said "stop"'  # ends in a quote: left alone
    assert tidy("42 is the answer") == "42 is the answer."
    # Fragments stay as they are: a command, a short phrase, nothing.
    assert tidy("kubectl") == "kubectl"
    assert tidy("next line") == "next line"
    assert tidy("") == ""
