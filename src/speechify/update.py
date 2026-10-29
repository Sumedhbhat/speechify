"""Updating the installed app from GitHub releases, from the tray.

check() asks GitHub for the newest release on the channel this build came
from: a nightly (or dev) build follows nightlies and stable releases, a
stable build only stable ones (docs/releases.md). download() fetches the
installer for this kind of install and checks it against the release's
SHA256SUMS. install() puts it in place and starts the new version once this
one has quit:

    Windows   the Inno Setup installer, silently, for the same user or all
              users as before; it closes Speechify and starts the new one
    macOS     the .app from the .dmg, copied over the running one's bundle
    AppImage  the new AppImage, renamed over the running one's file
    .deb      apt-get through pkexec, the desktop's own password prompt

Your data folder is never touched, the same as installing by hand.
"""

from __future__ import annotations

import hashlib
import http.client
import json
import os
import platform
import re
import shutil
import ssl
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

from . import __version__

REPO = "Sumedhbhat/speechify"
# SPEECHIFY_UPDATE_API points at a stand-in for GitHub's API, for testing
# updates end to end (scripts/fake_release.py, the check-update CI job).
API = os.environ.get("SPEECHIFY_UPDATE_API") or f"https://api.github.com/repos/{REPO}"
RELEASES = f"{API}/releases?per_page=100"
# Stable builds also ask for this: 100 nightlies in a row would push every
# stable release off RELEASES. It alone isn't enough either: a patch for an
# older line (0.1.5 after 0.2.0) becomes GitHub's latest release.
LATEST = f"{API}/releases/latest"
_VERSION = re.compile(r"^v?(\d+)\.(\d+)\.(\d+)(?:-(nightly|dev)\.(\d+))?$")


class UpdateError(RuntimeError):
    pass


# The SHA-256 of each file download() checked, so a .deb can be checked again
# where only root can change it (_install_deb).
_verified: dict[Path, str] = {}


def version_key(version: str) -> tuple[int, ...] | None:
    """Sorts versions: 0.1.0-nightly.9 < 0.1.0-nightly.10 < 0.1.0 < 0.2.0-nightly.1.
    None for a version that isn't ours (e.g. a source checkout's +unknown)."""
    m = _VERSION.match(version.strip())
    if not m:
        return None
    major, minor, patch, pre, n = m.groups()
    return (int(major), int(minor), int(patch), 0 if pre else 1, int(n or 0))


def is_stable(version: str) -> bool:
    return "-" not in version


@dataclass
class Release:
    version: str
    url: str  # the release page
    assets: dict[str, str]  # file name -> download URL


def newest(releases: list[dict], current: str = __version__) -> Release | None:
    """The newest release in `releases` (GitHub's JSON) newer than `current`,
    on current's channel."""
    mine = version_key(current)
    if mine is None:
        return None
    best: tuple[tuple[int, ...], dict] | None = None
    for r in releases:
        if r.get("draft"):
            continue
        key = version_key(r.get("tag_name", ""))
        if key is None or key <= mine:
            continue
        if is_stable(current) and (r.get("prerelease") or key[3] == 0):
            continue
        if best is None or key > best[0]:
            best = (key, r)
    if best is None:
        return None
    r = best[1]
    return Release(
        version=r["tag_name"].removeprefix("v"),
        url=r.get("html_url") or f"https://github.com/{REPO}/releases/tag/{r['tag_name']}",
        assets={a["name"]: a["browser_download_url"] for a in r.get("assets", [])},
    )


def _ssl_context() -> ssl.SSLContext:
    context = ssl.create_default_context()
    # The app's Python looks for certificates where OpenSSL is usually
    # installed, which isn't where macOS keeps them.
    if sys.platform == "darwin" and Path("/etc/ssl/cert.pem").exists():
        context.load_verify_locations("/etc/ssl/cert.pem")
    return context


def _open(url: str, timeout: float = 30):
    request = urllib.request.Request(url, headers={
        "User-Agent": f"speechify/{__version__}",
        "Accept": "application/vnd.github+json" if "api.github.com" in url else "*/*",
    })
    return urllib.request.urlopen(request, timeout=timeout, context=_ssl_context())


