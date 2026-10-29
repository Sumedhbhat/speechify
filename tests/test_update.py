"""Updating from the tray: which release is offered, which file is fetched,
and that a download is only installed if it matches SHA256SUMS."""

import hashlib
import io
import sys
import types

import pytest

from speechify import update


def release(tag, prerelease=None, draft=False, names=()):
    if prerelease is None:
        prerelease = "-" in tag
    return {
        "tag_name": tag, "prerelease": prerelease, "draft": draft,
        "html_url": f"https://example.invalid/{tag}",
        "assets": [{"name": n, "browser_download_url": f"https://example.invalid/{tag}/{n}"} for n in names],
    }


def test_versions_sort_nightlies_before_their_release():
    order = ["0.1.0-nightly.9", "0.1.0-nightly.10", "0.1.0", "0.1.1", "0.2.0-nightly.1", "0.2.0"]
    keys = [update.version_key(v) for v in order]
    assert keys == sorted(keys)
    assert update.version_key("v0.1.0-dev.3") < update.version_key("0.1.0")
    assert update.version_key("0.0.0+unknown") is None


def test_nightly_builds_follow_nightlies_and_stable_releases():
    releases = [release("v0.1.0-nightly.56"), release("v0.1.0-nightly.57"), release("v0.1.0-nightly.60", draft=True)]
    assert update.newest(releases, "0.1.0-nightly.56").version == "0.1.0-nightly.57"
    assert update.newest(releases, "0.1.0-nightly.57") is None
    assert update.newest(releases + [release("v0.1.0")], "0.1.0-nightly.57").version == "0.1.0"


def test_stable_builds_only_take_stable_releases():
    releases = [release("v0.1.0"), release("v0.1.1"), release("v0.2.0-nightly.3")]
    assert update.newest(releases, "0.1.0").version == "0.1.1"
    assert update.newest(releases, "0.1.1") is None
    assert update.newest(releases, "0.0.0+unknown") is None


@pytest.mark.parametrize("kind,machine,name", [
    ("windows", "AMD64", "Speechify-0.2.0-windows-x64-setup.exe"),
    ("macos", "arm64", "Speechify-0.2.0-macos-arm64.dmg"),
    ("appimage", "x86_64", "Speechify-0.2.0-x86_64.AppImage"),
    ("deb", "x86_64", "speechify_0.2.0_amd64.deb"),
])
def test_picks_the_file_for_this_install(monkeypatch, kind, machine, name):
    names = ["SHA256SUMS", "Speechify-0.2.0-windows-x64-setup.exe", "Speechify-0.2.0-macos-arm64.dmg",
             "Speechify-0.2.0-x86_64.AppImage", "speechify_0.2.0_amd64.deb"]
    monkeypatch.setattr(update.platform, "machine", lambda: machine)
    r = update.newest([release("v0.2.0", names=names)], "0.1.0")
    assert update.asset_name(r, kind) == name


def test_no_file_for_this_install(monkeypatch):
    monkeypatch.setattr(update.platform, "machine", lambda: "x86_64")
    r = update.newest([release("v0.2.0", names=["Speechify-0.2.0-macos-arm64.dmg"])], "0.1.0")
    with pytest.raises(update.UpdateError):
        update.asset_name(r, "macos")


class FakeResponse(io.BytesIO):
    def __init__(self, data):
        super().__init__(data)
        self.headers = {"Content-Length": str(len(data))}


@pytest.mark.parametrize("tampered", [False, True])
def test_download_checks_the_checksum(monkeypatch, tmp_path, tampered):
    deb = b"a" * (3 << 20)
    name = "speechify_0.2.0.nightly.5_amd64.deb"  # GitHub's name for 0.2.0~nightly.5
    sums = f"{hashlib.sha256(deb).hexdigest()}  speechify_0.2.0~nightly.5_amd64.deb\n".encode()
    files = {"SHA256SUMS": sums, name: deb + (b"!" if tampered else b"")}
    monkeypatch.setattr(update, "_open", lambda url, timeout=30: FakeResponse(files[url.rsplit("/", 1)[1]]))
    r = update.newest([release("v0.2.0-nightly.5", names=list(files))], "0.1.0-nightly.1")
    seen = []
    if tampered:
        with pytest.raises(update.UpdateError, match="checksum"):
            update.download(r, "deb", tmp_path, seen.append)
        assert not (tmp_path / name).exists()
    else:
        assert update.download(r, "deb", tmp_path, seen.append).read_bytes() == deb
        assert seen[-1] == 1.0


def test_check_explains_when_github_is_unreachable(monkeypatch):
    def offline(url, timeout=30):
        raise OSError("no network")

    monkeypatch.setattr(update, "_open", offline)
    with pytest.raises(update.UpdateError, match="GitHub"):
        update.check("0.1.0")


def test_source_checkouts_dont_update_themselves(monkeypatch):
    monkeypatch.delattr(sys, "frozen", raising=False)
    assert update.install_kind() is None
    assert "git pull" in update.why_not()


