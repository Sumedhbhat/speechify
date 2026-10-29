"""Exercise the built CLI and native-worker protocol as external callers.

Usage: python3 tests/cli.test.py /absolute/path/to/speechify-tauri-candidate
Inherited LLVM_PROFILE_FILE lets cargo-llvm-cov include these real processes.
"""
import json
import os
from pathlib import Path
import shutil
import re
import subprocess
import sys
import tempfile
import time
import unittest
import wave

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(sys.argv.pop(1)).resolve()


class CliBehavior(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workspace = tempfile.TemporaryDirectory(prefix="speechify-cli-")
        cls.root = Path(cls.workspace.name)
        declarations = (ROOT / "src-tauri/src/ffi.rs").read_text().split("pub fn verify")[0]
        source = cls.root / "native.rs"
        source.write_text(declarations + (ROOT / "tests/fixtures/native.rs").read_text())
        library = "transcribe.dll" if os.name == "nt" else "libtranscribe.dylib" if sys.platform == "darwin" else "libtranscribe.so"
        command = ["rustc", "--edition=2021", "--crate-type", "cdylib", str(source), "-o", str(cls.root / library)]
        if linker := os.environ.get("CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER"):
            command += ["-C", f"linker={linker}"]
        subprocess.run(command, check=True, capture_output=True)
        cls.worker_peer = cls.root / ("peer.exe" if os.name == "nt" else "peer")
        command = ["rustc", "--edition=2021", str(ROOT / "tests/fixtures/worker.rs"), "-o", str(cls.worker_peer)]
        if linker := os.environ.get("CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER"):
            command += ["-C", f"linker={linker}"]
        subprocess.run(command, check=True, capture_output=True)
        cls.wav = cls.root / "speech.wav"
        with wave.open(str(cls.wav), "wb") as wav:
            wav.setnchannels(1)
            wav.setsampwidth(2)
            wav.setframerate(16000)
            wav.writeframes(b"\x00\x10" * 160)
        cls.display = None
        cls.xserver = None
        if sys.platform == "linux":
            cls.xserver = subprocess.Popen(
                [os.environ.get("SPEECHIFY_TEST_XVFB", "Xvfb"), "-displayfd", "1", "-screen", "0", "800x600x24", "-nolisten", "tcp"],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            )
            number = cls.xserver.stdout.readline().strip()
            if not number:
                raise RuntimeError(f"Xvfb failed: {cls.xserver.stderr.read()}")
            cls.display = f":{number}"

    @classmethod
    def tearDownClass(cls):
        if cls.xserver:
            cls.xserver.terminate()
            cls.xserver.wait(timeout=5)
            cls.xserver.stdout.close()
            cls.xserver.stderr.close()
        cls.workspace.cleanup()

    def environment(self):
        env = {**os.environ, "SPEECHIFY_CANDIDATE_PARENT": str(self.root / self._testMethodName)}
        if self.display:
            env["DISPLAY"] = self.display
            env["XDG_SESSION_TYPE"] = "x11"
            env["GDK_BACKEND"] = "x11"
            env.pop("WAYLAND_DISPLAY", None)
        return env

    def run_cli(self, *args, text="", extra_env=None):
        env = self.environment()
        for key, value in (extra_env or {}).items():
            if value is None:
                env.pop(key, None)
            else:
                env[key] = str(value)
        return subprocess.run([str(BINARY), *map(str, args)], input=text, capture_output=True, text=True, timeout=10, env=env)

    def worker(self, model="plain", device="cpu", messages=None):
        messages = messages if messages is not None else [{"wav": str(self.wav), "language": "en", "prompt": "GitHub"}]
        return self.run_cli("--native-worker", self.root, model, device, "2", text="".join(json.dumps(m) + "\n" for m in messages))

    def test_diagnostics_reports_the_os_and_permission_guidance(self):
        result = self.run_cli("diagnostics")
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertTrue(report["os"])
        self.assertTrue(report["guidance"])

    def test_invalid_commands_fail_with_actionable_usage(self):
        for args, message in [
            (("--native-worker",), "native-worker requires"),
            (("--native-worker", self.root, "model", "cpu", "many"), "Invalid thread count"),
            (("fixture",), "fixture MODEL_ID"),
            (("install-model",), "install-model MODEL_ID"),
            (("install-model", "unknown"), "Unknown catalog model"),
        ]:
            with self.subTest(args=args):
                result = self.run_cli(*args)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(message, result.stderr)

    def test_fixture_transcribes_repeatedly_without_adding_to_history(self):
        data = self.root / self._testMethodName / "speechify-tauri-candidate"
        model = data / "models/whisper-ft-cli"
        model.mkdir(parents=True)
        (model / "model.bin").write_text("fixture")
        python = data / ("environments/whisper/Scripts/python.exe" if os.name == "nt" else "environments/whisper/bin/python")
        python.parent.mkdir(parents=True)
        shutil.copy2(self.worker_peer, python)
        for extra, count in [((), 1), (("2", "cpu"), 2), (("invalid",), 1)]:
            result = self.run_cli("fixture", "ft:whisper-ft-cli", self.wav, *extra)
            self.assertEqual(result.returncode, 0, result.stderr)
            rows = list(map(json.loads, result.stdout.splitlines()))
            self.assertEqual(len(rows), count)
            self.assertEqual([r["iteration"] for r in rows], list(range(count)))
            self.assertTrue(all(r["result"]["text"] == "get hub" for r in rows))
        import sqlite3
        with sqlite3.connect(data / "learning.db") as db:
            self.assertEqual(db.execute("SELECT count(*) FROM samples").fetchone()[0], 0)
        self.assertTrue((data / "config.toml").is_file())

    def test_data_root_override_is_private_and_does_not_touch_legacy_speechify(self):
        result = self.run_cli("install-model", "unknown")
        self.assertNotEqual(result.returncode, 0)
        parent = self.root / self._testMethodName
        data = parent / "speechify-tauri-candidate"
        self.assertEqual(sorted(p.name for p in data.iterdir()), ["audio", "exports", "logs", "models"])
        self.assertFalse((parent / "speechify").exists())
        if os.name != "nt":
            self.assertEqual(data.stat().st_mode & 0o777, 0o700)
        result = self.run_cli("install-model", "unknown", extra_env={"HOME": None, "APPDATA": None, "XDG_DATA_HOME": None})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("No user data directory", result.stderr)

    def test_unwritable_data_location_is_reported_as_an_error(self):
        occupied = self.root / "occupied"
        occupied.write_text("file")
        result = self.run_cli("install-model", "unknown", extra_env={"SPEECHIFY_CANDIDATE_PARENT": occupied})
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(occupied.read_text(), "file")

    def test_worker_keeps_serving_after_a_bad_clip(self):
        result = self.worker(messages=[{}, {"wav": "/does/not/exist.wav"}, {"wav": str(self.wav)}])
        self.assertEqual(result.returncode, 0, result.stderr)
        ready, missing, absent, transcript = map(json.loads, result.stdout.splitlines())
        self.assertTrue(ready["ready"])
        self.assertEqual(missing["error"], "Missing WAV")
        self.assertIn("error", absent)
        self.assertTrue(transcript["text"].startswith("160|en|"))
        self.assertEqual(transcript["device"], "cpu")

    def test_worker_retries_failed_gpu_loading_and_inference_on_cpu(self):
        for model in ["gpu-load-fails", "gpu-run-fails"]:
            with self.subTest(model=model):
                result = self.worker(model, "vulkan")
                self.assertEqual(result.returncode, 0, result.stderr)
                rows = list(map(json.loads, result.stdout.splitlines()))
                self.assertEqual(rows[-1]["device"], "cpu")
                self.assertTrue(rows[-1]["text"].startswith("160|en|"))

    def test_worker_reports_native_errors_and_rejects_malformed_messages(self):
        for device in ["cpu", "vulkan"]:
            result = self.worker("bad-load", device)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("21", result.stderr)
        result = self.worker("bad-run")
        self.assertEqual(json.loads(result.stdout.splitlines()[-1])["error"], "transcribe.cpp status 26")
        for text, message in [("invalid\n", "expected value"), ("x" * 32769 + "\n", "Worker message too large")]:
            result = self.run_cli("--native-worker", self.root, "plain", "cpu", "2", text=text)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(message, result.stderr)

    def write_clipboard(self, text, target="UTF8_STRING"):
        subprocess.run(["xclip", "-selection", "clipboard", "-i", "-target", target],
                       input=text.encode(), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                       env=self.environment(), check=True, timeout=5)

    def read_clipboard(self, target="UTF8_STRING"):
        return subprocess.run(["xclip", "-selection", "clipboard", "-o", "-target", target],
                              capture_output=True, env=self.environment(), timeout=5)

    def clipboard_worker(self, text):
        import select
        worker = subprocess.Popen([str(BINARY), "--clipboard-worker"], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=self.environment(), text=True)
        def close_worker():
            if worker.poll() is None:
                worker.kill()
            worker.wait(timeout=5)
            worker.stdin.close()
            worker.stdout.close()
            worker.stderr.close()
        self.addCleanup(close_worker)
        worker.stdin.write(json.dumps(text) + "\n")
        worker.stdin.flush()
        self.assertTrue(select.select([worker.stdout], [], [], 10)[0], "Clipboard worker did not respond")
        ready = json.loads(worker.stdout.readline())
        return worker, ready

    @unittest.skipUnless(sys.platform == "linux", "X11 clipboard contract")
    def test_clipboard_restores_rich_content_after_the_dictation_is_consumed(self):
        original = "<b>Earlier copied text & café</b>"
        self.write_clipboard(original, "text/html")
        worker, ready = self.clipboard_worker("A new dictation 🦀")
        self.assertEqual(ready, {"Ok": None})
        self.assertEqual(self.read_clipboard().stdout.decode(), "A new dictation 🦀")
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            restored = self.read_clipboard("text/html")
            if restored.returncode == 0:
                break
            time.sleep(.02)
        self.assertEqual(restored.stdout.decode(), original)
        self.write_clipboard("Next copy")
        self.assertEqual(worker.wait(timeout=5), 0)

    @unittest.skipUnless(sys.platform == "linux", "X11 clipboard contract")
    def test_new_copy_of_identical_text_is_never_overwritten_by_restoration(self):
        self.write_clipboard("Old clipboard")
        worker, ready = self.clipboard_worker("Dictation")
        self.assertEqual(ready, {"Ok": None})
        self.write_clipboard("Dictation")
        self.assertEqual(worker.wait(timeout=5), 0)
        time.sleep(.55)
        self.assertEqual(self.read_clipboard().stdout.decode(), "Dictation")

    @unittest.skipUnless(sys.platform == "linux", "X11 clipboard contract")
    def test_unsupported_or_oversized_clipboard_refuses_paste_and_preserves_the_copy(self):
        for target, original, reason in [("application/x-private-handle", "private", "cannot be preserved"),
                                         ("text/html", "x" * (128 * 1024 + 1), "128 KiB")]:
            with self.subTest(target=target):
                self.write_clipboard(original, target)
                worker, ready = self.clipboard_worker("Must not replace the clipboard")
                self.assertIn(reason, ready["Err"])
                self.assertNotEqual(worker.wait(timeout=5), 0)
                self.assertEqual(self.read_clipboard(target).stdout.decode(), original)

    @unittest.skipUnless(sys.platform == "linux", "X11 clipboard contract")
    def test_clipboard_worker_reports_a_missing_display_and_malformed_input(self):
        result = self.run_cli("--clipboard-worker", text=json.dumps("dictation") + "\n", extra_env={"DISPLAY": ":65530"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Cannot open clipboard display", json.loads(result.stdout)["Err"])
        result = self.run_cli("--clipboard-worker", text="invalid\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("expected value", result.stderr)

    @unittest.skipUnless(sys.platform == "linux", "X11 clipboard contract")
    def test_empty_clipboard_is_restored_to_empty_after_transient_dictation(self):
        import ctypes
        x = ctypes.CDLL("libX11.so.6")
        x.XOpenDisplay.argtypes = [ctypes.c_char_p]
        x.XOpenDisplay.restype = ctypes.c_void_p
        x.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
        x.XInternAtom.restype = ctypes.c_ulong
        x.XSetSelectionOwner.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong]
        x.XFlush.argtypes = [ctypes.c_void_p]
        x.XCloseDisplay.argtypes = [ctypes.c_void_p]
        display = x.XOpenDisplay(self.display.encode())
        self.assertTrue(display)
        try:
            x.XSetSelectionOwner(display, x.XInternAtom(display, b"CLIPBOARD", 0), 0, 0)
            x.XFlush(display)
        finally:
            x.XCloseDisplay(display)
        worker, ready = self.clipboard_worker("Transient dictation")
        self.assertEqual(ready, {"Ok": None})
        self.assertEqual(self.read_clipboard().stdout.decode(), "Transient dictation")
        time.sleep(.6)
        self.assertNotEqual(self.read_clipboard().returncode, 0)
        self.write_clipboard("Next copy")
        self.assertEqual(worker.wait(timeout=5), 0)

    @unittest.skipUnless(sys.platform == "linux", "Desktop lifecycle on an isolated X11/D-Bus session")
    def test_desktop_survives_closing_settings_reopens_from_tray_and_quits_cleanly(self):
        bus = subprocess.check_output(["dbus-daemon", "--session", "--fork", "--print-address=1", "--print-pid=1"], text=True).splitlines()
        self.addCleanup(lambda: os.kill(int(bus[1]), 15))
        env = self.environment()
        env.update(DBUS_SESSION_BUS_ADDRESS=bus[0], LIBGL_ALWAYS_SOFTWARE="1", WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS="1")
        # Restrict capture to a synthetic ALSA device, including if the test host
        # has a real microphone attached.
        alsa = self.root / "test-alsa.conf"
        alsa.write_text('pcm.!default { type null }\n')
        env["ALSA_CONFIG_PATH"] = str(alsa)
        (ROOT / "coverage").mkdir(exist_ok=True)
        log = open(ROOT / "coverage/desktop-startup.log", "w+")
        self.addCleanup(log.close)
        app = subprocess.Popen([str(BINARY)], stdin=subprocess.DEVNULL, stdout=log, stderr=log, env=env)
        def close_app():
            if app.poll() is None:
                app.kill()
            app.wait(timeout=5)
        self.addCleanup(close_app)

        def wait(predicate):
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                value = predicate()
                if value:
                    return value
                if app.poll() is not None:
                    log.seek(0)
                    self.fail(f"Desktop exited before completing its lifecycle: {log.read()}")
                time.sleep(.02)
            log.seek(0)
            self.fail(f"Desktop behavior did not settle: {log.read()}")

        def windows(visible=False):
            args = ["xdotool", "search"] + (["--onlyvisible"] if visible else []) + ["--name", "^Speechify Tauri Candidate$"]
            found = subprocess.run(args, capture_output=True, text=True, env=env)
            return found.stdout.split()

        window = wait(lambda: windows(True))[0]
        def dbus(*args):
            result = subprocess.run(["gdbus", *args], capture_output=True, text=True, env=env, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            return result.stdout
        def find_menu():
            names = dbus("call", "--session", "--dest", "org.freedesktop.DBus", "--object-path", "/org/freedesktop/DBus", "--method", "org.freedesktop.DBus.ListNames")
            for name in re.findall("'([^']+)'", names):
                process = subprocess.run(["gdbus", "call", "--session", "--dest", "org.freedesktop.DBus", "--object-path", "/org/freedesktop/DBus", "--method", "org.freedesktop.DBus.GetConnectionUnixProcessID", name], capture_output=True, text=True, env=env, timeout=5)
                if process.returncode or not re.search(r"\b" + str(app.pid) + r"\b", process.stdout):
                    continue
                # Single-instance and tray connections share a PID. Discover
                # the connection that actually exports a menu interface.
                tree = subprocess.run(["gdbus", "introspect", "--session", "--dest", name, "--object-path", "/", "--recurse"], capture_output=True, text=True, env=env, timeout=5)
                if tree.returncode:
                    continue
                match = re.search("readonly o Menu = '([^']+)'", tree.stdout)
                if match:
                    return name, match.group(1)
            return None
        menu_bus, menu_path = wait(find_menu)
        def menu(method, *args):
            return dbus("call", "--session", "--dest", menu_bus, "--object-path", menu_path, "--method", f"com.canonical.dbusmenu.{method}", "--", *args)
        layout = menu("GetLayout", "0", "-1", "[]")
        entries = {label: number for number, label in re.findall(r"\((\d+),[^{}]*\{[^{}]*'label': <'([^']+)'>", layout)}
        self.assertEqual(set(entries), {"Settings", "History", "Models", "Log location", "Quit candidate"})
        def activate(label):
            menu("Event", entries[label], "clicked", "<0>", "0")

        def close_settings():
            import ctypes
            x = ctypes.CDLL("libX11.so.6")
            x.XOpenDisplay.argtypes = [ctypes.c_char_p]
            x.XOpenDisplay.restype = ctypes.c_void_p
            x.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
            x.XInternAtom.restype = ctypes.c_ulong
            x.XSendEvent.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_long, ctypes.c_void_p]
            x.XCloseDisplay.argtypes = [ctypes.c_void_p]
            class Message(ctypes.Structure):
                _fields_ = [("type", ctypes.c_int), ("serial", ctypes.c_ulong), ("send", ctypes.c_int),
                            ("display", ctypes.c_void_p), ("window", ctypes.c_ulong),
                            ("message_type", ctypes.c_ulong), ("format", ctypes.c_int), ("data", ctypes.c_long * 5)]
            class Event(ctypes.Union):
                _fields_ = [("message", Message), ("pad", ctypes.c_long * 24)]
            display = x.XOpenDisplay(self.display.encode())
            self.assertTrue(display)
            try:
                event = Event()
                event.message.type = 33
                event.message.send = 1
                event.message.display = display
                event.message.window = int(window)
                event.message.message_type = x.XInternAtom(display, b"WM_PROTOCOLS", 0)
                event.message.format = 32
                event.message.data[0] = x.XInternAtom(display, b"WM_DELETE_WINDOW", 0)
                self.assertEqual(x.XSendEvent(display, int(window), 0, 0, ctypes.byref(event)), 1)
            finally:
                x.XCloseDisplay(display)

        for page in ["History", "Models", "Log location", "Settings"]:
            close_settings()
            wait(lambda: not windows(True))
            self.assertIsNone(app.poll(), "Closing settings stopped background dictation")
            activate(page)
            wait(lambda: windows(True))
        close_settings()
        wait(lambda: not windows(True))
        second = subprocess.run([str(BINARY)], capture_output=True, env=env, timeout=10)
        self.assertEqual(second.returncode, 0, second.stderr)
        wait(lambda: windows(True))

        # A short global talk-key tap must never create a dictation or audio.
        subprocess.run(["xdotool", "keydown", "Control_R"], check=True, env=env)
        time.sleep(.06)
        subprocess.run(["xdotool", "keyup", "Control_R"], check=True, env=env)
        time.sleep(.3)
        import sqlite3
        data = Path(env["SPEECHIFY_CANDIDATE_PARENT"]) / "speechify-tauri-candidate"
        with sqlite3.connect(data / "learning.db") as db:
            self.assertEqual(db.execute("SELECT count(*) FROM samples").fetchone()[0], 0)
        self.assertEqual(list((data / "audio").iterdir()), [])
        activate("Quit candidate")
        self.assertEqual(app.wait(timeout=10), 0)


if __name__ == "__main__":
    unittest.main()
