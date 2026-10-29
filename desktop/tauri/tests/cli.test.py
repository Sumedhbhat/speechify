"""Exercise the built CLI and native-worker protocol as external callers.

Usage: python3 tests/cli.test.py /absolute/path/to/speechify-tauri-candidate
Inherited LLVM_PROFILE_FILE lets cargo-llvm-cov include these real processes.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
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
        cls.wav = cls.root / "speech.wav"
        with wave.open(str(cls.wav), "wb") as wav:
            wav.setnchannels(1)
            wav.setsampwidth(2)
            wav.setframerate(16000)
            wav.writeframes(b"\x00\x10" * 160)

    @classmethod
    def tearDownClass(cls):
        cls.workspace.cleanup()

    def run_cli(self, *args, text=""):
        env = {**os.environ, "SPEECHIFY_CANDIDATE_PARENT": str(self.root / self._testMethodName)}
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


if __name__ == "__main__":
    unittest.main()
