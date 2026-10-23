"""Linux and macOS support that can be checked on any OS."""

import os
import sys
import threading
import time
from pathlib import Path

import pytest

from speechify import combos, config, keymaps, keys
from speechify.engines import catalog, host


def test_same_key():
    assert combos.same_key("ctrl_r", "ctrl_r", None)
    assert not combos.same_key("ctrl_r", "ctrl_l", None)
    assert combos.same_key("ctrl", "ctrl_l", None)  # pynput's Key.ctrl is the left one
    assert combos.same_key("alt_r", "alt_gr", None)  # X11 calls Right Alt AltGr
    assert combos.same_key("F", None, "f")
    assert not combos.same_key("f", "f13", None)


@pytest.mark.parametrize("table,allowed", [(keymaps.EVDEV, keymaps.EVDEV_NAMES),
                                           (keymaps.MAC, keymaps.MAC_NAMES)], ids=["evdev", "mac"])
def test_key_tables_cover_the_defaults(table, allowed):
    chars = {c for _, c in table.values() if c}
    names = {n for n, _ in table.values() if n}
    assert set("abcdefghijklmnopqrstuvwxyz0123456789") <= chars
    assert {"ctrl_l", "ctrl_r", "alt_l", "alt_r", "shift_l", "shift_r", "cmd_l", "cmd_r",
            "f1", "f12", "f13", "caps_lock", "enter", "esc"} <= names
    assert names - {"caps_lock"} <= allowed


def test_output_key_codes():
    assert keymaps.EVDEV[keymaps.EV_V] == (None, "v")
    assert keymaps.EVDEV[keymaps.EV_LEFTCTRL] == ("ctrl_l", None)
    assert keymaps.MAC[keymaps.MAC_V] == (None, "v")
    assert keymaps.MAC[keymaps.MAC_CMD] == ("cmd_l", None)
    assert set(keymaps.MAC_MODIFIER_BITS) <= set(keymaps.MAC)


def test_hotkeys_from_native_backends():
    # The native backends report letters by character only, never a vk that
    # could collide with another key's code.
    m = combos.Matcher(["<ctrl>+<alt>+f"])
    m.press("ctrl_l", None, None)
    m.press("alt_r", None, None)
    assert m.press(None, None, "f") == ["<ctrl>+<alt>+f"]


@pytest.mark.parametrize("platform,env,expected", [
    ("win32", {}, "pynput"),
    ("darwin", {}, "quartz"),
    ("linux", {"DISPLAY": ":0"}, "pynput"),
    ("linux", {"WAYLAND_DISPLAY": "wayland-0"}, "evdev"),
    ("linux", {"XDG_SESSION_TYPE": "wayland"}, "evdev"),
    ("linux", {"WAYLAND_DISPLAY": "wayland-0", "SPEECHIFY_KEYBOARD": "pynput"}, "pynput"),
])
def test_keyboard_backend(monkeypatch, platform, env, expected):
    for var in ("WAYLAND_DISPLAY", "XDG_SESSION_TYPE", "SPEECHIFY_KEYBOARD"):
        monkeypatch.delenv(var, raising=False)
    for var, value in env.items():
        monkeypatch.setenv(var, value)
    monkeypatch.setattr(sys, "platform", platform)
    keys.backend.cache_clear()
    try:
        assert keys.backend() == expected
    finally:
        keys.backend.cache_clear()


@pytest.mark.parametrize("platform,cuda,ptt,model", [
    ("win32", True, "ctrl_r", "whisper-large-v3-turbo"),
    ("darwin", False, "alt_r", "parakeet-tdt-0.6b-v3"),
    ("linux", True, "ctrl_r", "whisper-large-v3-turbo"),
    ("linux", False, "ctrl_r", "parakeet-tdt-0.6b-v3"),
])
def test_platform_defaults(monkeypatch, tmp_path, platform, cuda, ptt, model):
    monkeypatch.setenv("SPEECHIFY_HOME", str(tmp_path))
    monkeypatch.setattr(sys, "platform", platform)
    monkeypatch.setattr(config, "has_cuda", lambda: cuda)
    cfg = config.Config.load()
    assert (cfg.ptt_key, cfg.model, cfg.device) == (ptt, model, "auto")
    assert cfg.compute_type == ("float16" if cuda else "int8")
    assert f'ptt_key = "{ptt}"' in (tmp_path / "config.toml").read_text(encoding="utf-8")


def test_saved_settings_win_over_platform_defaults(monkeypatch, tmp_path):
    monkeypatch.setenv("SPEECHIFY_HOME", str(tmp_path))
    (tmp_path / "config.toml").write_text('model = "moonshine-base"\n', encoding="utf-8")
    monkeypatch.setattr(config, "has_cuda", lambda: False)
    cfg = config.Config.load()
    assert cfg.model == "moonshine-base" and cfg.device == "auto"


