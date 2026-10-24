"""The parts that make Speechify an app: settings, start at login, one
instance, the tray, and the names shown for keys."""

import sys
import types

import pytest

from speechify import autostart, combos


# --- names ---------------------------------------------------------------------


def test_describe_keys_and_combos():
    assert combos.describe_key("ctrl_r") == "Right Ctrl"
    assert combos.describe_key("alt_r", mac=True) == "Right Option"
    assert combos.describe_key("f13") == "F13"
    assert combos.describe_key("scroll_lock") == "Scroll Lock"
    assert combos.describe_combo("<ctrl>+<alt>+h") == "Ctrl+Alt+H"
    assert combos.describe_combo("<alt>+<ctrl>+m", mac=True) == "Control+Option+M"


def test_keys_from_tk():
    assert combos.key_from_tk("Control_R") == "ctrl_r"
    assert combos.key_from_tk("Option_R") == "alt_r"
    assert combos.key_from_tk("F13") == "f13"
    assert combos.key_from_tk("a") is None  # would type while held
    assert combos.key_from_tk("Return") is None


# --- start at login --------------------------------------------------------------


@pytest.mark.skipif(sys.platform != "linux", reason="XDG autostart")
def test_autostart_desktop_entry(tmp_path, monkeypatch):
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path))
    monkeypatch.setenv("APPIMAGE", "/home/me/Apps/Speechify 1.AppImage")
    assert not autostart.is_enabled()
    autostart.set_enabled(True)
    entry = (tmp_path / "autostart" / "speechify.desktop").read_text()
    assert "Exec='/home/me/Apps/Speechify 1.AppImage'" in entry
    assert autostart.is_enabled()
    autostart.set_enabled(False)
    assert not autostart.is_enabled()


def test_autostart_command_from_source(monkeypatch):
    monkeypatch.delenv("APPIMAGE", raising=False)
    assert autostart.command() == [sys.executable, "-m", "speechify"]


# --- hotkeys ---------------------------------------------------------------------


def test_hotkeys_rebind(monkeypatch):
    from speechify import hotkeys

    monkeypatch.setattr(hotkeys.keys, "listener", lambda press, release: types.SimpleNamespace())
    fired = []
    monkeypatch.setattr(hotkeys.threading, "Thread",
                        lambda target, daemon: types.SimpleNamespace(start=target))
    hk = hotkeys.GlobalHotkeys({"<ctrl>+<alt>+h": lambda: fired.append("old")})
    hk.rebind({"<ctrl>+<alt>+j": lambda: fired.append("new")})
    for key in ("h", "j"):
        hk._press("ctrl_l", None, None)
        hk._press("alt_l", None, None)
        hk._press(None, None, key)
        hk._release(None, None, key)
        hk._release("alt_l", None, None)
        hk._release("ctrl_l", None, None)
    assert fired == ["new"]
