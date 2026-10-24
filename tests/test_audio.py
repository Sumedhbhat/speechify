"""The microphone is open only while the talk key is held (unless keep_mic_open)."""

import sys
import threading
import time
import types

import numpy as np
import pytest

from speechify import audio


class FakeStream:
    opened: list["FakeStream"] = []
    fail = False

    def __init__(self, callback, **kwargs):
        if FakeStream.fail:
            raise OSError("no input device")
        self.callback = callback
        self.running = False
        FakeStream.opened.append(self)

    def start(self):
        self.running = True

    def stop(self):
        self.running = False

    def close(self):
        pass

    def feed(self, value: float):
        self.callback(np.full((audio.BLOCK, 1), value, dtype=np.float32), audio.BLOCK, None, None)


@pytest.fixture(autouse=True)
def fake_sounddevice(monkeypatch):
    FakeStream.opened, FakeStream.fail = [], False
    monkeypatch.setitem(sys.modules, "sounddevice", types.SimpleNamespace(InputStream=FakeStream))


def wait_for(predicate, timeout=2.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.005)
    raise AssertionError("timed out")


def is_open() -> bool:
    return any(s.running for s in FakeStream.opened)


def test_mic_is_closed_until_the_key_goes_down():
    rec = audio.Recorder()
    time.sleep(0.05)
    assert not is_open()

    rec.start()
    wait_for(is_open)
    stream = FakeStream.opened[-1]
    assert not rec.live
    stream.feed(0.1)
    assert rec.live and rec.level == pytest.approx(0.1)

    clip = rec.stop()
    assert clip.size == audio.BLOCK
    wait_for(lambda: not is_open())
    assert rec.level == 0 and not rec.live
    rec.close()


def test_audio_after_release_is_not_kept_for_the_next_dictation():
    rec = audio.Recorder()
    rec.start()
    wait_for(is_open)
    stream = FakeStream.opened[-1]
    rec.stop()
    stream.feed(0.5)  # arrives before the worker has closed the stream
    rec.start()
    assert rec.stop().size == 0
    rec.close()


def test_quick_presses_settle_on_the_last_request():
    rec = audio.Recorder()
    for _ in range(20):
        rec.start()
        rec.stop()
    wait_for(lambda: not is_open())
    time.sleep(0.05)
    assert not is_open()
    rec.start()
    wait_for(is_open)
    assert sum(s.running for s in FakeStream.opened) == 1
    rec.close()
    assert not is_open()


def test_keep_open_opens_at_startup_with_preroll():
    rec = audio.Recorder(keep_open=True)
    assert is_open()
    stream = FakeStream.opened[-1]
    stream.feed(0.2)  # before the key goes down
    rec.start()
    stream.feed(0.2)
    assert rec.stop().size == 2 * audio.BLOCK
    time.sleep(0.05)
    assert is_open()
    rec.close()


def test_open_failure_is_reported_and_retried():
    errors = []
    got = threading.Event()
    FakeStream.fail = True
    rec = audio.Recorder(on_error=lambda e: (errors.append(e), got.set()))
    rec.start()
    assert got.wait(2)
    assert errors == ["no input device"] and rec.error == "no input device"
    rec.stop()

    FakeStream.fail = False
    rec.start()
    assert rec.error is None
    wait_for(is_open)
    rec.stop()
    rec.close()


def test_pill_helpers():
    pytest.importorskip("tkinter")
    from speechify import ui

    assert ui.level_to_bar(0.0) == 0.0
    assert ui.level_to_bar(1.0) == 1.0
    assert 0.5 < ui.level_to_bar(0.05) < 1.0  # normal speech
    assert ui.level_to_bar(0.001) < 0.1  # room noise

    points = ui.capsule(0, 0, 100, 40)
    xs, ys = points[0::2], points[1::2]
    assert min(xs) == pytest.approx(0) and max(xs) == pytest.approx(100)
    assert min(ys) == pytest.approx(0) and max(ys) == pytest.approx(40)
    assert ui._mix("#ffffff", "#000000", 0.5) == "#808080"
