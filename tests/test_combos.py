import pytest

from speechify.combos import Matcher, parse


def test_parse():
    assert parse("<ctrl>+<alt>+f") == (frozenset({"ctrl", "alt"}), "f")
    assert parse("<ctrl>+<shift>+<f13>") == (frozenset({"ctrl", "shift"}), "f13")
    with pytest.raises(ValueError):
        parse("<hyper>+f")


def test_windows_ctrl_alt_letter_has_no_char():
    # The case pynput's GlobalHotKeys misses: with Ctrl+Alt held, Windows
    # reports the F key as vk 0x46 with no character.
    m = Matcher(["<ctrl>+<alt>+f", "<ctrl>+<alt>+h"])
    assert m.press("ctrl_l", None, None) == []
    assert m.press("alt_l", None, None) == []
    assert m.press(None, 0x46, None) == ["<ctrl>+<alt>+f"]


def test_x11_keysym_and_char():
    m = Matcher(["<ctrl>+<alt>+f"])
    m.press("ctrl", None, None)
    m.press("alt_gr", None, None)
    assert m.press(None, ord("f"), "f") == ["<ctrl>+<alt>+f"]


def test_modifiers_must_match_exactly():
    m = Matcher(["<ctrl>+<alt>+f"])
    m.press("ctrl_l", None, None)
    assert m.press(None, 0x46, None) == []  # Ctrl+F alone is the app's own Find
    m.release(None, 0x46, None)
    m.press("alt_l", None, None)
    m.press("shift_l", None, None)
    assert m.press(None, 0x46, None) == []  # Ctrl+Alt+Shift+F is something else


def test_auto_repeat_fires_once_and_release_rearms():
    m = Matcher(["<ctrl>+<alt>+m"])
    m.press("ctrl_r", None, None)
    m.press("alt_l", None, None)
    assert m.press(None, 0x4D, None) == ["<ctrl>+<alt>+m"]
    assert m.press(None, 0x4D, None) == []  # held down: OS repeats the key
    m.release(None, 0x4D, None)
    assert m.press(None, 0x4D, None) == ["<ctrl>+<alt>+m"]


def test_releasing_a_modifier():
    m = Matcher(["<ctrl>+<alt>+f"])
    m.press("ctrl_l", None, None)
    m.press("alt_l", None, None)
    m.release("alt_l", None, None)
    assert m.press(None, 0x46, None) == []


def test_named_final_key():
    m = Matcher(["<ctrl>+<shift>+<f13>"])
    m.press("ctrl", None, None)
    m.press("shift", None, None)
    assert m.press("f13", None, None) == ["<ctrl>+<shift>+<f13>"]
