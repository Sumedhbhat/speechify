import argparse
import json
from datetime import datetime, timezone
from pathlib import Path

MARKER = "<!-- speechify-performance -->"
PLATFORMS = [("linux", "Linux"), ("macos", "macOS"), ("windows", "Windows")]
NOISE = 0.15


def load_platforms(directory):
    found = {}
    for path in sorted(Path(directory).rglob("*.json")):
        data = json.loads(path.read_text(encoding="utf-8"))
        if isinstance(data, dict) and data.get("schema") == 1 and "platform" in data:
            found[data["platform"]] = data
    return found


def metrics(platform):
    rows = []
    for speech in platform.get("speech", []):
        name = speech["model"]
        seconds = round(speech["audio_s"])
        rows.append((f"Dictation, {name} ({seconds} s clip)", speech["dictation_ms"], 20.0))
        rows.append((f"Model load, {name}", speech["load_ms"], 20.0))
    rows.append(("Formatter, all scenarios once", platform["rules"]["pass_ms"], 0.2))
    llm = platform.get("llm")
    if llm:
        rows.append((f"Cleanup model, median call ({llm['model']})", llm["median_ms"], 200.0))
        rows.append(("Cleanup model, slowest 10% of calls", llm["p90_ms"], 200.0))
    return rows


def duration(value):
    if value >= 10_000:
        return f"{value / 1000:.1f} s"
    if value >= 100:
        return f"{value:.0f} ms"
    return f"{value:.2f} ms"


def change(now, before, floor):
    if before is None or before <= 0:
        return "", None
    ratio = now / before - 1
    if abs(ratio) > NOISE and abs(now - before) > floor:
        flag = "slower" if ratio > 0 else "faster"
    else:
        flag = None
    mark = {"slower": " ⚠️", "faster": " 🟢"}.get(flag, "")
    return f" ({ratio:+.0%}{mark})", flag


def passing(platform, path):
    return {s["id"] for s in platform.get("scenarios", []) if (s.get(path) or {}).get("passed")}


def scenario_ids(platform, path):
    return [s["id"] for s in platform.get("scenarios", []) if s.get(path) is not None]


def speed_table(current, baseline, flags):
    columns = [(key, label) for key, label in PLATFORMS if key in current]
    names = []
    for key, _ in columns:
        for name, _, _ in metrics(current[key]):
            if name not in names:
                names.append(name)
    lines = ["| | " + " | ".join(label for _, label in columns) + " |", "| --- |" + " ---: |" * len(columns)]
    for name in names:
        cells = []
        for key, label in columns:
            now = {n: (v, f) for n, v, f in metrics(current[key])}.get(name)
            before = {n: v for n, v, _ in metrics(baseline[key])}.get(name) if key in baseline else None
            if now is None:
                cells.append("—")
                continue
            text, flag = change(now[0], before, now[1])
            if flag:
                flags.append((flag, f"{name} on {label}"))
            cells.append(duration(now[0]) + text)
        lines.append(f"| {name} | " + " | ".join(cells) + " |")
    return lines


def scenario_table(current, baseline):
    columns = [(key, label) for key, label in PLATFORMS if key in current]
    lines = ["| | " + " | ".join(label for _, label in columns) + " |", "| --- |" + " ---: |" * len(columns)]
    for path, name in [("rules", "Rules only"), ("llm", "With the cleanup model")]:
        cells = []
        for key, _ in columns:
            ids = scenario_ids(current[key], path)
            if not ids:
                cells.append("—")
                continue
            count = len(passing(current[key], path) & set(ids))
            cell = f"{count}/{len(ids)}"
            if key in baseline and scenario_ids(baseline[key], path):
                before = len(passing(baseline[key], path))
                if count != before:
                    cell += f" ({count - before:+d})"
            cells.append(cell)
        lines.append(f"| {name} | " + " | ".join(cells) + " |")
    return lines


def flips(current, baseline):
    changes = {}
    for key, label in PLATFORMS:
        if key not in current or key not in baseline:
            continue
        for path, name in [("rules", "rules only"), ("llm", "cleanup model")]:
            if not scenario_ids(baseline[key], path) or not scenario_ids(current[key], path):
                continue
            now, before = passing(current[key], path), passing(baseline[key], path)
            shared = set(scenario_ids(current[key], path)) & set(scenario_ids(baseline[key], path))
            for scenario in sorted(shared):
                if (scenario in now) != (scenario in before):
                    state = "now passes" if scenario in now else "now fails"
                    changes.setdefault((scenario, name, state), []).append(label)
    lines = []
    for (scenario, name, state), labels in sorted(changes.items()):
        mark = "🟢" if state == "now passes" else "❌"
        lines.append(f"- {mark} `{scenario}` {state} with {name} on {', '.join(labels)}")
    return lines