def check(current: str = __version__) -> Release | None:
    """The release to update to, or None if this is the newest. Raises
    UpdateError if GitHub can't be reached."""
    releases = _get(RELEASES)
    if is_stable(current):
        latest = _get(LATEST, missing_ok=True)  # 404: no stable release yet
        if latest:
            releases.append(latest)
    return newest(releases, current)


def _get(url: str, missing_ok: bool = False):
    try:
        with _open(url, timeout=15) as resp:
            return json.load(resp)
    except urllib.error.HTTPError as exc:
        if missing_ok and exc.code == 404:
            return None
        raise UpdateError(f"Couldn't reach GitHub to check for updates: {exc}") from exc
    except (OSError, ValueError, http.client.HTTPException) as exc:
        raise UpdateError(f"Couldn't reach GitHub to check for updates: {exc}") from exc


# --- what kind of install this is ---------------------------------------------


def install_kind() -> str | None:
    """"windows", "macos", "appimage" or "deb", or None when Speechify can't
    update itself (running from source, or installed some other way)."""
    if not getattr(sys, "frozen", False):
        return None
    if sys.platform == "win32":
        return "windows"
    if sys.platform == "darwin":
        return "macos" if _app_bundle() is not None else None
    if os.environ.get("APPIMAGE"):
        return "appimage"
    if Path(sys.executable).resolve().parent == Path("/opt/speechify") and shutil.which("dpkg"):
        return "deb"
    return None


def why_not() -> str:
    """Why Speechify can't update itself here, for a dialog."""
    if not getattr(sys, "frozen", False):
        return "Speechify is running from source; update it with git pull."
    return "This copy of Speechify wasn't installed from a release, so it can't update itself."


def _app_bundle() -> Path | None:
    """Speechify.app, from .../Speechify.app/Contents/MacOS/Speechify."""
    exe = Path(sys.executable).resolve()
    bundle = exe.parents[2] if len(exe.parents) > 2 else None
    return bundle if bundle is not None and bundle.suffix == ".app" else None


def asset_name(release: Release, kind: str) -> str:
    """The file in `release` for this kind of install."""
    suffix = {
        "windows": "-windows-x64-setup.exe",
        "macos": f"-macos-{platform.machine()}.dmg",
        "appimage": f"-{platform.machine()}.AppImage",
        "deb": "_amd64.deb",
    }[kind]
    for name in release.assets:
        if name.endswith(suffix):
            return name
    raise UpdateError(f"Speechify {release.version} has no {suffix.lstrip('-_')} download.")


# --- downloading ----------------------------------------------------------------


def _checksums(release: Release) -> dict[str, str]:
    url = release.assets.get("SHA256SUMS")
    if not url:
        raise UpdateError(f"Speechify {release.version} has no SHA256SUMS to check the download against.")
    with _open(url) as resp:
        text = resp.read().decode()
    sums = {}
    for line in text.splitlines():
        parts = line.split()
        if len(parts) == 2:
            # GitHub renames uploads: the .deb's 0.2.0~nightly.5 becomes 0.2.0.nightly.5.
            sums[parts[1].lstrip("*").replace("~", ".")] = parts[0].lower()
    return sums


def download(release: Release, kind: str, folder: Path,
             progress: Callable[[float], None] = lambda fraction: None) -> Path:
    """Download this install's file into `folder` and check its SHA-256.
    `progress` gets 0 to 1 as it goes."""
    name = asset_name(release, kind)
    try:
        expected = _checksums(release).get(name.replace("~", "."))
        if not expected:
            raise UpdateError(f"SHA256SUMS doesn't list {name}.")
        target = folder / name
        digest = hashlib.sha256()
        with _open(release.assets[name]) as resp, open(target, "wb") as out:
            total = int(resp.headers.get("Content-Length") or 0)
            done = 0
            while chunk := resp.read(1 << 20):
                out.write(chunk)
                digest.update(chunk)
                done += len(chunk)
                if total:
                    progress(done / total)
    except (OSError, http.client.HTTPException) as exc:
        raise UpdateError(f"The download failed: {exc}") from exc
    if digest.hexdigest() != expected:
        target.unlink(missing_ok=True)
        raise UpdateError(f"{name} doesn't match its checksum, so it wasn't installed. Try again later.")
    _verified[target] = expected
    return target


