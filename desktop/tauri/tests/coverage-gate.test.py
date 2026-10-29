"""The gate must reject incomplete reports, including unimported files."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("gate", Path(__file__).resolve().parents[1] / "scripts/check-coverage.py")
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


class CoverageGate(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        for name in ["src/main.ts", "src/env.d.ts", "src-tauri/src/main.rs", "src-tauri/src/mac_clipboard.rs"]:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("application code")
        self.frontend = self.root / "frontend.json"
        self.frontend.write_text(json.dumps({str(self.root / "src/main.ts"): {"s": {"0": 1}, "f": {"0": 1}, "b": {"0": [1, 1]}}}))
        self.patch = patch.object(gate, "ROOT", self.root)
        self.patch.start()
        self.addCleanup(self.patch.stop)

    def backend(self, name, source, counts):
        path = self.root / name
        path.write_text(f"SF:{source}\n" + "\n".join(f"DA:{line},{count}" for line, count in counts) + "\nFNDA:1,application_function\nend_of_record\n")
        return path

    def test_platform_files_must_be_measured_even_on_another_host(self):
        linux = self.backend("linux.info", "/runner/desktop/tauri/src-tauri/src/main.rs", [(1, 1)])
        failures = gate.check(self.frontend, [linux])
        self.assertEqual(failures, ["src-tauri/src/mac_clipboard.rs: not measured (run on its supported OS)"])

    def test_combines_native_runs_without_requiring_identical_checkout_paths(self):
        linux = self.backend("linux.info", "/runner/desktop/tauri/src-tauri/src/main.rs", [(1, 1), (2, 0)])
        windows = self.backend("windows.info", "D:\\checkout\\desktop\\tauri\\src-tauri\\src\\main.rs", [(1, 0), (2, 1)])
        mac = self.backend("mac.info", "/Users/runner/desktop/tauri/src-tauri/src/mac_clipboard.rs", [(1, 1)])
        self.assertEqual(gate.check(self.frontend, [linux, windows, mac]), [])
        self.assertIn("src-tauri/src/main.rs: 1/2 lines uncovered", gate.check(self.frontend, [linux, mac]))

    def test_unexecuted_source_and_frontend_branches_fail_the_gate(self):
        self.frontend.write_text(json.dumps({str(self.root / "src/main.ts"): {"s": {"0": 1}, "f": {"0": 0}, "b": {"0": [1, 0]}}}))
        (self.root / "src/new-feature.ts").write_text("never imported")
        failures = gate.check(self.frontend, [])
        self.assertIn("src/main.ts: 1/1 functions uncovered", failures)
        self.assertIn("src/main.ts: 1/2 branches uncovered", failures)
        self.assertIn("src/new-feature.ts: not measured", failures)


if __name__ == "__main__":
    unittest.main()