def failures(current):
    rows = {}
    for key, label in PLATFORMS:
        for scenario in current.get(key, {}).get("scenarios", []):
            for path, name in [("rules", "rules"), ("llm", "model")]:
                result = scenario.get(path)
                if not result or result["passed"]:
                    continue
                entry = (scenario["id"], name, result["text"], "; ".join(result["failures"]))
                rows.setdefault(entry, (scenario["kind"], scenario["raw"], []))[2].append(label)
    lines = ["| Scenario | Path | Platforms | Said | Got | Why |", "| --- | --- | --- | --- | --- | --- |"]
    for (scenario, name, text, why), (kind, raw, labels) in sorted(rows.items()):
        got = text.replace("\n", "⏎").replace("|", "\\|") or "(nothing)"
        lines.append(f"| `{scenario}` ({kind}) | {name} | {', '.join(labels)} | {raw} | {got} | {why} |")
    return lines if rows else []


def summary(current):
    out = {}
    for key, platform in current.items():
        out[key] = {
            "metrics_ms": {name: round(value, 3) for name, value, _ in metrics(platform)},
            "failing": {
                path: sorted(set(scenario_ids(platform, path)) - passing(platform, path))
                for path in ("rules", "llm")
                if scenario_ids(platform, path)
            },
        }
    return out


def render(current, baseline_doc, pr, run_url, results_path):
    baseline = (baseline_doc or {}).get("platforms", {})
    lines = [MARKER, "## Performance and cleanup scenarios", ""]
    if baseline_doc:
        source = f"#{baseline_doc['pr']}" if baseline_doc.get("pr") else "main"
        sha = (baseline_doc.get("head_sha") or "")[:7]
        lines.append(f"Compared with the latest results on `main` (from {source}, `{sha}`).")
    else:
        lines.append("There are no results on `main` yet, so nothing is compared. This run becomes the baseline once it is merged.")
    lines.append(
        f"Timings come from shared GitHub runners: changes within ±{NOISE:.0%} are usually noise, "
        "so only larger ones are marked (⚠️ slower, 🟢 faster)."
    )
    missing = [label for key, label in PLATFORMS if key not in current]
    if missing:
        lines += ["", f"⚠️ No results from {', '.join(missing)}: that job failed or was cancelled."]
    flags = []
    lines += ["", "### Speed", ""] + speed_table(current, baseline, flags)
    lines += ["", "### Scenarios passed", ""] + scenario_table(current, baseline)
    changed = flips(current, baseline)
    if changed:
        lines += ["", "### Changes from main", ""] + changed
    slower = [what for flag, what in flags if flag == "slower"]
    lines += ["", "### Verdict", ""]
    regressions = [line for line in changed if "now fails" in line]
    if not baseline_doc:
        lines.append("No baseline to compare with yet.")
    elif not slower and not regressions:
        lines.append("No regressions beyond the noise margin.")
    else:
        if slower:
            lines.append(f"Slower than main: {'; '.join(slower)}.")
        if regressions:
            lines.append(f"{len(regressions)} scenario result(s) got worse; see Changes from main.")
    failing = failures(current)
    if failing:
        lines += ["", "<details><summary>Scenarios that fail on this branch</summary>", ""] + failing + ["", "</details>"]
    lines += [
        "",
        "<details><summary>Summary as JSON</summary>",
        "",
        "```json",
        json.dumps(summary(current), indent=1, ensure_ascii=False),
        "```",
        "",
        "</details>",
        "",
        f"Scenarios: `bench/scenarios.json`. Cleanup prompt: `core/src/prompts/cleanup.txt`. "
        f"After every check passes, CI records these results in `{results_path}`.",
    ]
    if run_url:
        lines.append(f"[Run]({run_url})")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("results")
    parser.add_argument("--baseline")
    parser.add_argument("--pr", type=int, required=True)
    parser.add_argument("--head-sha", required=True)
    parser.add_argument("--run-url", default="")
    parser.add_argument("--out-json", required=True)
    parser.add_argument("--out-md", required=True)
    args = parser.parse_args()
    current = load_platforms(args.results)
    baseline = None
    if args.baseline and Path(args.baseline).is_file():
        baseline = json.loads(Path(args.baseline).read_text(encoding="utf-8"))
    record = {
        "schema": 1,
        "pr": args.pr,
        "head_sha": args.head_sha,
        "recorded_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "run_url": args.run_url,
        "platforms": current,
    }
    Path(args.out_json).write_text(json.dumps(record, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    results_path = f"bench/perf/results/pr-{args.pr}.json"
    Path(args.out_md).write_text(render(current, baseline, args.pr, args.run_url, results_path), encoding="utf-8")


if __name__ == "__main__":
    main()
