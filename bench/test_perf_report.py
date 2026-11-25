import copy
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import perf_report

HERE = Path(__file__).parent


def platform(name, time_ms=1000.0, passed=True, text="Okay."):
    return {
        "schema": perf_report.SCHEMA,
        "platform": name,
        "models": {"speech": "speech-a", "cleanup": "clean-a", "cleanup_digest": "sha256:1"},
        "transcription": {"load_ms": 500.0, "time_ms": time_ms, "realtime": 11.0, "wer": 0.0, "audio_s": 11.0},
        "cleanup": {
            "rules_ms": 1.0,
            "llm": {"median_ms": 9000.0, "p90_ms": 12000.0, "calls": 2, "rejected": 0},
        },
        "scenarios": [
            {
                "id": "filler-short",
                "kind": "fillers",
                "raw": "Erm. Okay",
                "rules": {"text": "Okay", "passed": True, "failures": []},
                "llm": {"text": text, "passed": passed, "failures": [] if passed else ['expected exactly "Okay"']},
            }
        ],
    }


def everywhere(**kwargs):
    return {key: platform(key, **kwargs) for key, _ in perf_report.PLATFORMS}


def doc(platforms):
    return {"schema": perf_report.SCHEMA, "head_sha": "abcdef1234", "platforms": platforms}


class Render(unittest.TestCase):
    def test_report_reads_the_format_the_benchmark_writes(self):
        source = (HERE / "src/bin/perf.rs").read_text()
        self.assertIn(f"const SCHEMA: u32 = {perf_report.SCHEMA};", source)

    def test_new_scenarios_are_not_counted_as_improvements(self):
        baseline = doc(everywhere())
        current = everywhere()
        for p in current.values():
            extra = copy.deepcopy(p["scenarios"][0])
            extra["id"] = "filler-new"
            p["scenarios"].append(extra)
        body = perf_report.render(current, baseline, "", "")
        self.assertIn("| Fillers | 2/2 (was 1/1) / 2/2 (was 1/1) |", body)
        self.assertNotIn("(+1)", body)

    def test_rejections_compare_only_with_the_same_number_of_calls(self):
        baseline = doc(everywhere())
        current = everywhere()
        for p in current.values():
            p["cleanup"]["llm"].update(calls=3, rejected=1)
        body = perf_report.render(current, baseline, "", "")
        self.assertIn("1/3 (main: 0/2)", body)
        self.assertIn("No regressions beyond the noise margin.", body)

    def test_without_a_baseline(self):
        body = perf_report.render(everywhere(), None, "", "")
        self.assertIn("No nightly release has performance results yet", body)
        self.assertIn("Nothing to compare with yet.", body)
        self.assertNotIn("⚠️)", body)

    def test_old_format_is_reported_not_hidden(self):
        body = perf_report.render(everywhere(), None, "", "v1-nightly.3", skipped_schema=1)
        self.assertIn("Not compared", body)
        self.assertIn("`v1-nightly.3` has results in format 1", body)

    def test_missing_platform_is_named(self):
        current = everywhere()
        del current["macos"]
        body = perf_report.render(current, None, "", "")
        self.assertIn("No results from macOS", body)
        self.assertNotIn("| macOS", body)

    def test_slower_beyond_noise_is_flagged(self):
        body = perf_report.render(everywhere(time_ms=1300.0), doc(everywhere()), "", "v1-nightly.4")
        self.assertIn("1300 ms (+30% ⚠️)", body)
        self.assertIn("Worse than main: Time to text on Linux", body)

    def test_small_changes_are_noise(self):
        body = perf_report.render(everywhere(time_ms=1100.0), doc(everywhere()), "", "v1-nightly.4")
        self.assertIn("1100 ms (+10%)", body)
        self.assertIn("No regressions beyond the noise margin.", body)

    def test_rows_compare_by_key_not_label(self):
        baseline = doc(everywhere(time_ms=500.0))
        original = perf_report.transcription

        def renamed(p):
            return [(key, label.upper(), *rest) for key, label, *rest in original(p)]

        try:
            perf_report.transcription = renamed
            body = perf_report.render(everywhere(), baseline, "", "")
        finally:
            perf_report.transcription = original
        self.assertIn("1000 ms (+100% ⚠️)", body)

    def test_scenario_that_stops_passing(self):
        body = perf_report.render(everywhere(passed=False, text="Okay!"), doc(everywhere()), "", "")
        self.assertIn("❌ `filler-short` now fails with the language model on Linux, macOS, Windows", body)
        self.assertIn("1 scenario result(s) got worse", body)
        self.assertIn("| fillers | `filler-short` | with model | Linux, macOS, Windows | Erm. Okay | Okay! |", body)

    def test_different_models_are_named(self):
        baseline = doc(everywhere())
        for p in baseline["platforms"].values():
            p["models"] = {"speech": "speech-b", "cleanup": "clean-b", "cleanup_digest": "sha256:2"}
        body = perf_report.render(everywhere(), baseline, "", "")
        self.assertIn("`main` was measured with speech `speech-b`, language model `clean-b`", body)


