import argparse
import json
from datetime import datetime, timezone
from pathlib import Path

MARKER = "<!-- speechify-performance -->"
SCHEMA = 2
PLATFORMS = [("linux", "Linux"), ("macos", "macOS"), ("windows", "Windows")]
KINDS = ["corrections", "repetition", "lists", "paragraphs", "fillers", "formatting", "keep as said"]
NOISE = 0.15


def load_platforms(directory):
    found = {}
    for path in sorted(Path(directory).rglob("*.json")):
        data = json.loads(path.read_text(encoding="utf-8"))
        if isinstance(data, dict) and data.get("schema") == SCHEMA and "platform" in data:
            found[data["platform"]] = data
    return found


def ms(value):
    if value >= 10_000:
        return f"{value / 1000:.1f} s"
    if value >= 100:
        return f"{value:.0f} ms"
    return f"{value:.2f} ms"


def times(value):
    return f"{value:.2f}× real time" if value < 10 else f"{value:.0f}× real time"


def percent(value):
    return f"{value * 100:.1f}%"


def transcription(platform):
    t = platform["transcription"]
    return [
        ("Load the speech model", t["load_ms"], ms, "time", 20.0),
        ("Time to text", t["time_ms"], ms, "time", 20.0),
        ("Speed", t["realtime"], times, "rate", 0.5),
        ("Words wrong", t["wer"], percent, "error", 0.005),
    ]


def cleanup(platform):
    c = platform["cleanup"]
    rows = [("Rules, every scenario once", c["rules_ms"], ms, "time", 0.2)]
    llm = c.get("llm")
    if llm:
        rows += [
            ("Language model, typical call", llm["median_ms"], ms, "time", 200.0),
            ("Language model, slowest 10% of calls", llm["p90_ms"], ms, "time", 200.0),
            ("Answers rejected, rules used instead", llm["rejected"], lambda v: f"{v:.0f}/{llm['calls']}", "count", 0.5),
        ]
    return rows


def change(now, before, kind, floor):
    if before is None:
        return "", None
    worse = {"time": now > before, "rate": now < before, "error": now > before, "count": now > before}[kind]
    if kind in ("error", "count"):
        delta = now - before
        text = f" ({delta * 100:+.1f} pts)" if kind == "error" else f" ({delta:+.0f})"
        flagged = abs(delta) > floor
    else:
        ratio = now / before - 1 if before else 0
        text = f" ({ratio:+.0%})"
        flagged = abs(ratio) > NOISE and abs(now - before) > floor
    if not flagged or now == before:
        return (text if now != before else ""), None
    return text.replace(")", " ⚠️)" if worse else " 🟢)"), "worse" if worse else "better"


def metric_table(section, current, baseline, flags):
    columns = [(key, label) for key, label in PLATFORMS if key in current]
    names = []
    for key, _ in columns:
        for row in section(current[key]):
            if row[0] not in names:
                names.append(row[0])
    lines = ["| | " + " | ".join(label for _, label in columns) + " |", "| --- |" + " ---: |" * len(columns)]
    for name in names:
        cells = []
        for key, label in columns:
            now = {row[0]: row for row in section(current[key])}.get(name)
            before = {row[0]: row[1] for row in section(baseline[key])}.get(name) if key in baseline else None
            if now is None:
                cells.append("—")
                continue
            _, value, show, kind, floor = now
            text, flag = change(value, before, kind, floor)
            if flag == "worse":
                flags.append(f"{name} on {label}")
            cells.append(show(value) + text)
        lines.append(f"| {name} | " + " | ".join(cells) + " |")
    return lines


def tally(platform, path):
    counts = {}
    for s in platform.get("scenarios", []):
        result = s.get(path)
        if result is None:
            continue
        passed, total = counts.get(s["kind"], (0, 0))
        counts[s["kind"]] = (passed + bool(result["passed"]), total + 1)
    return counts


def kinds(*platforms):
    seen = {s["kind"] for p in platforms for s in p.get("scenarios", [])}
    return [k for k in KINDS if k in seen] + sorted(seen - set(KINDS))


def scenario_table(current, baseline):
    columns = [(key, label) for key, label in PLATFORMS if key in current]
    lines = [
        "| | " + " | ".join(f"{label}: rules / with model" for _, label in columns) + " |",
        "| --- |" + " ---: |" * len(columns),
    ]
    for kind in kinds(*current.values()):
        cells = []
        for key, _ in columns:
            parts = []
            for path in ("rules", "llm"):
                passed, total = tally(current[key], path).get(kind, (0, 0))
                if not total:
                    parts.append("—")
                    continue
                cell = f"{passed}/{total}"
                if key in baseline:
                    before = tally(baseline[key], path).get(kind)
                    if before and before[0] != passed:
                        cell += f" ({passed - before[0]:+d})"
                parts.append(cell)
            cells.append(" / ".join(parts))
        lines.append(f"| {kind.capitalize()} | " + " | ".join(cells) + " |")
    return lines


def results(platform, path):
    return {s["id"]: s[path]["passed"] for s in platform.get("scenarios", []) if s.get(path)}