def test_cuda_override(monkeypatch):
    monkeypatch.setattr(sys, "platform", "darwin")
    monkeypatch.setenv("SPEECHIFY_CUDA", "1")
    assert config.has_cuda()
    monkeypatch.setattr(sys, "platform", "win32")
    monkeypatch.setenv("SPEECHIFY_CUDA", "0")
    assert not config.has_cuda()


def test_mac_data_dir(monkeypatch, tmp_path):
    monkeypatch.delenv("SPEECHIFY_HOME", raising=False)
    monkeypatch.setattr(sys, "platform", "darwin")
    monkeypatch.setattr(Path, "home", lambda: tmp_path)
    assert config.data_dir() == tmp_path / "Library" / "Application Support" / "speechify"


def test_no_cuda_packages_without_a_gpu():
    for backend in catalog.BACKENDS.values():
        for req in backend.requirements_for(cuda=False):
            assert "nvidia" not in req and "gpu" not in req, (backend.id, req)
    onnx = catalog.BACKENDS["onnx"]
    assert "onnxruntime" in onnx.requirements_for(cuda=False)
    assert "onnxruntime-gpu[cuda,cudnn]" in onnx.requirements_for(cuda=True)
    assert "onnxruntime" not in onnx.requirements_for(cuda=True)  # same module; can't have both
    assert any("cudnn" in r for r in catalog.BACKENDS["whisper"].requirements_for(cuda=True))


def test_gpu_change_reinstalls_the_environment(monkeypatch):
    monkeypatch.setattr(host, "has_cuda", lambda: False)
    cpu = host._requirements_hash("whisper")
    monkeypatch.setattr(host, "has_cuda", lambda: True)
    assert host._requirements_hash("whisper") != cpu


def test_linux_worker_finds_pip_cuda_libraries(monkeypatch, tmp_path):
    monkeypatch.setenv("SPEECHIFY_ENGINES", str(tmp_path))
    monkeypatch.setenv("LD_LIBRARY_PATH", "/opt/existing")
    monkeypatch.setattr(sys, "platform", "linux")
    monkeypatch.setattr(host, "_shim", lambda: tmp_path)
    for lib in ("cublas", "cudnn"):
        (tmp_path / "whisper" / "lib" / "python3.12" / "site-packages" / "nvidia" / lib / "lib").mkdir(parents=True)
    spec = catalog.MODELS["whisper-large-v3-turbo"]
    paths = host._worker_env(spec, {}, False, True)["LD_LIBRARY_PATH"].split(os.pathsep)
    assert [Path(p).parent.name for p in paths[:2]] == ["cublas", "cudnn"]
    assert paths[-1] == "/opt/existing"


# --- Linux evdev / uinput -------------------------------------------------------

linux = pytest.mark.skipif(not sys.platform.startswith("linux"), reason="Linux only")


@linux
def test_decode_events():
    from speechify import linuxkeys

    raw = b"".join([
        linuxkeys._EVENT.pack(0, 0, linuxkeys.EV_KEY, 97, 1),
        linuxkeys._EVENT.pack(0, 0, linuxkeys.EV_SYN, 0, 0),
        linuxkeys._EVENT.pack(0, 0, linuxkeys.EV_KEY, 97, 0),
    ])
    assert linuxkeys.decode(raw) == [(97, 1), (97, 0)]


def _can_use_uinput() -> bool:
    if not sys.platform.startswith("linux"):
        return False
    import grp

    try:
        in_input_group = grp.getgrnam("input").gr_gid in os.getgroups()
    except KeyError:
        in_input_group = False
    can = os.access("/dev/uinput", os.W_OK) and in_input_group
    # CI sets this so the round trip below can't silently skip.
    if os.environ.get("SPEECHIFY_REQUIRE_UINPUT") and not can:
        raise RuntimeError("SPEECHIFY_REQUIRE_UINPUT is set, but /dev/uinput or the input group isn't usable")
    return can


@linux
@pytest.mark.skipif(_can_use_uinput(), reason="this machine allows /dev/input access")
def test_explains_missing_input_permission():
    from speechify import linuxkeys

    if not list(Path("/dev/input").glob("event*")):
        pytest.skip("no input devices")
    with pytest.raises(PermissionError, match="input' group"):
        linuxkeys.Listener(lambda *a: None, lambda *a: None).start()
    with pytest.raises(PermissionError, match="uinput"):
        linuxkeys.Keyboard()


