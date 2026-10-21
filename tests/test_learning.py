import numpy as np

from speechify.learning import Dictation, Learner, build_replacer, extract_edits


def test_extracts_misheard_phrase():
    assert extract_edits("push it to get hub please", "push it to GitHub please") == [
        ("get hub", "GitHub")
    ]


def test_extracts_case_only_fix():
    assert extract_edits("open the github repo.", "open the GitHub repo.") == [("github", "GitHub")]


def test_ignores_punctuation_and_long_rewrites():
    assert extract_edits("hello world", "Hello, world.") == [("hello", "Hello")]
    assert extract_edits("a b c d e f", "one two three four five six") == []


def test_replacer_matches_whole_words_and_keeps_sentence_case():
    replace = build_replacer({"get hub": "GitHub", "cube control": "kubectl"})
    assert replace("Get hub and get  hub.") == "GitHub and GitHub."
    assert replace("forget hub") == "forget hub"
    assert replace("Cube control apply") == "Kubectl apply"


def test_replacer_does_not_chain():
    replace = build_replacer({"a": "b", "b": "c"})
    assert replace("a b") == "b c"


def test_learner_activates_rule_after_threshold(tmp_path):
    learner = Learner(tmp_path, autocorrect_after=2)
    audio = np.zeros(1600, dtype=np.float32)

    learner.correct(Dictation(audio, "use cube control", "use cube control"), "use kubectl")
    assert learner.apply("cube control get pods") == "cube control get pods"
    assert "kubectl" in learner.prompt()  # vocabulary hint applies immediately

    learner.correct(Dictation(audio, "cube control", "cube control"), "kubectl")
    assert learner.apply("cube control get pods") == "kubectl get pods"

    stats = learner.stats()
    assert stats["corrected"] == 2 and stats["active_rules"] == 1
    assert len(list((tmp_path / "audio").glob("*.wav"))) == 2


def test_most_frequent_fix_wins(tmp_path):
    learner = Learner(tmp_path, autocorrect_after=1)
    audio = np.zeros(1600, dtype=np.float32)
    for right in ["their", "their", "they're"]:
        learner.correct(Dictation(audio, "there", "there"), right)
    assert learner.apply("there") == "their"


def test_second_fix_updates_same_sample(tmp_path):
    learner = Learner(tmp_path)
    d = Dictation(np.zeros(1600, dtype=np.float32), "hello word", "hello word")
    learner.record(d, keep_audio=True)
    learner.correct(d, "hello world")
    assert learner.stats()["samples"] == 1 and learner.stats()["corrected"] == 1


def test_forget(tmp_path):
    learner = Learner(tmp_path, autocorrect_after=1)
    learner.correct(Dictation(np.zeros(160, dtype=np.float32), "get hub", "get hub"), "GitHub")
    assert learner.apply("get hub") == "GitHub"
    assert learner.forget("GitHub") == 2
    assert learner.apply("get hub") == "get hub" and learner.prompt() == ""