def flips(current, baseline):
    changes = {}
    for key, label in PLATFORMS:
        if key not in current or key not in baseline:
            continue
        for path, name in [("rules", "rules only"), ("llm", "with the language model")]:
            now, before = results(current[key], path), results(baseline[key], path)
            for scenario in sorted(now.keys() & before.keys()):
                if now[scenario] != before[scenario]:
                    state = "now passes" if now[scenario] else "now fails"
                    changes.setdefault((scenario, name, state), []).append(label)
    return [
        f"- {'🟢' if state == 'now passes' else '❌'} `{scenario}` {state} {name} on {', '.join(labels)}"
        for (scenario, name, state), labels in sorted(changes.items())
    ]


def failures(current):
    rows = {}
    for key, label in PLATFORMS:
        for s in current.get(key, {}).get("scenarios", []):
            for path, name in [("rules", "rules"), ("llm", "with model")]:
                result = s.get(path)
                if not result or result["passed"]:
                    continue
                entry = (s["kind"], s["id"], name, result["text"], "; ".join(result["failures"]))
                rows.setdefault(entry, (s["raw"], []))[1].append(label)
    if not rows:
        return []
    lines = ["| Kind | Scenario | Path | Platforms | Said | Got | Why |", "| --- | --- | --- | --- | --- | --- | --- |"]
    order = {k: i for i, k in enumerate(KINDS)}
    for (kind, scenario, name, text, why), (raw, labels) in sorted(
        rows.items(), key=lambda item: (order.get(item[0][0], 99), item[0][1:])
    ):
        got = text.replace("\n", "⏎").replace("|", "\\|") or "(nothing)"
        lines.append(f"| {kind} | `{scenario}` | {name} | {', '.join(labels)} | {raw} | {got} | {why} |")
    return lines


def summary(current):
    out = {}
    for key, platform in current.items():
        out[key] = {
            "transcription": {name: round(value, 4) for name, value, *_ in transcription(platform)},
            "cleanup": {name: round(value, 4) for name, value, *_ in cleanup(platform)},
            "scenarios_failing": {
                path: sorted(id for id, passed in results(platform, path).items() if not passed)
                for path in ("rules", "llm")
                if results(platform, path)
            },
        }
    return out


def models(platforms):
    for platform in platforms.values():
        return platform.get("models", {})
    return {}


def render(current, baseline_doc, run_url, results_path):
    baseline = (baseline_doc or {}).get("platforms", {})
    lines = [MARKER, "## Performance", ""]
    if baseline_doc:
        source = f"#{baseline_doc['pr']}" if baseline_doc.get("pr") else "main"
        sha = (baseline_doc.get("head_sha") or "")[:7]
        lines.append(f"Compared with the latest results on `main` (from {source}, `{sha}`).")
    else:
        lines.append("There are no comparable results on `main` yet. This run becomes the baseline once it is merged.")
    lines.append(
        f"Timings come from shared GitHub runners: changes within ±{NOISE:.0%} are usually noise, "
        "so only larger ones are marked (⚠️ worse, 🟢 better)."
    )
    missing = [label for key, label in PLATFORMS if key not in current]
    if missing:
        lines += ["", f"⚠️ No results from {', '.join(missing)}: that job failed or was cancelled."]
    seconds = next((round(p["transcription"]["audio_s"]) for p in current.values()), 0)
    flags = []
    lines += ["", "### 1. Transcription", "", f"Speech to raw text, for one recording of {seconds} s.", ""]
    lines += metric_table(transcription, current, baseline, flags)
    lines += ["", "### 2. Cleanup", "", "Raw text to the text that gets typed.", ""]
    lines += metric_table(cleanup, current, baseline, flags)
    lines += ["", "### 3. Scenarios", "", "Scenarios passed in each group, by the rules alone and with the language model.", ""]
    lines += scenario_table(current, baseline)
    changed = flips(current, baseline)
    if changed:
        lines += ["", "#### Changes from main", ""] + changed
    lines += ["", "### Verdict", ""]
    worse = [line for line in changed if "now fails" in line]
    if not baseline_doc:
        lines.append("Nothing to compare with yet.")
    elif not flags and not worse:
        lines.append("No regressions beyond the noise margin.")
    else:
        if flags:
            lines.append(f"Worse than main: {'; '.join(flags)}.")
        if worse:
            lines.append(f"{len(worse)} scenario result(s) got worse; see Changes from main.")
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
    ]
    used = models(current)
    footer = (
        f"Models are the defaults in `core/defaults.json`: speech `{used.get('speech')}`, "
        f"language model `{used.get('cleanup')}`."
    )
    before = models(baseline)
    if baseline_doc and before and (before.get("speech"), before.get("cleanup_digest")) != (
        used.get("speech"),
        used.get("cleanup_digest"),
    ):
        footer += f" `main` was measured with speech `{before.get('speech')}`, language model `{before.get('cleanup')}`."
    lines.append(footer)
    lines.append(
        "Scenarios: `bench/scenarios.json`. Cleanup prompt: `core/src/prompts/cleanup.txt`. "
        f"Once every check passes, CI records these results in `{results_path}`."
    )
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
        loaded = json.loads(Path(args.baseline).read_text(encoding="utf-8"))
        if loaded.get("schema") == SCHEMA:
            baseline = loaded
    record = {
        "schema": SCHEMA,
        "pr": args.pr,
        "head_sha": args.head_sha,
        "recorded_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "run_url": args.run_url,
        "platforms": current,
    }
    Path(args.out_json).write_text(json.dumps(record, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    results_path = f"bench/perf/results/pr-{args.pr}.json"
    Path(args.out_md).write_text(render(current, baseline, args.run_url, results_path), encoding="utf-8")


if __name__ == "__main__":
    main()