@pytest.mark.skipif(sys.platform != "linux", reason="Linux installs")
def test_linux_install_kinds(monkeypatch, tmp_path):
    monkeypatch.setattr(sys, "frozen", True, raising=False)
    monkeypatch.setenv("APPIMAGE", str(tmp_path / "Speechify.AppImage"))
    assert update.install_kind() == "appimage"
    monkeypatch.delenv("APPIMAGE")
    monkeypatch.setattr(sys, "executable", "/opt/speechify/speechify")
    monkeypatch.setattr(update.shutil, "which", lambda name: "/usr/bin/dpkg")
    assert update.install_kind() == "deb"
    monkeypatch.setattr(sys, "executable", str(tmp_path / "speechify"))
    assert update.install_kind() is None


def test_started_programs_dont_inherit_the_frozen_setup(monkeypatch):
    monkeypatch.setenv("LD_LIBRARY_PATH", "/opt/speechify/_internal")
    monkeypatch.setenv("LD_LIBRARY_PATH_ORIG", "/usr/local/lib")
    monkeypatch.setenv("APPIMAGE", "/x.AppImage")
    env = update._clean_env()
    assert env["LD_LIBRARY_PATH"] == "/usr/local/lib"
    assert env["PYINSTALLER_RESET_ENVIRONMENT"] == "1"
    assert "APPIMAGE" not in env and "LD_LIBRARY_PATH_ORIG" not in env


@pytest.mark.skipif(sys.platform == "win32", reason="sh")
def test_after_exit_waits_then_runs(tmp_path):
    import subprocess

    sleeper = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(0.5)"])
    marker = tmp_path / "ran"
    script = f"echo swapped > '{marker}'"
    helper = subprocess.Popen(["/bin/sh", "-c", update._AFTER_EXIT, "sh", str(sleeper.pid), script,
                               "/bin/sh", "-c", f"echo started >> '{marker}'"])
    assert not marker.exists()  # still waiting for the old process
    sleeper.wait()  # reaped, the way the old app's launcher would
    assert helper.wait(timeout=10) == 0
    assert marker.read_text() == "swapped\nstarted\n"


class Shell:
    """The parts of App the update flow uses, run synchronously."""

    def __init__(self):
        self.shown, self.quit_called = [], False
        self.ui = types.SimpleNamespace(post=lambda fn: fn(), show=lambda *a: self.shown.append(a[1]),
                                        hide=lambda: None)
        self.tray = types.SimpleNamespace(refresh=lambda: None)


@pytest.mark.parametrize("fails", [False, True])
def test_app_installs_or_explains(monkeypatch, tmp_path, fails):
    import tkinter.messagebox

    from speechify.app import App

    errors = []
    monkeypatch.setattr(tkinter.messagebox, "showerror", lambda title, text: errors.append(text))
    monkeypatch.setattr(update, "download_folder", lambda kind: tmp_path)

    def download(release, kind, folder, progress):
        progress(0.5)
        if fails:
            raise update.UpdateError("checksum mismatch")
        return folder / "setup.exe"

    installed = []
    monkeypatch.setattr(update, "download", download)
    monkeypatch.setattr(update, "install", lambda path, kind: installed.append((path.name, kind)))
    app = Shell()
    app._updating = True
    app.quit = lambda: setattr(app, "quit_called", True)
    App._install_update(app, update.Release("0.2.0", "https://example.invalid", {}), "windows")
    assert "Downloading update… 50%" in app.shown
    if fails:
        assert not app._updating and not app.quit_called and not installed
        assert "checksum mismatch" in errors[0]
    else:
        assert installed == [("setup.exe", "windows")] and app.quit_called and not errors


def test_stable_builds_ask_for_the_latest_release(monkeypatch):
    import json
    import urllib.error

    asked = []

    def fake(url, timeout=30):
        asked.append(url)
        if url == update.LATEST:
            return FakeResponse(json.dumps(release("v0.3.0")).encode())
        return FakeResponse(json.dumps([release("v0.3.0-nightly.2")]).encode())

    monkeypatch.setattr(update, "_open", fake)
    assert update.check("0.2.0").version == "0.3.0"
    assert update.check("0.3.0-nightly.1").version == "0.3.0-nightly.2"
    assert asked == [update.LATEST, update.RELEASES]

    def none_yet(url, timeout=30):
        raise urllib.error.HTTPError(url, 404, "Not Found", {}, None)

    monkeypatch.setattr(update, "_open", none_yet)
    assert update.check("0.2.0") is None


def test_truncated_reply_is_an_update_error(monkeypatch):
    import http.client

    def truncated(url, timeout=30):
        raise http.client.IncompleteRead(b"[{")

    monkeypatch.setattr(update, "_open", truncated)
    with pytest.raises(update.UpdateError):
        update.check("0.1.0-nightly.1")


def test_only_one_update_installs_at_a_time(monkeypatch):
    import threading
    import tkinter.messagebox

    from speechify.app import App

    monkeypatch.setattr(update, "install_kind", lambda: "appimage")
    started = []
    monkeypatch.setattr(threading, "Thread", lambda target, daemon: types.SimpleNamespace(
        start=lambda: started.append(target)))
    app = Shell()
    app._updating = False
    r = update.Release("0.2.0", "https://example.invalid", {})
    App._offer_update(app, r, confirmed=True)
    App._offer_update(app, r, confirmed=True)
    assert len(started) == 1

    # Another click finishing while the dialog was open.
    app._updating = False

    def answer(*args):
        app._updating = True
        return True

    monkeypatch.setattr(tkinter.messagebox, "askyesno", answer)
    App._offer_update(app, r, confirmed=False)
    assert len(started) == 1
