import csv
import sqlite3

import numpy as np

from speechify.learning import SCHEMA, Dictation, Learner


def _record(learner, text):
    d = Dictation(np.zeros(1600, dtype=np.float32), text, text)
    learner.record(d, keep_audio=True)
    return d


def test_every_dictation_is_saved_and_editable(tmp_path):
    learner = Learner(tmp_path, autocorrect_after=1)
    d = _record(learner, "deploy to cube control")
    _record(learner, "hello world")

    samples = learner.samples()
    assert [s.text for s in samples] == ["hello world", "deploy to cube control"]
    assert all(s.path.exists() for s in samples)

    assert learner.edit_sample(d.sample_id, "deploy to kubectl") == [("cube control", "kubectl")]
    edited = next(s for s in learner.samples() if s.id == d.sample_id)
    assert edited.corrected == "deploy to kubectl" and edited.text == "deploy to kubectl"
    assert learner.apply("cube control") == "kubectl"

    # Setting the text back to what was pasted clears the correction.
    learner.edit_sample(d.sample_id, "deploy to cube control")
    assert next(s for s in learner.samples() if s.id == d.sample_id).corrected is None


def test_search(tmp_path):
    learner = Learner(tmp_path)
    _record(learner, "alpha one")
    _record(learner, "beta two")
    assert [s.text for s in learner.samples("beta")] == ["beta two"]


def test_exclude_and_delete(tmp_path):
    learner = Learner(tmp_path)
    keep = _record(learner, "keep me")
    drop = _record(learner, "drop me")
    gone = _record(learner, "delete me")

    learner.set_excluded(drop.sample_id, True)
    path = next(s.path for s in learner.samples() if s.id == gone.sample_id)
    learner.delete_sample(gone.sample_id)
    assert not path.exists()
    stats = learner.stats()
    assert stats["samples"] == 2 and stats["excluded"] == 1

    out = tmp_path / "export"
    assert learner.export(out) == 1
    rows = list(csv.reader(open(out / "metadata.csv", encoding="utf-8")))
    assert rows[0] == ["file_name", "transcription"] and rows[1][1] == "keep me"
    assert (out / rows[1][0]).exists()


def test_upgrades_old_database(tmp_path):
    old_schema = SCHEMA.replace(
        "corrected TEXT,          -- what you said it should have been\n"
        "    excluded INTEGER NOT NULL DEFAULT 0  -- 1 = keep, but don't train on it",
        "corrected TEXT",
    )
    assert old_schema != SCHEMA
    db = sqlite3.connect(tmp_path / "learning.db")
    db.executescript(old_schema)
    db.execute("INSERT INTO samples (ts, audio, raw, pasted) VALUES (0, 'a.wav', 'x', 'x')")
    db.commit()
    db.close()
    learner = Learner(tmp_path)
    assert learner.samples()[0].excluded == 0
