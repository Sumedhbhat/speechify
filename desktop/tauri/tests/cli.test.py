"""Exercise the built CLI and native-worker protocol as external callers.

Usage: python3 tests/cli.test.py /absolute/path/to/speechify-tauri-candidate
Inherited LLVM_PROFILE_FILE lets cargo-llvm-cov include these real processes.
"""

import json
from contextlib import closing
import os
import queue
from pathlib import Path
import shutil
import re
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import wave

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(sys.argv.pop(1)).resolve()


def read_line(process, timeout=10):
    """Bound complete protocol lines on both Windows and POSIX pipes."""
    result = queue.Queue(maxsize=1)

    def read():
        try:
            result.put((True, process.stdout.readline()))
        except Exception as error:
            result.put((False, error))

    threading.Thread(target=read, daemon=True).start()
    try:
        success, value = result.get(timeout=timeout)
    except queue.Empty:
        process.kill()
        process.wait(timeout=5)
        raise subprocess.TimeoutExpired(process.args, timeout) from None
    if not success:
        raise value
    return value


class CliBehavior(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workspace = tempfile.TemporaryDirectory(prefix="speechify-cli-")
        cls.root = Path(cls.workspace.name)
        declarations = (ROOT / "src-tauri/src/ffi.rs").read_text().split("pub fn verify")[0]
        source = cls.root / "native.rs"
        source.write_text(declarations + (ROOT / "tests/fixtures/native.rs").read_text())
        library = (
            "transcribe.dll"
            if os.name == "nt"
            else "libtranscribe.dylib"
            if sys.platform == "darwin"
            else "libtranscribe.so"
        )
        command = [
            "rustc",
            "--edition=2021",
            "--crate-type",
            "cdylib",
            str(source),
            "-o",
            str(cls.root / library),
        ]
        if linker := os.environ.get("CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER"):
            command += ["-C", f"linker={linker}"]
        subprocess.run(command, check=True, capture_output=True, timeout=300)
        cls.worker_peer = cls.root / ("peer.exe" if os.name == "nt" else "peer")
        command = [
            "rustc",
            "--edition=2021",
            str(ROOT / "tests/fixtures/worker.rs"),
            "-o",
            str(cls.worker_peer),
        ]
        if linker := os.environ.get("CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER"):
            command += ["-C", f"linker={linker}"]
        subprocess.run(command, check=True, capture_output=True, timeout=300)
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
                [
                    os.environ.get("SPEECHIFY_TEST_XVFB", "Xvfb"),
                    "-displayfd",
                    "1",
                    "-screen",
                    "0",
                    "800x600x24",
                    "-nolisten",
                    "tcp",
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            number = read_line(cls.xserver, timeout=15).strip()
            if not number:
                cls.xserver.kill()
                cls.xserver.wait(timeout=5)
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
        return subprocess.run(
            [str(BINARY), *map(str, args)],
            input=text,
            capture_output=True,
            text=True,
            timeout=10,
            env=env,
        )

    def worker(self, model="plain", device="cpu", messages=None):
        messages = (
            messages
            if messages is not None
            else [{"wav": str(self.wav), "language": "en", "prompt": "GitHub"}]
        )
        return self.run_cli(
            "--native-worker",
            self.root,
            model,
            device,
            "2",
            text="".join(json.dumps(m) + "\n" for m in messages),
        )

    def test_a_silent_protocol_helper_times_out_and_is_terminated(self):
        for output in ["", "partial line"]:
            with self.subTest(output=output):
                script = f"import time; print({output!r}, end='', flush=True); time.sleep(60)"
                helper = subprocess.Popen(
                    [sys.executable, "-c", script],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                )
                try:
                    with self.assertRaises(subprocess.TimeoutExpired):
                        read_line(helper, timeout=0.1)
                    self.assertIsNotNone(helper.poll())
                finally:
                    if helper.poll() is None:
                        helper.kill()
                    helper.wait(timeout=5)
                    helper.stdout.close()
                    helper.stderr.close()

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
        python = data / (
            "environments/whisper/Scripts/python.exe"
            if os.name == "nt"
            else "environments/whisper/bin/python"
        )
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

        with closing(sqlite3.connect(data / "learning.db")) as db:
            self.assertEqual(db.execute("SELECT count(*) FROM samples").fetchone()[0], 0)
        self.assertTrue((data / "config.toml").is_file())

    def test_native_model_fixture_uses_a_supervised_child_without_adding_history(self):
        data = self.root / self._testMethodName / "speechify-tauri-candidate"
        models = data / "models"
        models.mkdir(parents=True)
        shutil.copyfile(
            ROOT / ".build/fixtures/moonshine-base.gguf", models / "moonshine-base.gguf"
        )
        result = self.run_cli(
            "fixture",
            "moonshine-base",
            self.wav,
            "2",
            "cpu",
            extra_env={"SPEECHIFY_NATIVE_DIR": self.root},
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        rows = list(map(json.loads, result.stdout.splitlines()))
        self.assertEqual([row["iteration"] for row in rows], [0, 1])
        self.assertEqual([row["result"]["text"] for row in rows], ["160|en|||pnc=2"] * 2)
        self.assertTrue(all(row["result"]["device"] == "cpu" for row in rows))
        import sqlite3

        with closing(sqlite3.connect(data / "learning.db")) as db:
            self.assertEqual(db.execute("SELECT count(*) FROM samples").fetchone()[0], 0)

    @unittest.skipUnless(os.name != "nt", "Unix native-worker parent watchdog")
    def test_native_worker_exits_when_its_parent_disappears_even_with_stdin_still_open(self):
        reader, writer = os.pipe()
        helper = r"""
import json,os,subprocess,sys
child=subprocess.Popen([sys.argv[1],"--native-worker",sys.argv[2],"plain","cpu","2"],
    stdin=int(sys.argv[3]),stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,start_new_session=True,text=True)
ready=json.loads(child.stdout.readline())
print(json.dumps({"pid":child.pid,"ready":ready}),flush=True)
os._exit(0)
"""
        try:
            parent = subprocess.run(
                [sys.executable, "-c", helper, str(BINARY), str(self.root), str(reader)],
                pass_fds=(reader,),
                capture_output=True,
                text=True,
                timeout=5,
                env=self.environment(),
            )
            self.assertEqual(parent.returncode, 0, parent.stderr)
            child = json.loads(parent.stdout)
            self.assertTrue(child["ready"]["ready"])
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                status = subprocess.run(
                    ["ps", "-p", str(child["pid"]), "-o", "stat="],
                    capture_output=True,
                    text=True,
                    timeout=2,
                )
                if not status.stdout.strip() or status.stdout.strip().startswith("Z"):
                    break
                time.sleep(0.02)
            else:
                self.fail("Native worker kept running after its parent disappeared")
        finally:
            os.close(reader)
            os.close(writer)

    def test_data_root_override_is_private_and_does_not_touch_legacy_speechify(self):
        result = self.run_cli("install-model", "unknown")
        self.assertNotEqual(result.returncode, 0)
        parent = self.root / self._testMethodName
        data = parent / "speechify-tauri-candidate"
        self.assertEqual(
            sorted(p.name for p in data.iterdir()), ["audio", "exports", "logs", "models"]
        )
        self.assertFalse((parent / "speechify").exists())
        if os.name != "nt":
            self.assertEqual(data.stat().st_mode & 0o777, 0o700)
        result = self.run_cli(
            "install-model",
            "unknown",
            extra_env={"HOME": None, "APPDATA": None, "XDG_DATA_HOME": None},
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("No user data directory", result.stderr)

    def test_unwritable_data_location_is_reported_as_an_error(self):
        occupied = self.root / "occupied"
        occupied.write_text("file")
        result = self.run_cli(
            "install-model", "unknown", extra_env={"SPEECHIFY_CANDIDATE_PARENT": occupied}
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(occupied.read_text(), "file")

    def test_failed_model_download_reports_progress_and_preserves_the_installed_file(self):
        import http.server
        import threading

        requests = []

        class Proxy(http.server.BaseHTTPRequestHandler):
            def do_CONNECT(self):
                requests.append(self.path)
                self.send_error(502, "Test network unavailable")

            def log_message(self, *_):
                pass

        data = self.root / self._testMethodName / "speechify-tauri-candidate"
        models = data / "models"
        models.mkdir(parents=True)
        installed = models / "moonshine-base.gguf"
        installed.write_bytes(b"Existing installation")
        with http.server.ThreadingHTTPServer(("127.0.0.1", 0), Proxy) as proxy:
            server = threading.Thread(target=proxy.serve_forever)
            server.start()
            try:
                address = f"http://127.0.0.1:{proxy.server_port}"
                env = {
                    key: address
                    for key in [
                        "HTTP_PROXY",
                        "HTTPS_PROXY",
                        "ALL_PROXY",
                        "http_proxy",
                        "https_proxy",
                        "all_proxy",
                    ]
                }
                env.update(NO_PROXY="", no_proxy="")
                result = self.run_cli("install-model", "moonshine-base", extra_env=env)
            finally:
                proxy.shutdown()
                server.join(timeout=5)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('"stage":"Downloading"', result.stderr)
        self.assertTrue(requests)
        self.assertTrue(all(target == "huggingface.co:443" for target in requests))
        self.assertEqual(installed.read_bytes(), b"Existing installation")

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
        result = self.worker(
            "gpu-run-fails-cpu-load-fails",
            "vulkan",
            messages=[{"wav": str(self.wav)}, {"wav": str(self.wav)}],
        )
        rows = list(map(json.loads, result.stdout.splitlines()))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("21", rows[1]["error"])
        self.assertEqual(rows[2]["error"], "Engine unavailable")
        result = self.worker("bad-run")
        self.assertEqual(
            json.loads(result.stdout.splitlines()[-1])["error"], "transcribe.cpp status 26"
        )
        for text, message in [
            ("invalid\n", "expected value"),
            ("x" * 32769 + "\n", "Worker message too large"),
        ]:
            result = self.run_cli("--native-worker", self.root, "plain", "cpu", "2", text=text)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(message, result.stderr)

    def test_worker_stays_ready_while_its_owner_is_alive_and_serves_a_later_request(self):
        worker = subprocess.Popen(
            [str(BINARY), "--native-worker", str(self.root), "plain", "cpu", "2"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=self.environment(),
        )
        try:
            self.assertTrue(json.loads(read_line(worker))["ready"])
            time.sleep(1.2)
            self.assertIsNone(worker.poll())
            worker.stdin.write(json.dumps({"wav": str(self.wav)}) + "\n")
            worker.stdin.flush()
            self.assertTrue(json.loads(read_line(worker))["text"].startswith("160|en|"))
            worker.stdin.close()
            self.assertEqual(worker.wait(timeout=5), 0)
        finally:
            if worker.poll() is None:
                worker.kill()
                worker.wait(timeout=5)
            worker.stdin.close()
            worker.stdout.close()
            worker.stderr.close()

    def test_fixture_without_runtime_overrides_reports_the_missing_model(self):
        result = self.run_cli(
            "fixture", "moonshine-base", self.wav, extra_env={"SPEECHIFY_NATIVE_DIR": None}
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Install the selected model", result.stderr)

    def write_clipboard(self, text, target="UTF8_STRING"):
        subprocess.run(
            ["xclip", "-selection", "clipboard", "-i", "-target", target],
            input=text.encode(),
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            env=self.environment(),
            check=True,
            timeout=5,
        )

    def read_clipboard(self, target="UTF8_STRING"):
        return subprocess.run(
            ["xclip", "-selection", "clipboard", "-o", "-target", target],
            capture_output=True,
            env=self.environment(),
            timeout=5,
        )

    def clipboard_worker(self, text, extra_env=None):
        worker = subprocess.Popen(
            [str(BINARY), "--clipboard-worker"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env={**self.environment(), **(extra_env or {})},
            text=True,
        )

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
        ready = json.loads(read_line(worker))
        return worker, ready

    @unittest.skipUnless(sys.platform == "linux", "X11 clipboard contract")
    def test_clipboard_restores_rich_content_after_the_dictation_is_consumed(self):
        original = "<b>Earlier copied text & café</b>"
        self.write_clipboard(original, "text/html")
        worker, ready = self.clipboard_worker("A new dictation 🦀")
        self.assertEqual(ready, {"Ok": None})
        self.assertEqual(self.read_clipboard().stdout.decode(), "A new dictation 🦀")
        targets = self.read_clipboard("TARGETS")
        self.assertEqual(targets.returncode, 0, targets.stderr)
        self.assertIn(b"UTF8_STRING", targets.stdout)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            restored = self.read_clipboard("text/html")
            if restored.returncode == 0:
                break
            time.sleep(0.02)
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
        time.sleep(0.55)
        self.assertEqual(self.read_clipboard().stdout.decode(), "Dictation")

    @unittest.skipUnless(sys.platform == "linux", "X11 clipboard contract")
    def test_unsupported_or_oversized_clipboard_refuses_paste_and_preserves_the_copy(self):
        for target, original, reason in [
            ("application/x-private-handle", "private", "cannot be preserved"),
            ("text/html", "x" * (128 * 1024 + 1), "128 KiB"),
        ]:
            with self.subTest(target=target):
                self.write_clipboard(original, target)
                worker, ready = self.clipboard_worker("Must not replace the clipboard")
                self.assertIn(reason, ready["Err"])
                self.assertNotEqual(worker.wait(timeout=5), 0)
                self.assertEqual(self.read_clipboard(target).stdout.decode(), original)

    @unittest.skipUnless(sys.platform == "linux", "X11 clipboard contract")
    def test_clipboard_worker_reports_a_missing_display_and_malformed_input(self):
        result = self.run_cli(
            "--clipboard-worker",
            text=json.dumps("dictation") + "\n",
            extra_env={"DISPLAY": ":65530"},
        )
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
        x.XSetSelectionOwner.argtypes = [
            ctypes.c_void_p,
            ctypes.c_ulong,
            ctypes.c_ulong,
            ctypes.c_ulong,
        ]
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
        time.sleep(0.6)
        self.assertNotEqual(self.read_clipboard().returncode, 0)
        self.write_clipboard("Next copy")
        self.assertEqual(worker.wait(timeout=5), 0)

    @unittest.skipUnless(sys.platform == "linux", "X11 clipboard contract")
    def test_clipboard_boundary_refusals_preserve_the_previous_copy(self):
        shim = str(ROOT / ".build/x11-desktop.so")
        for scenario, message in [
            ("no-selection-notifications", "notifications unavailable"),
            ("changed-before-grab", "changed before paste"),
            ("queued-copy-notification", "changed before paste"),
            ("ownership-refused", "ownership refused"),
        ]:
            with self.subTest(scenario=scenario):
                self.write_clipboard("Earlier copy")
                worker, ready = self.clipboard_worker(
                    "Must not overwrite",
                    {
                        "LD_PRELOAD": shim,
                        "SPEECHIFY_TEST_X11_SHIM": shim,
                        "SPEECHIFY_TEST_X11_SCENARIO": scenario,
                    },
                )
                self.assertIn(message, ready["Err"])
                self.assertNotEqual(worker.wait(timeout=5), 0)
                self.assertEqual(self.read_clipboard().stdout.decode(), "Earlier copy")

    @unittest.skipUnless(sys.platform == "linux", "X11 clipboard contract")
    def test_clipboard_startup_events_are_drained_and_an_event_loop_failure_is_reported(self):
        shim = str(ROOT / ".build/x11-desktop.so")
        for scenario in ["startup-event", "unavailable-event-api"]:
            with self.subTest(scenario=scenario):
                self.write_clipboard("Previous copy")
                worker, ready = self.clipboard_worker(
                    "Dictation",
                    {
                        "LD_PRELOAD": shim,
                        "SPEECHIFY_TEST_X11_SHIM": shim,
                        "SPEECHIFY_TEST_X11_SCENARIO": scenario,
                    },
                )
                self.assertEqual(ready, {"Ok": None})
                if scenario == "unavailable-event-api":
                    self.assertNotEqual(worker.wait(timeout=5), 0)
                    self.assertIn("Clipboard owner stopped", worker.stderr.read())
                else:
                    self.assertEqual(self.read_clipboard().stdout.decode(), "Dictation")
                    self.write_clipboard("Newer copy")
                    self.assertEqual(worker.wait(timeout=5), 0)
                    self.assertEqual(self.read_clipboard().stdout.decode(), "Newer copy")

    @unittest.skipUnless(sys.platform == "linux", "X11 clipboard contract")
    def test_failed_promised_formats_and_excessive_total_snapshot_size_refuse_paste(self):
        tools = self.root / self._testMethodName / "tools"
        tools.mkdir(parents=True)
        for scenario, listing, body, message in [
            ("failed-target", "printf 'UTF8_STRING\\n'", "exit 1", "Cannot snapshot"),
            (
                "empty-snapshot",
                "printf 'TARGETS\\nTIMESTAMP\\n'",
                "exit 1",
                "changed while snapshotting",
            ),
            ("invalid-targets", "printf '\\377'", "exit 1", "Invalid clipboard targets"),
            (
                "total-limit",
                "i=0; while [ $i -lt 257 ]; do printf 'text/type-%s\\n' $i; i=$((i+1)); done",
                "/usr/bin/head -c 131072 /dev/zero",
                "exceeds 32 MB",
            ),
        ]:
            with self.subTest(scenario=scenario):
                script = tools / "xclip"
                script.write_text(
                    f'#!/bin/sh\nif [ "$5" = TARGETS ]; then {listing}; else {body}; fi\n'
                )
                script.chmod(0o700)
                self.write_clipboard("Previous copy")
                worker, ready = self.clipboard_worker(
                    "Must not overwrite", {"PATH": str(tools) + os.pathsep + os.environ["PATH"]}
                )
                self.assertIn(message, ready["Err"])
                self.assertNotEqual(worker.wait(timeout=5), 0)
                self.assertEqual(self.read_clipboard().stdout.decode(), "Previous copy")

    @unittest.skipUnless(sys.platform == "linux", "X11 clipboard contract")
    def test_clipboard_owner_disappearing_before_restore_keeps_the_new_empty_clipboard(self):
        shim = str(ROOT / ".build/x11-desktop.so")
        self.write_clipboard("Previous copy")
        worker, ready = self.clipboard_worker(
            "Temporary",
            {
                "LD_PRELOAD": shim,
                "SPEECHIFY_TEST_X11_SHIM": shim,
                "SPEECHIFY_TEST_X11_SCENARIO": "owner-disappeared-before-restore",
            },
        )
        self.assertEqual(ready, {"Ok": None})
        self.assertEqual(worker.wait(timeout=5), 0)
        self.assertNotEqual(self.read_clipboard().returncode, 0)

    @unittest.skipUnless(sys.platform == "linux", "X11 clipboard contract")
    def test_selection_clients_omitting_the_property_still_receive_the_dictation(self):
        import ctypes as c

        self.write_clipboard("Previous copy")
        worker, ready = self.clipboard_worker("Dictation 🦀")
        self.assertEqual(ready, {"Ok": None})
        x = c.CDLL("libX11.so.6")
        signatures = {
            "XOpenDisplay": (c.c_void_p, [c.c_char_p]),
            "XDefaultRootWindow": (c.c_ulong, [c.c_void_p]),
            "XCreateSimpleWindow": (
                c.c_ulong,
                [
                    c.c_void_p,
                    c.c_ulong,
                    c.c_int,
                    c.c_int,
                    c.c_uint,
                    c.c_uint,
                    c.c_uint,
                    c.c_ulong,
                    c.c_ulong,
                ],
            ),
            "XInternAtom": (c.c_ulong, [c.c_void_p, c.c_char_p, c.c_int]),
            "XConvertSelection": (
                c.c_int,
                [c.c_void_p, c.c_ulong, c.c_ulong, c.c_ulong, c.c_ulong, c.c_ulong],
            ),
            "XFlush": (c.c_int, [c.c_void_p]),
            "XPending": (c.c_int, [c.c_void_p]),
            "XNextEvent": (c.c_int, [c.c_void_p, c.c_void_p]),
            "XGetWindowProperty": (
                c.c_int,
                [
                    c.c_void_p,
                    c.c_ulong,
                    c.c_ulong,
                    c.c_long,
                    c.c_long,
                    c.c_int,
                    c.c_ulong,
                    c.POINTER(c.c_ulong),
                    c.POINTER(c.c_int),
                    c.POINTER(c.c_ulong),
                    c.POINTER(c.c_ulong),
                    c.POINTER(c.c_void_p),
                ],
            ),
            "XFree": (c.c_int, [c.c_void_p]),
            "XDestroyWindow": (c.c_int, [c.c_void_p, c.c_ulong]),
            "XCloseDisplay": (c.c_int, [c.c_void_p]),
        }
        for name, (result, args) in signatures.items():
            getattr(x, name).restype, getattr(x, name).argtypes = result, args
        display = x.XOpenDisplay(self.display.encode())
        self.assertTrue(display)
        window = x.XCreateSimpleWindow(display, x.XDefaultRootWindow(display), 0, 0, 1, 1, 0, 0, 0)
        try:
            target = x.XInternAtom(display, b"UTF8_STRING", 0)
            x.XConvertSelection(
                display, x.XInternAtom(display, b"CLIPBOARD", 0), target, 0, window, 0
            )
            x.XFlush(display)
            deadline = time.monotonic() + 5
            while not x.XPending(display) and time.monotonic() < deadline:
                time.sleep(0.001)
            self.assertTrue(x.XPending(display), "No selection reply")
            event = (c.c_long * 24)()
            x.XNextEvent(display, event)
            self.assertEqual(c.cast(event, c.POINTER(c.c_int))[0], 31)
            kind, count, remaining, data, format_ = (
                c.c_ulong(),
                c.c_ulong(),
                c.c_ulong(),
                c.c_void_p(),
                c.c_int(),
            )
            self.assertEqual(
                x.XGetWindowProperty(
                    display,
                    window,
                    target,
                    0,
                    1024,
                    1,
                    0,
                    c.byref(kind),
                    c.byref(format_),
                    c.byref(count),
                    c.byref(remaining),
                    c.byref(data),
                ),
                0,
            )
            try:
                self.assertEqual(format_.value, 8)
                self.assertEqual(c.string_at(data, count.value).decode(), "Dictation 🦀")
            finally:
                x.XFree(data)
        finally:
            x.XDestroyWindow(display, window)
            x.XCloseDisplay(display)
        self.write_clipboard("Newer copy")
        self.assertEqual(worker.wait(timeout=5), 0)

    @unittest.skipUnless(
        sys.platform == "linux", "Desktop lifecycle on an isolated X11/D-Bus session"
    )
    def test_desktop_records_into_a_target_survives_closing_settings_and_quits_cleanly(self):
        bus = subprocess.check_output(
            ["dbus-daemon", "--session", "--fork", "--print-address=1", "--print-pid=1"],
            text=True,
            timeout=10,
        ).splitlines()
        self.addCleanup(lambda: os.kill(int(bus[1]), 15))
        env = self.environment()
        env.update(
            DBUS_SESSION_BUS_ADDRESS=bus[0],
            LIBGL_ALWAYS_SOFTWARE="1",
            WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS="1",
        )
        # Restrict capture to a synthetic ALSA device, including if the test host
        # has a real microphone attached.
        env["ALSA_CONFIG_PATH"] = str(ROOT / "tests/fixtures/alsa.conf")
        env["ALSA_PLUGIN_DIR"] = str(ROOT / ".build/alsa")
        data = Path(env["SPEECHIFY_CANDIDATE_PARENT"]) / "speechify-tauri-candidate"
        model = data / "models/whisper-ft-cli"
        model.mkdir(parents=True)
        (model / "model.bin").write_text("fixture")
        python = data / "environments/whisper/bin/python"
        python.parent.mkdir(parents=True)
        shutil.copy2(self.worker_peer, python)
        (data / "config.toml").write_text(
            'model = "ft:whisper-ft-cli"\ndevice = "cpu"\ninput_device = "Speechify float input"\nlearn_auto = false\n'
        )
        (ROOT / "coverage").mkdir(exist_ok=True)
        log = open(ROOT / "coverage/desktop-startup.log", "w+")
        self.addCleanup(log.close)
        app = subprocess.Popen(
            [str(BINARY)], stdin=subprocess.DEVNULL, stdout=log, stderr=log, env=env
        )

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
                time.sleep(0.02)
            log.seek(0)
            self.fail(f"Desktop behavior did not settle: {log.read()}")

        def windows(visible=False):
            args = (
                ["xdotool", "search"]
                + (["--onlyvisible"] if visible else [])
                + ["--name", "^Speechify Tauri Candidate$"]
            )
            found = subprocess.run(args, capture_output=True, text=True, env=env, timeout=5)
            return found.stdout.split()

        window = wait(lambda: windows(True))[0]

        def dbus(*args):
            result = subprocess.run(
                ["gdbus", *args], capture_output=True, text=True, env=env, timeout=5
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            return result.stdout

        def find_menu():
            names = dbus(
                "call",
                "--session",
                "--dest",
                "org.freedesktop.DBus",
                "--object-path",
                "/org/freedesktop/DBus",
                "--method",
                "org.freedesktop.DBus.ListNames",
            )
            for name in re.findall("'([^']+)'", names):
                process = subprocess.run(
                    [
                        "gdbus",
                        "call",
                        "--session",
                        "--dest",
                        "org.freedesktop.DBus",
                        "--object-path",
                        "/org/freedesktop/DBus",
                        "--method",
                        "org.freedesktop.DBus.GetConnectionUnixProcessID",
                        name,
                    ],
                    capture_output=True,
                    text=True,
                    env=env,
                    timeout=5,
                )
                if process.returncode or not re.search(
                    r"\b" + str(app.pid) + r"\b", process.stdout
                ):
                    continue
                # Single-instance and tray connections share a PID. Discover
                # the connection that actually exports a menu interface.
                tree = subprocess.run(
                    [
                        "gdbus",
                        "introspect",
                        "--session",
                        "--dest",
                        name,
                        "--object-path",
                        "/",
                        "--recurse",
                    ],
                    capture_output=True,
                    text=True,
                    env=env,
                    timeout=5,
                )
                if tree.returncode:
                    continue
                match = re.search("readonly o Menu = '([^']+)'", tree.stdout)
                if match:
                    return name, match.group(1)
            return None

        menu_bus, menu_path = wait(find_menu)

        def menu(method, *args):
            return dbus(
                "call",
                "--session",
                "--dest",
                menu_bus,
                "--object-path",
                menu_path,
                "--method",
                f"com.canonical.dbusmenu.{method}",
                "--",
                *args,
            )

        layout = menu("GetLayout", "0", "-1", "[]")
        entries = {
            label: number
            for number, label in re.findall(r"\((\d+),[^{}]*\{[^{}]*'label': <'([^']+)'>", layout)
        }
        self.assertEqual(
            set(entries), {"Settings", "History", "Models", "Log location", "Quit candidate"}
        )

        def activate(label):
            menu("Event", entries[label], "clicked", "<0>", "0")

        def close_settings():
            import ctypes

            x = ctypes.CDLL("libX11.so.6")
            x.XOpenDisplay.argtypes = [ctypes.c_char_p]
            x.XOpenDisplay.restype = ctypes.c_void_p
            x.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
            x.XInternAtom.restype = ctypes.c_ulong
            x.XSendEvent.argtypes = [
                ctypes.c_void_p,
                ctypes.c_ulong,
                ctypes.c_int,
                ctypes.c_long,
                ctypes.c_void_p,
            ]
            x.XCloseDisplay.argtypes = [ctypes.c_void_p]

            class Message(ctypes.Structure):
                _fields_ = [
                    ("type", ctypes.c_int),
                    ("serial", ctypes.c_ulong),
                    ("send", ctypes.c_int),
                    ("display", ctypes.c_void_p),
                    ("window", ctypes.c_ulong),
                    ("message_type", ctypes.c_ulong),
                    ("format", ctypes.c_int),
                    ("data", ctypes.c_long * 5),
                ]

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

        for hotkey in ["ctrl+alt+f", "ctrl+alt+h", "ctrl+alt+m"]:
            close_settings()
            wait(lambda: not windows(True))
            subprocess.run(["xdotool", "keydown", hotkey], check=True, env=env, timeout=5)
            time.sleep(0.1)
            subprocess.run(["xdotool", "keyup", hotkey], check=True, env=env, timeout=5)
            wait(lambda: windows(True))

        # A short global talk-key tap must never create a dictation or audio.
        # Physical right-Control avoids xdotool's automatic left modifiers.
        subprocess.run(["xdotool", "keydown", "105"], check=True, env=env, timeout=5)
        time.sleep(0.06)
        subprocess.run(["xdotool", "keyup", "105"], check=True, env=env, timeout=5)
        time.sleep(0.3)
        import sqlite3

        with closing(sqlite3.connect(data / "learning.db")) as db:
            self.assertEqual(db.execute("SELECT count(*) FROM samples").fetchone()[0], 0)
        self.assertEqual(list((data / "audio").iterdir()), [])

        # An external X11 target receives the native paste key and consumes the
        # transient clipboard, just as an editor does when handling Ctrl+V.
        import ctypes

        x = ctypes.CDLL("libX11.so.6")
        x.XOpenDisplay.argtypes = [ctypes.c_char_p]
        x.XOpenDisplay.restype = ctypes.c_void_p
        x.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
        x.XDefaultRootWindow.restype = ctypes.c_ulong
        x.XCreateSimpleWindow.argtypes = [
            ctypes.c_void_p,
            ctypes.c_ulong,
            ctypes.c_int,
            ctypes.c_int,
            ctypes.c_uint,
            ctypes.c_uint,
            ctypes.c_uint,
            ctypes.c_ulong,
            ctypes.c_ulong,
        ]
        x.XCreateSimpleWindow.restype = ctypes.c_ulong
        x.XMapWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        x.XSelectInput.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_long]
        x.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
        x.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
        x.XPending.argtypes = [ctypes.c_void_p]
        x.XNextEvent.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
        x.XDestroyWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        x.XCloseDisplay.argtypes = [ctypes.c_void_p]

        class KeyEvent(ctypes.Structure):
            _fields_ = [
                ("type", ctypes.c_int),
                ("serial", ctypes.c_ulong),
                ("send", ctypes.c_int),
                ("display", ctypes.c_void_p),
                ("window", ctypes.c_ulong),
                ("root", ctypes.c_ulong),
                ("subwindow", ctypes.c_ulong),
                ("time", ctypes.c_ulong),
                ("x", ctypes.c_int),
                ("y", ctypes.c_int),
                ("x_root", ctypes.c_int),
                ("y_root", ctypes.c_int),
                ("state", ctypes.c_uint),
                ("keycode", ctypes.c_uint),
                ("same_screen", ctypes.c_int),
            ]

        class KeyUnion(ctypes.Union):
            _fields_ = [("key", KeyEvent), ("pad", ctypes.c_long * 24)]

        target_display = x.XOpenDisplay(self.display.encode())
        self.assertTrue(target_display)
        target = x.XCreateSimpleWindow(
            target_display, x.XDefaultRootWindow(target_display), 0, 0, 200, 100, 0, 0, 0
        )
        try:
            x.XSelectInput(target_display, target, 1 | 2)
            x.XMapWindow(target_display, target)
            x.XSetInputFocus(target_display, target, 1, 0)
            x.XSync(target_display, 0)
            previous = "Keep my earlier copy 🦀"
            self.write_clipboard(previous)
            subprocess.run(["xdotool", "keydown", "105"], check=True, env=env, timeout=5)
            time.sleep(0.4)
            subprocess.run(["xdotool", "keyup", "105"], check=True, env=env, timeout=5)
            received = []

            def consume_paste():
                while x.XPending(target_display):
                    event = KeyUnion()
                    x.XNextEvent(target_display, ctypes.byref(event))
                    if event.key.type == 2 and event.key.keycode == 55 and event.key.state & 4:
                        received.append(self.read_clipboard().stdout.decode())
                return bool(received)

            wait(consume_paste)
            self.assertEqual(received, ["get hub"])
            wait(lambda: self.read_clipboard().stdout.decode() == previous)
            with closing(sqlite3.connect(data / "learning.db")) as db:
                self.assertEqual(
                    db.execute("SELECT raw,pasted FROM samples").fetchall(),
                    [("get hub", "get hub")],
                )
            self.assertEqual(len(list((data / "audio").glob("*.wav"))), 1)
            self.assertEqual(list(data.glob("pending-*.wav")), [])
        finally:
            x.XDestroyWindow(target_display, target)
            x.XCloseDisplay(target_display)
        activate("Quit candidate")
        self.assertEqual(app.wait(timeout=10), 0)

        # A fresh installation without its selected model still opens settings
        # and exposes a functioning tray, so the user can install a model.
        shutil.rmtree(model)
        app = subprocess.Popen(
            [str(BINARY)], stdin=subprocess.DEVNULL, stdout=log, stderr=log, env=env
        )
        wait(lambda: windows(True))
        menu_bus, menu_path = wait(find_menu)
        layout = menu("GetLayout", "0", "-1", "[]")
        entries = {
            label: number
            for number, label in re.findall(r"\((\d+),[^{}]*\{[^{}]*'label': <'([^']+)'>", layout)
        }
        self.assertEqual(
            set(entries), {"Settings", "History", "Models", "Log location", "Quit candidate"}
        )
        time.sleep(0.1)
        self.assertIsNone(app.poll(), "A missing model stopped the desktop")
        activate("Models")
        wait(lambda: windows(True))
        activate("Quit candidate")
        self.assertEqual(app.wait(timeout=10), 0)


if __name__ == "__main__":
    unittest.main()