def download_folder(kind: str) -> Path:
    if kind == "appimage":
        # Next to the AppImage, so the new one can be renamed over it.
        return Path(os.environ["APPIMAGE"]).parent
    return Path(tempfile.mkdtemp(prefix="speechify-update-"))


# --- installing -------------------------------------------------------------------


def _clean_env() -> dict[str, str]:
    """The environment for programs we start, without what PyInstaller set
    for this process, so a new Speechify sets itself up from scratch."""
    env = dict(os.environ)
    env["PYINSTALLER_RESET_ENVIRONMENT"] = "1"
    if "LD_LIBRARY_PATH_ORIG" in env:
        env["LD_LIBRARY_PATH"] = env.pop("LD_LIBRARY_PATH_ORIG")
    else:
        env.pop("LD_LIBRARY_PATH", None)
    for name in ("APPIMAGE", "APPDIR", "ARGV0", "OWD"):  # the AppImage runtime sets these again
        env.pop(name, None)
    return env


def _detached(cmd: list[str]) -> None:
    kwargs: dict = {"env": _clean_env(), "stdin": subprocess.DEVNULL, "close_fds": True}
    if sys.platform == "win32":
        kwargs["creationflags"] = subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_PROCESS_GROUP
    else:
        kwargs["start_new_session"] = True
    subprocess.Popen(cmd, **kwargs)


# Waits for the Speechify with pid $1 to quit, runs $2 (shell, may be empty),
# then starts the rest. Speechify only allows one copy at a time (instance.py),
# so the new one can't start before the old one is gone.
_AFTER_EXIT = 'while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; pid=$1; shift; sh -c "$1"; shift; exec "$@"'


def _after_exit(then: str, cmd: list[str]) -> None:
    _detached(["/bin/sh", "-c", _AFTER_EXIT, "sh", str(os.getpid()), then, *cmd])


def install(path: Path, kind: str, restart: bool = True) -> None:
    """Install the downloaded `path`, finishing once this process has quit,
    and with `restart`, start the new version then. The caller quits right
    after this returns."""
    if kind == "windows":
        _install_windows(path, restart)
    elif kind == "macos":
        _install_macos(path, restart)
    elif kind == "appimage":
        _install_appimage(path, restart)
    elif kind == "deb":
        _install_deb(path, restart)
    else:
        raise UpdateError(why_not())


def _install_windows(setup: Path, restart: bool) -> None:
    app = Path(sys.executable).resolve().parent
    local = Path(os.environ.get("LOCALAPPDATA") or Path.home() / "AppData" / "Local")
    # Installed for everyone (Program Files) needs the admin prompt again.
    scope = "/CURRENTUSER" if str(app).lower().startswith(str(local.resolve()).lower()) else "/ALLUSERS"
    # /RELAUNCH=1 starts the new Speechify when it's done (speechify.iss).
    _detached([str(setup), "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", scope]
              + (["/RELAUNCH=1"] if restart else []))


