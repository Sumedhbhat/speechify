#!/usr/bin/env python3
"""Fail unless every application source is measured and fully covered.

Consumes real Vitest and cargo-llvm-cov reports. Test fixtures, declarations,
and build tooling are outside the application source denominator.
"""
import argparse
import json
from pathlib import Path


def check(frontend: Path, backend: Path) -> list[str]:
    root = Path(__file__).resolve().parents[1]
    failures = []
    js = json.loads(frontend.read_text())
    measured = {Path(name).resolve(): value for name, value in js.items()}
    for source in sorted((root / "src").rglob("*.ts")):
        if source.name.endswith(".d.ts"):
            continue
        report = measured.get(source.resolve())
        if report is None:
            failures.append(f"{source.relative_to(root)}: not measured")
            continue
        for name, counts in [("statements", report["s"].values()),
                             ("functions", report["f"].values()),
                             ("branches", (n for values in report["b"].values() for n in values))]:
            counts = list(counts)
            missing = sum(n == 0 for n in counts)
            if missing:
                failures.append(f"{source.relative_to(root)}: {missing}/{len(counts)} {name} uncovered")

    llvm = json.loads(backend.read_text())
    files = {Path(f["filename"]).resolve(): f for run in llvm["data"] for f in run["files"]}
    for source in sorted((root / "src-tauri/src").glob("*.rs")):
        if source.name.endswith("_tests.rs"):
            continue
        report = files.get(source.resolve())
        if report is None:
            failures.append(f"{source.relative_to(root)}: not measured (run on its supported OS)")
            continue
        for name in ["lines", "functions", "regions"]:
            counts = report["summary"][name]
            if counts["covered"] != counts["count"]:
                failures.append(f"{source.relative_to(root)}: {counts['count'] - counts['covered']}/{counts['count']} {name} uncovered")
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frontend", type=Path, default=Path("coverage/coverage-final.json"))
    parser.add_argument("--backend", type=Path, default=Path("coverage/rust.json"))
    args = parser.parse_args()
    failures = check(args.frontend, args.backend)
    for failure in failures:
        print(failure)
    if not failures:
        print("All Tauri application sources have 100% coverage.")
    return int(bool(failures))


if __name__ == "__main__":
    raise SystemExit(main())
