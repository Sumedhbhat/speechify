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
        path.write_text(f"SF:{source}\n" + "\n".join(f"DA:{line},{count}" for line, count in counts) + "\nFN:1,application_function\nFNDA:1,application_function\nend_of_record\n")
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

    def test_same_named_platform_functions_cannot_cover_each_others_bodies(self):
        linux = self.backend("linux.info", "src-tauri/src/main.rs", [(1, 1), (2, 0)])
        windows = self.backend("windows.info", "src-tauri/src/main.rs", [(1, 0), (2, 1)])
        windows.write_text(windows.read_text().replace("FN:1,", "FN:2,").replace("FNDA:1,", "FNDA:0,"))
        mac = self.backend("mac.info", "src-tauri/src/mac_clipboard.rs", [(1, 1)])
        failures = gate.check(self.frontend, [linux, windows, mac], details=True)
        self.assertIn("src-tauri/src/main.rs: 1/2 functions uncovered", failures)
        self.assertIn("  functions: application_function [line 2]", failures)

    def test_a_function_count_without_a_source_definition_is_rejected(self):
        backend = self.backend("invalid.info", "src-tauri/src/main.rs", [(1, 1)])
        backend.write_text(backend.read_text().replace("FN:1,application_function\n", ""))
        with self.assertRaisesRegex(ValueError, "no source location"):
            gate.check(self.frontend, [backend])

    def test_generic_instantiations_merge_but_distinct_closure_bodies_do_not(self):
        first = "app::download::<app::install<first::Callback>>::{closure#0}"
        second = "app::download::<second::Callback>::{closure#0}"
        self.assertEqual(gate.function_definition(first), "app::download::{closure#0}")
        self.assertEqual(gate.function_definition(first), gate.function_definition(second))
        self.assertNotEqual(gate.function_definition(first), gate.function_definition(second.replace("closure#0", "closure#1")))
        self.assertEqual(gate.function_definition("<app::Capture>::open"), "<app::Capture>::open")

    def test_source_fingerprints_detect_changes_and_normalize_platform_line_endings(self):
        path = self.root / "src/main.ts"
        path.write_text("application code\n")
        before = gate.source_fingerprints()
        path.write_bytes(b"application code\r\n")
        self.assertEqual(gate.source_fingerprints(), before)
        path.write_text("different behavior\n")
        self.assertNotEqual(gate.source_fingerprints(), before)
        before = gate.source_fingerprints()
        (self.root / "package-lock.json").write_text('{"version":"changed"}')
        self.assertNotEqual(gate.source_fingerprints(), before)


if __name__ == "__main__":
    unittest.main()