class Comment(unittest.TestCase):
    def failing(self, count, text):
        current = everywhere()
        for key, p in current.items():
            template = p["scenarios"][0]
            p["scenarios"] = []
            for i in range(count):
                s = copy.deepcopy(template)
                s["id"] = f"case-{i}"
                s["llm"] = {"text": f"{key} {text}", "passed": False, "failures": ["nope"]}
                p["scenarios"].append(s)
        return current

    def test_long_failure_tables_are_cut(self):
        body = perf_report.comment(self.failing(30, "x"), None, "", "", None)
        self.assertIn(f"…and {90 - perf_report.FAILURE_ROWS} more in the run's job summary.", body)
        self.assertLessEqual(len(body), perf_report.COMMENT_LIMIT)

    def test_comment_stays_under_the_limit(self):
        current = self.failing(200, "y" * 5000)
        self.assertGreater(len(perf_report.render(current, None, "", "")), perf_report.COMMENT_LIMIT)
        body = perf_report.comment(current, None, "", "", None)
        self.assertLessEqual(len(body), perf_report.COMMENT_LIMIT)
        self.assertIn("### Verdict", body)

    def test_cells_are_escaped_and_shortened(self):
        self.assertEqual(perf_report.cell("a|b\nc"), "a\\|b⏎c")
        self.assertEqual(len(perf_report.cell("z" * 1000)), perf_report.CELL_CHARS)


class Command(unittest.TestCase):
    def run_report(self, platforms, *extra):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            for key, p in platforms.items():
                folder = tmp / "in" / f"perf-result-{key}"
                folder.mkdir(parents=True)
                (folder / f"{key}.json").write_text(json.dumps(p))
            out = subprocess.run(
                [
                    sys.executable,
                    str(HERE / "perf_report.py"),
                    str(tmp / "in"),
                    "--head-sha",
                    "1234567",
                    "--out-json",
                    str(tmp / "record.json"),
                    "--out-md",
                    str(tmp / "comment.md"),
                    *extra,
                ],
                capture_output=True,
                text=True,
            )
            record = json.loads((tmp / "record.json").read_text()) if out.returncode == 0 else None
            return out, record

    def test_record_is_written(self):
        out, record = self.run_report(everywhere(), "--complete")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(record["schema"], perf_report.SCHEMA)
        self.assertIsNone(record["pr"])
        self.assertEqual(record["platforms"]["linux"]["transcription"]["time_ms"], 1000.0)

    def test_complete_refuses_a_missing_platform(self):
        current = everywhere()
        del current["windows"]
        out, _ = self.run_report(current, "--complete")
        self.assertNotEqual(out.returncode, 0)
        self.assertIn("No results from Windows", out.stderr)


if __name__ == "__main__":
    unittest.main()
