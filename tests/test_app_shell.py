"""The parts that make Speechify an app: settings, start at login, one
instance, the tray, and the names shown for keys."""

import sys
import types

import pytest

from speechify import autostart, combos, settings
from speechify.config import Config


@pytest.fixture
def home(tmp_path, monkeypatch):
    monkeypatch.setenv("SPEECHIFY_HOME", str(tmp_path / "data"))
    return tmp_path


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


# --- settings ------------------------------------------------------------------


def test_settings_changes_are_typed_and_saved(home):
    cfg = Config.load()
    values = {"min_hold_ms": "300", "keep_mic_open": True, "idle_minutes": "5", "language": " de "}
    assert settings.problems(values) == []
    changed = settings.changes(cfg, values)
    assert changed == {"min_hold_ms": 300, "keep_mic_open": True, "language": "de"}
    settings.save(cfg, changed)
    again = Config.load()
    assert (again.min_hold_ms, again.keep_mic_open, again.language) == (300, True, "de")


def test_settings_problems():
    assert settings.problems({"min_hold_ms": "soon"}) == ["min hold ms must be a number"]
    assert settings.problems({"beam_size": "0"})
    assert settings.problems({"device": "tpu"})
    assert settings.problems({"correct_hotkey": "<hyper>+f"})
    same = {"correct_hotkey": "<ctrl>+<alt>+f", "history_hotkey": "<alt>+<ctrl>+f"}
    assert settings.problems(same) == ["two shortcuts are the same"]


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


# --- tray ------------------------------------------------------------------------


def test_tray_menu(monkeypatch):
    pytest.importorskip("PIL")
    made = {}

    class MenuItem:
        def __init__(self, text, action, enabled=True, default=False):
            self.text, self.action, self.enabled, self.default = text, action, enabled, default

    class Menu:
        SEPARATOR = object()

        def __init__(self, *items):
            self.items = items

    class Icon:
        def __init__(self, name, image, title, menu):
            made.update(icon=self, menu=menu)
            self.icon = image

        def run(self):
            made["ran"] = True

        def run_detached(self):
            made["ran"] = True

        def stop(self):
            made["stopped"] = True

    monkeypatch.setitem(sys.modules, "pystray", types.SimpleNamespace(MenuItem=MenuItem, Menu=Menu, Icon=Icon))
    from speechify.tray import Tray

    calls = []
    hotkey = ["Ctrl+Alt+H"]
    tray = Tray(lambda: "Parakeet on cpu", lambda: "Hold Right Ctrl to talk", [
        ("Settings…", lambda: calls.append("settings")),
        (lambda: f"History…  {hotkey[0]}", lambda: calls.append("history")),
        None,
        ("Quit", lambda: calls.append("quit")),
    ])
    assert tray.start()
    items = made["menu"].items
    texts = [i.text(i) if callable(i.text) else i.text for i in items if isinstance(i, MenuItem)]
    assert texts == ["Parakeet on cpu", "Hold Right Ctrl to talk", "Settings…", "History…  Ctrl+Alt+H", "Quit"]
    hotkey[0] = "Ctrl+Alt+J"
    assert items[4].text(items[4]) == "History…  Ctrl+Alt+J"
    assert items[3].default and not items[4].default
    items[3].action("icon", "item")
    assert calls == ["settings"]

    normal = made["icon"].icon
    tray.set_listening(True)
    assert made["icon"].icon is not normal
    tray.set_listening(False)
    assert made["icon"].icon is normal
    tray.stop()
    assert made["stopped"]


def test_tray_failure_is_not_fatal(monkeypatch):
    monkeypatch.setitem(sys.modules, "pystray", None)  # import fails
    from speechify.tray import Tray

    tray = Tray(lambda: "", lambda: "", [("Quit", lambda: None)])
    assert not tray.start()
    tray.set_listening(True)
    tray.stop()


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
