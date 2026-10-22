import math
import sqlite3

import numpy as np
import pytest

from speechify import labels
from speechify.labels import Clip, classify, hallucination, judge, split
from speechify.learning import TEACHER_COLUMNS, Dictation, Learner


def speech(seconds: float) -> np.ndarray:
    """Loud noise: every frame counts as voiced."""
    return np.random.default_rng(0).uniform(-0.3, 0.3, int(seconds * 16_000)).astype(np.float32)


def clip(i=1, corrected=None, excluded=0, teacher="the teacher text", conf=0.9, ts=1.0, note=None,
         audio="a.wav"):
    return Clip(i, audio, corrected, excluded, teacher, conf, ts, note)


# --- label selection ---------------------------------------------------------


def test_correction_wins_over_teacher():
    assert classify(clip(corrected="what I said"), 0.7) == ("what I said", "corrected")


def test_confident_teacher_used_when_not_corrected():
    assert classify(clip(), 0.7) == ("the teacher text", "teacher")


def test_low_confidence_teacher_skipped():
    assert classify(clip(conf=0.5), 0.7) == (None, "low confidence")


def test_never_falls_back_to_what_was_pasted():
    # No teacher label yet: skipped, not trained on the student's own output.
    assert classify(clip(teacher=None, conf=None, ts=None), 0.7) == (None, "pending")
    # A rejected teacher label is skipped too.
    assert classify(clip(note="repeated phrase", conf=0.0), 0.7) == (None, "rejected")


def test_excluded_respected_even_when_corrected():
    assert classify(clip(corrected="fix", excluded=1), 0.7) == (None, "excluded")
    assert classify(clip(excluded=1), 0.7) == (None, "excluded")


def test_disagreeing_teacher_is_kept(tmp_path):
    """The clips where the teacher and the student disagree are the useful ones."""
    learner = Learner(tmp_path)
    d = Dictation(speech(2), raw="push it to get hub", pasted="push it to get hub")
    learner.record(d, keep_audio=True)
    learner.set_teacher(d.sample_id, "push it to GitHub", "granite-speech-4.1-2b", 0.92, None)
    found = labels.labels(learner.clips(), 0.7)
    assert [(x.text, x.source) for x in found] == [("push it to GitHub", "teacher")]


def test_labelled_from_db_matches_learner(tmp_path):
    learner = Learner(tmp_path)
    for i in range(3):
        d = Dictation(speech(1), raw=f"r{i}", pasted=f"p{i}")
        learner.record(d, keep_audio=True)
        learner.set_teacher(d.sample_id, f"t{i}", "m", [0.9, 0.2, 0.8][i], None)
    learner.edit_sample(1, "corrected one")
    found = labels.labelled_from_db(tmp_path / "learning.db", 0.7)
    assert sorted((x.text, x.source) for x in found) == [("corrected one", "corrected"), ("t2", "teacher")]


# --- held-out split ----------------------------------------------------------


def _labels(n_corrected, n_teacher):
    out = []
    for i in range(n_corrected):
        out.append(labels.Label(clip(i, corrected=f"c{i}", audio=f"c{i}.wav"), f"c{i}", "corrected"))
    for i in range(n_teacher):
        out.append(labels.Label(clip(1000 + i, audio=f"t{i}.wav"), f"t{i}", "teacher"))
    return out


def test_heldout_uses_teacher_labels_until_20_corrections():
    train, held, source = split(_labels(19, 300))
    assert source == "teacher" and 20 <= len(held) <= 50
    assert all(x.source == "teacher" for x in held)
    assert len(train) + len(held) == 319


def test_heldout_uses_corrections_once_there_are_20():
    train, held, source = split(_labels(20, 300))
    assert source == "corrected" and len(held) == 20
    assert all(x.source == "corrected" for x in held)
    train, held, _ = split(_labels(400, 300))
    assert 60 <= len(held) <= 100  # about a fifth


def test_trained_clips_never_move_into_the_heldout_set():
    seen_train: set[str] = set()
    for n in range(20, 400, 7):
        train, held, _ = split(_labels(n, 50))
        assert not {x.text for x in train} & {x.text for x in held}
        assert not seen_train & {x.text for x in held}
        seen_train |= {x.text for x in train if x.source == "corrected"}


# --- hallucinations and confidence -------------------------------------------


@pytest.mark.parametrize("text, seconds, why", [
    ("", 2, "empty"),
    ("  ...  ", 2, "empty"),
    ("thank you thank you thank you thank you", 3, "repeated phrase"),
    ("so so so so so", 2, "repeated phrase"),
    ("word " * 60, 3, "repeated phrase"),
    ("we could not possibly have said all of these different words in only one second of audio", 1,
     "too long for the clip"),
    ("yes", 12, "too short for the clip"),
])
def test_hallucinations_rejected(text, seconds, why):
    assert hallucination(text, seconds) == why