@linux
@pytest.mark.skipif(not _can_use_uinput(), reason="needs read access to /dev/input and write to /dev/uinput")
def test_virtual_keyboard_round_trip():
    """Type through uinput and read it back through evdev: the whole Wayland path."""
    from speechify import linuxkeys

    events: list[tuple[str, str | None, str | None]] = []
    got_v_up = threading.Event()

    def press(name, vk, char):
        events.append(("down", name, char))

    def release(name, vk, char):
        events.append(("up", name, char))
        if char == "v":
            got_v_up.set()

    kb = linuxkeys.Keyboard(name="speechify test keyboard")
    listener = linuxkeys.Listener(press, release)  # ignores only the app's own device
    try:
        listener.start()
        time.sleep(0.2)
        kb.chord(keymaps.EV_LEFTCTRL, keymaps.EV_V)
        assert got_v_up.wait(5), events
    finally:
        listener.stop()
        kb.close()
    assert events[-4:] == [("down", "ctrl_l", None), ("down", None, "v"),
                           ("up", None, "v"), ("up", "ctrl_l", None)]


@linux
@pytest.mark.skipif(not _can_use_uinput(), reason="needs read access to /dev/input and write to /dev/uinput")
def test_listener_ignores_its_own_virtual_keyboard():
    from speechify import linuxkeys

    seen = []
    kb = linuxkeys.Keyboard()  # the app's own name
    listener = linuxkeys.Listener(lambda *a: seen.append(a), lambda *a: None)
    try:
        listener.start()
        time.sleep(0.2)
        kb.chord(keymaps.EV_LEFTCTRL, keymaps.EV_V)
        time.sleep(0.5)
    finally:
        listener.stop()
        kb.close()
    assert seen == []


# --- macOS Quartz ---------------------------------------------------------------


@pytest.mark.skipif(sys.platform != "darwin", reason="macOS only")
def test_mac_keys_round_trip_or_explains_permission():
    """With permission, typed keys come back through the event tap. Without,
    starting says what to allow instead of failing silently."""
    from speechify import mackeys

    events = []
    got = threading.Event()

    def release(name, vk, char):
        events.append((name, char))
        if name == "left":
            got.set()

    listener = mackeys.Listener(lambda *a: None, release)
    try:
        listener.start()
    except PermissionError as exc:
        assert "Privacy & Security" in str(exc)
        return
    try:
        mackeys.shift_left(1)
        assert got.wait(5), events
    finally:
        listener.stop()


@pytest.mark.parametrize("platform,key,ok", [
    ("darwin", "alt_r", True), ("darwin", "fn", True),
    ("darwin", "caps_lock", False),  # macOS only reports toggles, so it can't be held
    ("darwin", "scroll_lock", False),
    ("linux", "scroll_lock", True), ("linux", "fn", False), ("linux", "F", True),
])
def test_talk_key_must_exist_on_this_backend(monkeypatch, platform, key, ok):
    monkeypatch.setattr(sys, "platform", platform)
    monkeypatch.setenv("SPEECHIFY_KEYBOARD", "evdev")
    keys.backend.cache_clear()
    try:
        if ok:
            assert keys.check_key_name(key) == (key.lower() if len(key) == 1 else key)
        else:
            with pytest.raises(ValueError):
                keys.check_key_name(key)
    finally:
        keys.backend.cache_clear()


def test_installed_app_uses_its_own_uv(monkeypatch, tmp_path):
    exe = "uv.exe" if sys.platform == "win32" else "uv"
    (tmp_path / exe).write_text("")
    monkeypatch.delenv("UV", raising=False)
    monkeypatch.setattr(sys, "frozen", True, raising=False)
    monkeypatch.setattr(sys, "_MEIPASS", str(tmp_path), raising=False)
    assert host.find_uv() == str(tmp_path / exe)


@pytest.mark.parametrize("wayland,tools,missing", [
    (True, {"wl-copy", "wl-paste"}, None),
    (True, {"xclip"}, "wl-clipboard"),
    (False, {"xsel"}, None),
    (False, set(), "xclip"),
])
def test_clipboard_problem(monkeypatch, wayland, tools, missing):
    from speechify import linuxsetup

    monkeypatch.setattr(linuxsetup, "_is_wayland", lambda: wayland)
    monkeypatch.setattr(linuxsetup.shutil, "which", lambda name: f"/usr/bin/{name}" if name in tools else None)
    problem = linuxsetup.clipboard_problem()
    assert problem is None if missing is None else missing in problem


def test_appimage_menu_entry(monkeypatch, tmp_path):
    from speechify import linuxsetup

    appdir = tmp_path / "mnt"
    appdir.mkdir()
    (appdir / "speechify.png").write_bytes(b"png")
    monkeypatch.setenv("APPIMAGE", '/opt/my apps/Speechify "1".AppImage')
    monkeypatch.setenv("APPDIR", str(appdir))
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "share"))
    linuxsetup.register_appimage()
    entry = (tmp_path / "share" / "applications" / "speechify.desktop").read_text()
    assert 'Exec="/opt/my apps/Speechify \\\\"1\\\\".AppImage"\n' in entry
    assert (tmp_path / "share" / "icons" / "hicolor" / "256x256" / "apps" / "speechify.png").read_bytes() == b"png"