def _install_macos(dmg: Path, restart: bool) -> None:
    bundle = _app_bundle()
    if bundle is None:
        raise UpdateError(why_not())
    if not os.access(bundle.parent, os.W_OK):
        raise UpdateError(f"Speechify can't write to {bundle.parent}. Open the download and drag "
                          f"Speechify into Applications yourself:\n{dmg}")
    staged = bundle.with_name(".Speechify-update.app")
    mount = Path(tempfile.mkdtemp(prefix="speechify-dmg-"))
    shutil.rmtree(staged, ignore_errors=True)
    try:
        subprocess.run(["hdiutil", "attach", "-nobrowse", "-readonly", "-mountpoint", str(mount), str(dmg)],
                       check=True, capture_output=True)
        try:
            subprocess.run(["ditto", str(mount / "Speechify.app"), str(staged)], check=True, capture_output=True)
        finally:
            subprocess.run(["hdiutil", "detach", "-force", str(mount)], capture_output=True)
    except subprocess.CalledProcessError as exc:
        shutil.rmtree(staged, ignore_errors=True)
        raise UpdateError(f"Couldn't unpack {dmg.name}: {exc.stderr.decode(errors='replace').strip()}") from exc
    # Swapped once this one has quit, so nothing runs from a moved bundle. The
    # old bundle is only deleted once the new one is in place.
    old = bundle.with_name(".Speechify-previous.app")
    swap = (f"rm -rf '{_sh(old)}' && mv '{_sh(bundle)}' '{_sh(old)}' && "
            f"if mv '{_sh(staged)}' '{_sh(bundle)}'; then rm -rf '{_sh(old)}'; "
            f"else mv '{_sh(old)}' '{_sh(bundle)}'; fi")
    _after_exit(swap, ["open", str(bundle)] if restart else [])


def _install_appimage(new: Path, restart: bool) -> None:
    current = Path(os.environ["APPIMAGE"])
    new.chmod(0o755)
    # Same name as before, so the app menu entry, start at sign-in and any
    # shortcut you made keep working. The running copy is mounted, so it
    # keeps going until it quits.
    try:
        os.replace(new, current)
    except OSError as exc:
        new.unlink(missing_ok=True)
        raise UpdateError(f"Couldn't replace {current}: {exc}") from exc
    if restart:
        _after_exit("", [str(current)])


# Run as root with the downloaded .deb as $1 and its SHA-256 as $2. The
# download is in a folder you own, so another program running as you could
# swap it while the password prompt is open; root copies it somewhere only
# root can write, checks that copy and installs it. 0755 so apt's sandbox
# user can read it. --allow-downgrades: going from a nightly to a stable
# build is fine; apt sorts 0.2.0~nightly.5 before 0.2.0 anyway.
_DEB_SCRIPT = """set -e
dir=$(mktemp -d /tmp/speechify-update.XXXXXX)
trap 'rm -rf "$dir"' EXIT
chmod 755 "$dir"
cp -- "$1" "$dir/speechify.deb"
chmod 644 "$dir/speechify.deb"
echo "$2  $dir/speechify.deb" | sha256sum -c --status - || exit 3
if command -v apt-get >/dev/null; then
    apt-get install -y --allow-downgrades "$dir/speechify.deb"
else
    dpkg -i "$dir/speechify.deb"
fi
"""


def _install_deb(deb: Path, restart: bool) -> None:
    root = os.geteuid() == 0  # `sudo speechify update` needs no password prompt
    if not root and not shutil.which("pkexec"):
        raise UpdateError(f"Installing a .deb needs your password, and pkexec isn't installed. "
                          f"Run this instead:\n    sudo apt install {deb}")
    if deb not in _verified:
        raise UpdateError(f"{deb.name} wasn't downloaded and checked by Speechify, so it wasn't installed.")
    cmd = ["/bin/sh", "-c", _DEB_SCRIPT, "sh", str(deb), _verified[deb]]
    done = subprocess.run(cmd if root else ["pkexec", *cmd], env=_clean_env(), capture_output=True, text=True)
    if done.returncode == 126 and not root:
        raise UpdateError("Update cancelled.")
    if done.returncode == 3:
        raise UpdateError(f"{deb.name} changed after it was downloaded, so it wasn't installed. Try again.")
    if done.returncode != 0:
        print(done.stdout + done.stderr)
        raise UpdateError(f"apt couldn't install the update (see the log):\n{done.stderr.strip()[-400:]}")
    if restart:
        _after_exit("", ["/opt/speechify/speechify"])


def _sh(path: Path) -> str:
    """`path` for inside single quotes in sh."""
    return str(path).replace("'", "'\\''")