@pytest.mark.parametrize("text, seconds", [
    ("Push it to GitHub, please.", 2),
    ("very very very good", 2),
    ("yes", 1),
    ("Okay, sure.", 2.5),
])
def test_normal_text_accepted(text, seconds):
    assert hallucination(text, seconds) is None


def test_confidence_from_logprob():
    result = judge("hello there", speech(1.5), mean_logprob=-0.1)
    assert result.note is None and result.confidence == round(math.exp(-0.1), 3)


def test_heuristic_confidence_without_logprob():
    clean = judge("Push it to GitHub, please, and then deploy it.", speech(3), None)
    assert clean.note is None and clean.confidence == labels.HEURISTIC_CEILING
    slow = judge("one two", speech(6), None)  # 0.33 words a second
    assert slow.confidence < clean.confidence


def test_rejected_output_gets_zero_confidence():
    result = judge("the the the the the", speech(2), -0.01)
    assert result.text is None and result.confidence == 0.0 and result.note == "repeated phrase"


def test_voiced_seconds_ignores_silence():
    audio = np.concatenate([np.zeros(16_000 * 3, dtype=np.float32), speech(2)])
    assert 1.8 <= labels.voiced_seconds(audio) <= 2.2


# --- migration ---------------------------------------------------------------


OLD_SCHEMA = """
CREATE TABLE samples (id INTEGER PRIMARY KEY, ts REAL NOT NULL, audio TEXT, raw TEXT NOT NULL,
                      pasted TEXT NOT NULL, corrected TEXT);
CREATE TABLE rules (wrong TEXT NOT NULL, right TEXT NOT NULL, count INTEGER NOT NULL, PRIMARY KEY (wrong, right));
CREATE TABLE vocab (term TEXT PRIMARY KEY, count INTEGER NOT NULL);
INSERT INTO samples VALUES (1, 100, 'a.wav', 'get hub', 'get hub', 'GitHub');
INSERT INTO samples VALUES (2, 200, 'b.wav', 'hello', 'hello', NULL);
INSERT INTO vocab VALUES ('GitHub', 1);
"""


def test_migration_keeps_existing_data(tmp_path):
    db = sqlite3.connect(tmp_path / "learning.db")
    db.executescript(OLD_SCHEMA)
    db.close()

    learner = Learner(tmp_path)
    columns = {row[1] for row in learner._db.execute("PRAGMA table_info(samples)")}
    assert set(TEACHER_COLUMNS) | {"excluded"} <= columns
    assert [(s.id, s.text, s.excluded) for s in learner.samples()] == [(2, "hello", 0), (1, "GitHub", 0)]
    assert "GitHub" in learner.prompt()
    # Nothing has a teacher label yet, so both are queued, newest first.
    assert [i for i, _ in learner.teacher_queue()] == [2, 1]
    learner._db.close()

    # Running it again changes nothing.
    again = Learner(tmp_path)
    assert len(again.samples()) == 2


def test_teacher_queue_newest_first_and_skips_excluded(tmp_path):
    learner = Learner(tmp_path)
    ids = []
    for i in range(4):
        d = Dictation(speech(0.5), raw=f"r{i}", pasted=f"p{i}")
        learner.record(d, keep_audio=True)
        learner._db.execute("UPDATE samples SET ts = ? WHERE id = ?", (i, d.sample_id))
        ids.append(d.sample_id)
    learner._db.commit()
    learner.set_excluded(ids[2], True)
    learner.set_teacher(ids[1], "done", "m", 0.9, None)
    assert [i for i, _ in learner.teacher_queue()] == [ids[3], ids[0]]


def test_teacher_text_never_reaches_the_history_window(tmp_path):
    learner = Learner(tmp_path)
    d = Dictation(speech(1), raw="raw", pasted="pasted")
    learner.record(d, keep_audio=True)
    learner.set_teacher(d.sample_id, "TEACHER SUGGESTION", "m", 0.99, None)
    sample = learner.samples()[0]
    assert "TEACHER" not in repr(sample) and sample.text == "pasted"


def test_state_round_trip(tmp_path):
    learner = Learner(tmp_path)
    assert learner.get_state("x", 5) == 5
    learner.set_state("x", {"a": [1, 2]})
    learner.set_state("x", {"a": [3]})
    assert learner.get_state("x") == {"a": [3]}
