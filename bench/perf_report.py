import argparse
import json
import sys
from datetime import datetime, timezone
from pathlib import Path
from string import Template

SCHEMA = 2
PLATFORMS = [("linux", "Linux"), ("macos", "macOS"), ("windows", "Windows")]
KINDS = ["corrections", "repetition", "lists", "paragraphs", "fillers", "formatting", "keep as said"]
NOISE = 0.15
COMMENT_LIMIT = 60_000
FAILURE_ROWS = 40
CELL_CHARS = 300
TEMPLATE = Path(__file__).with_name("perf_comment.md")


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
        ("load_ms", "Load the speech model", t["load_ms"], ms, "time", 20.0),
        ("time_ms", "Time to text", t["time_ms"], ms, "time", 20.0),
        ("realtime", "Speed", t["realtime"], times, "rate", 0.5),
        ("wer", "Words wrong", t["wer"], percent, "error", 0.005),
    ]


def cleanup(platform):
    c = platform["cleanup"]
    rows = [("rules_ms", "Rules, every scenario once", c["rules_ms"], ms, "time", 0.2)]
    llm = c.get("llm")
    if llm:
        rows += [
            ("llm_median_ms", "Language model, typical call", llm["median_ms"], ms, "time", 200.0),
            ("llm_p90_ms", "Language model, slowest 10% of calls", llm["p90_ms"], ms, "time", 200.0),
            (
                "llm_rejected",
                "Answers rejected, rules used instead",
                llm["rejected"],
                lambda v: f"{v:.0f}/{llm['calls']}",
                "count",
                0.5,
                llm["calls"],
            ),
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
    order = {}
    for key, _ in columns:
        for row in section(current[key]):
            order.setdefault(row[0], row[1])
    lines = ["| | " + " | ".join(label for _, label in columns) + " |", "| --- |" + " ---: |" * len(columns)]
    for metric, name in order.items():
        cells = []
        for key, label in columns:
            now = {row[0]: row for row in section(current[key])}.get(metric)
            old = {row[0]: row for row in section(baseline[key])}.get(metric) if key in baseline else None
            if now is None:
                cells.append("—")
                continue
            _, _, value, show, kind, floor, *basis = now
            if old and list(old[6:]) != basis:
                cells.append(f"{show(value)} (main: {old[3](old[2])})")
                continue
            text, flag = change(value, old[2] if old else None, kind, floor)
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
                text = f"{passed}/{total}"
                before = tally(baseline[key], path).get(kind) if key in baseline else None
                if before and before[1] != total:
                    text += f" (was {before[0]}/{before[1]})"
                elif before and before[0] != passed:
                    text += f" ({passed - before[0]:+d})"
                parts.append(text)
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
                    changes.setdefault((scenario, name, now[scenario]), []).append(label)
    return [(scenario, name, passes, labels) for (scenario, name, passes), labels in sorted(changes.items())]


def flip_line(scenario, name, passes, labels):
    state = "now passes" if passes else "now fails"
    return f"- {'🟢' if passes else '❌'} `{scenario}` {state} {name} on {', '.join(labels)}"


def cell(text):
    text = text.replace("\n", "⏎").replace("|", "\\|")
    return text if len(text) <= CELL_CHARS else text[: CELL_CHARS - 1] + "…"


def failures(current, limit=None):
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
    ranked = sorted(rows.items(), key=lambda item: (order.get(item[0][0], 99), item[0][1:]))
    for (kind, scenario, name, text, why), (raw, labels) in ranked[:limit]:
        got = cell(text) or "(nothing)"
        lines.append(f"| {kind} | `{scenario}` | {name} | {', '.join(labels)} | {cell(raw)} | {got} | {why} |")
    if limit is not None and len(ranked) > limit:
        lines += ["", f"…and {len(ranked) - limit} more in the run's job summary."]
    return lines


def summary(current):
    out = {}
    for key, platform in current.items():
        out[key] = {
            "transcription": {metric: round(value, 4) for metric, _, value, *_ in transcription(platform)},
            "cleanup": {metric: round(value, 4) for metric, _, value, *_ in cleanup(platform)},
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


def block(*lines):
    return "\n".join(["", *lines]) if lines else ""


def render(current, baseline_doc, run_url, release, skipped_schema=None, limit=None, with_json=True, head_sha=""):
    baseline = (baseline_doc or {}).get("platforms", {})
    if baseline_doc:
        sha = (baseline_doc.get("head_sha") or "")[:7]
        comparison = (
            f"Compared with `main` at `{sha}`, measured for the nightly release `{release}`. "
            "If this branch is behind `main`, changes merged since then show up here too."
        )
    elif skipped_schema is not None:
        comparison = (
            f"Not compared: the nightly release `{release}` has results in format {skipped_schema}, "
            f"and this run writes format {SCHEMA}. The next merge to `main` records results in the new format."
        )
    else:
        comparison = "No nightly release has performance results yet. The next merge to `main` records them."
    missing = [label for key, label in PLATFORMS if key not in current]
    flags = []
    transcription_rows = metric_table(transcription, current, baseline, flags)
    cleanup_rows = metric_table(cleanup, current, baseline, flags)
    changed = flips(current, baseline)
    worse = [flip for flip in changed if not flip[2]]
    if not baseline_doc:
        verdict = ["Nothing to compare with yet."]
    elif not flags and not worse:
        verdict = ["No regressions beyond the noise margin."]
    else:
        verdict = []
        if flags:
            verdict.append(f"Worse than main: {'; '.join(flags)}.")
        if worse:
            verdict.append(f"{len(worse)} scenario result(s) got worse; see Changes from main.")
    failing = failures(current, limit)
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
    values = {
        "commit": head_sha[:7] or "this branch",
        "comparison": comparison,
        "noise": f"{NOISE:.0%}",
        "missing": block("", f"⚠️ No results from {', '.join(missing)}: that job failed or was cancelled.") if missing else "",
        "seconds": next((round(p["transcription"]["audio_s"]) for p in current.values()), 0),
        "transcription": "\n".join(transcription_rows),
        "cleanup": "\n".join(cleanup_rows),
        "scenarios": "\n".join(scenario_table(current, baseline)),
        "changes": block("", "#### Changes from main", "", *[flip_line(*flip) for flip in changed]) if changed else "",
        "verdict": "\n".join(verdict),
        "failures": block(
            "", "<details><summary>Scenarios that fail on this branch</summary>", "", *failing, "", "</details>"
        )
        if failing
        else "",
        "json": block(
            "",
            "<details><summary>Summary as JSON</summary>",
            "",
            "```json",
            json.dumps(summary(current), indent=1, ensure_ascii=False),
            "```",
            "",
            "</details>",
        )
        if with_json
        else "",
        "models": footer,
        "run": f"[Run]({run_url})" if run_url else "",
    }
    body = Template(TEMPLATE.read_text(encoding="utf-8")).substitute(values)
    lines = []
    for line in body.splitlines():
        if line or (lines and lines[-1]):
            lines.append(line)
    return "\n".join(lines).strip("\n") + "\n"


def comment(current, baseline_doc, run_url, release, skipped_schema, head_sha=""):
    for limit, with_json in ((FAILURE_ROWS, True), (FAILURE_ROWS, False), (0, False)):
        body = render(current, baseline_doc, run_url, release, skipped_schema, limit, with_json, head_sha)
        if len(body) <= COMMENT_LIMIT:
            break
    return body


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("results")
    parser.add_argument("--baseline")
    parser.add_argument("--baseline-release", default="")
    parser.add_argument("--pr", type=int)
    parser.add_argument("--complete", action="store_true")
    parser.add_argument("--head-sha", required=True)
    parser.add_argument("--run-url", default="")
    parser.add_argument("--out-json", required=True)
    parser.add_argument("--out-md", required=True)
    parser.add_argument("--out-summary")
    args = parser.parse_args()
    current = load_platforms(args.results)
    missing = [label for key, label in PLATFORMS if key not in current]
    if args.complete and missing:
        sys.exit(f"No results from {', '.join(missing)}")
    baseline = None
    skipped_schema = None
    if args.baseline and Path(args.baseline).is_file():
        loaded = json.loads(Path(args.baseline).read_text(encoding="utf-8"))
        if loaded.get("schema") == SCHEMA:
            baseline = loaded
        else:
            skipped_schema = loaded.get("schema")
    record = {
        "schema": SCHEMA,
        "pr": args.pr,
        "head_sha": args.head_sha,
        "recorded_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "run_url": args.run_url,
        "platforms": current,
    }
    Path(args.out_json).write_text(json.dumps(record, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    release = args.baseline_release
    Path(args.out_md).write_text(comment(current, baseline, args.run_url, release, skipped_schema, args.head_sha), encoding="utf-8")
    if args.out_summary:
        Path(args.out_summary).write_text(
            render(current, baseline, args.run_url, release, skipped_schema, head_sha=args.head_sha), encoding="utf-8"
        )


if __name__ == "__main__":
    main()
