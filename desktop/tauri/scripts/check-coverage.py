#!/usr/bin/env python3
"""Require every Rust/TypeScript application source to be fully covered.

Accept LCOV reports from each supported OS. A line/function is covered if a
native run exercises it on any supported OS; platform sources never disappear
from the denominator just because the current machine cannot compile them.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def source_fingerprints() -> dict[str, str]:
    paths = list((ROOT / "src").rglob("*.ts")) + list((ROOT / "src").rglob("*.css"))
    paths += list((ROOT / "src-tauri/src").rglob("*.rs"))
    paths += list((ROOT / "src-tauri/resources").glob("*.json"))
    for pattern in ["*.py", "*.in", "*.lock"]:
        paths += list((ROOT / "src-tauri/resources").rglob(pattern))
    paths += list((ROOT / "src-tauri/capabilities").glob("*.json"))
    paths += [ROOT / name for name in ["index.html", "package.json", "package-lock.json", "src-tauri/build.rs", "src-tauri/Cargo.toml", "src-tauri/Cargo.lock", "src-tauri/tauri.conf.json", "src-tauri/tauri.macos.conf.json", "src-tauri/tauri.windows.conf.json", "src-tauri/Entitlements.plist"]]
    return {p.relative_to(ROOT).as_posix(): hashlib.sha256(p.read_text(encoding="utf-8").encode()).hexdigest()
            for p in sorted(paths) if p.is_file()}


def source_path(name: str) -> str:
    name = name.replace("\\", "/")
    for prefix in ["src-tauri/src/", "src/"]:
        if prefix in name:
            return prefix + name.rsplit(prefix, 1)[1]
    return name


def function_definition(name: str) -> str:
    """Merge monomorphizations while retaining distinct function/closure bodies."""
    while "::<" in name:
        start = name.index("::<")
        depth = 1
        end = start + 3
        while end < len(name) and depth:
            depth += (name[end] == "<") - (name[end] == ">" and name[end - 1] != "-")
            end += 1
        if depth:
            raise ValueError(f"Unbalanced demangled function name: {name}")
        name = name[:start] + name[end:]
    return name


def check(frontend: Path, backends: list[Path], details: bool = False) -> list[str]:
    failures = []
    js = {source_path(name): value for name, value in json.loads(frontend.read_text(encoding="utf-8")).items()}
    for source in sorted((ROOT / "src").rglob("*.ts")):
        if source.name.endswith(".d.ts"):
            continue
        name = source.relative_to(ROOT).as_posix()
        report = js.get(name)
        if report is None:
            failures.append(f"{name}: not measured")
            continue
        for metric, counts in [("statements", report["s"].values()),
                               ("functions", report["f"].values()),
                               ("branches", (n for values in report["b"].values() for n in values))]:
            counts = list(counts)
            missing = sum(n == 0 for n in counts)
            if missing:
                failures.append(f"{name}: {missing}/{len(counts)} {metric} uncovered")

    files = {}
    for path in backends:
        current = None
        function_lines = {}
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.startswith("SF:"):
                current = files.setdefault(source_path(line[3:]), {"lines": {}, "functions": {}})
                function_lines = {}
            elif line == "end_of_record":
                current = None
            elif current is not None and line.startswith("DA:"):
                number, count, *_ = line[3:].split(",")
                current["lines"][number] = max(current["lines"].get(number, 0), int(count))
            elif current is not None and line.startswith("FN:"):
                number, name = line[3:].split(",", 1)
                function_lines[name] = int(number)
            elif current is not None and line.startswith("FNDA:"):
                count, name = line[5:].split(",", 1)
                if name not in function_lines:
                    raise ValueError(f"{path}: function has no source location: {name}")
                key = (name, function_lines[name])
                current["functions"][key] = max(current["functions"].get(key, 0), int(count))
    # The same function has different linker symbols in test/CLI binaries and
    # across target triples. Demangle names before merging their executions.
    # rustfilt's stdin filter only recognizes legacy symbols; pass names as args
    # so its demangler also handles the current Rust v0 symbol format.
    symbols = sorted({name for f in files.values() for name, _ in f["functions"] if name.startswith(("_R", "_ZN"))})
    canonical = {}
    for offset in range(0, len(symbols), 40):
        batch = symbols[offset:offset + 40]
        result = subprocess.run(["rustfilt", *batch], check=True, capture_output=True, text=True, encoding="utf-8")
        names = result.stdout.splitlines()
        if len(names) != len(batch):
            raise ValueError("rustfilt did not return one name per function")
        canonical.update(zip(batch, names))
    for filename, report in files.items():
        source = ROOT / filename
        annotations = {number for number, line in enumerate(source.read_text(encoding="utf-8").splitlines(), 1)
                       if line.strip() == "#[tauri::command]"} if source.is_file() else set()
        # LLVM attributes SDK-generated serialization closures to the closing
        # bracket of this non-executable annotation. The authored handler starts
        # on the following line and remains fully measured.
        report["lines"] = {line: count for line, count in report["lines"].items() if int(line) not in annotations}
        merged = {}
        for (name, line), count in report["functions"].items():
            if line in annotations:
                continue
            # cfg-specific bodies can have identical Rust names. Their source
            # locations distinguish e.g. Windows and X11 Poller::new().
            key = f"{function_definition(canonical.get(name, name))} [line {line}]"
            merged[key] = max(merged.get(key, 0), count)
        report["functions"] = merged
    for source in sorted((ROOT / "src-tauri/src").rglob("*.rs")):
        name = source.relative_to(ROOT).as_posix()
        report = files.get(name)
        if report is None:
            failures.append(f"{name}: not measured (run on its supported OS)")
            continue
        for metric, counts in report.items():
            if not counts:
                failures.append(f"{name}: no measured {metric}")
            elif missing := sum(n == 0 for n in counts.values()):
                failures.append(f"{name}: {missing}/{len(counts)} {metric} uncovered")
                if details:
                    failures.extend(f"  {metric}: {key}" for key, count in counts.items() if count == 0)
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frontend", type=Path, default=Path("coverage/coverage-final.json"))
    parser.add_argument("--backend", type=Path, nargs="+", default=[Path("coverage/rust.info")])
    parser.add_argument("--details", action="store_true", help="List uncovered Rust lines and source functions")
    parser.add_argument("--snapshot", type=Path, help="Write fingerprints of the sources and locked dependencies")
    parser.add_argument("--compare-source", type=Path, help="Require this prior snapshot to match before writing a report snapshot")
    args = parser.parse_args()
    if args.compare_source and not args.snapshot:
        parser.error("--compare-source requires --snapshot")
    fingerprints = source_fingerprints()
    if args.snapshot:
        if args.compare_source and json.loads(args.compare_source.read_text(encoding="utf-8")) != fingerprints:
            print("Application source changed during the run; rerun coverage on the current source.")
            return 1
        args.snapshot.parent.mkdir(parents=True, exist_ok=True)
        args.snapshot.write_text(json.dumps(fingerprints, indent=2) + "\n", encoding="utf-8")
        return 0
    for source in {p.with_name("source.json") for p in [args.frontend, *args.backend]}:
        if not source.is_file() or json.loads(source.read_text(encoding="utf-8")) != fingerprints:
            print(f"{source}: missing or stale source snapshot; rerun coverage on the current source.")
            return 1
    failures = check(args.frontend, args.backend, args.details)
    for failure in failures:
        print(failure)
    if not failures:
        print("All Rust/TypeScript application sources have 100% coverage.")
    return int(bool(failures))


if __name__ == "__main__":
    raise SystemExit(main())
