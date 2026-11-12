"""Summarise and chart a `speechify-bench quiet` run.

    uv run --with matplotlib python -I bench/quiet_report.py RESULTS.jsonl OUT_DIR

Reads only the JSON lines the bench wrote. The detector threshold is judged
on the "dev" half of the clips; every headline number and chart uses the
other ("report") half, so the threshold was not tuned on what is reported.
Writes summary.json, summary.md and four PNG charts to OUT_DIR.
"""

import json
import pathlib
import statistics
import sys

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

# Validated with the dataviz palette checker (light surface): old = orange, new = blue.
OLD, NEW = "#eb6834", "#2a78d6"
SURFACE, INK, MUTED, GRID = "#fcfcfb", "#0b0b0b", "#52514e", "#e4e3df"
CHOSEN = "0.35"
SECOND = "0.15"


def style(ax, title, subtitle):
    ax.set_facecolor(SURFACE)
    for side in ("top", "right"):
        ax.spines[side].set_visible(False)
    for side in ("left", "bottom"):
        ax.spines[side].set_color(GRID)
    ax.tick_params(colors=MUTED, labelsize=9)
    ax.grid(axis="y", color=GRID, linewidth=0.8)
    ax.set_axisbelow(True)
    ax.set_title(title, loc="left", fontsize=12, color=INK, pad=22, fontweight="bold")
    ax.text(0, 1.02, subtitle, transform=ax.transAxes, fontsize=9, color=MUTED)


def share(rows, test):
    return 100.0 * sum(1 for r in rows if test(r)) / len(rows) if rows else float("nan")


def group_label(row):
    level = row.get("level")
    numeric = isinstance(level, (int, float)) and not isinstance(level, bool)
    if row["kind"].startswith("whisper"):
        return f"whisper {int(level)}" if numeric else "whisper"
    if numeric:
        return f"{int(level)}"
    return "as recorded"


def tick(key):
    if key.lstrip("-").isdigit():
        return f"{key} dBFS"
    if key == "whisper":
        return "real whisper"
    if key.startswith("whisper "):
        return f"whisper\n{key.split()[1]} dBFS"
    return key


def main():
    results, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    rows = [json.loads(line) for line in results.read_text().splitlines() if line.strip()]
    dev = [r for r in rows if r["split"] == "dev"]
    report = [r for r in rows if r["split"] == "report"]
    speech = [r for r in report if r["label"] == "speech"]
    noise = [r for r in report if r["label"] == "nonspeech"]
    background = [r for r in report if r["label"] == "background-speech"]

    # Threshold trade-off on the development half.
    thresholds = sorted(dev[0]["sweep"], key=float) if dev else []
    sweep = {
        t: {
            "speech_kept": share([r for r in dev if r["label"] == "speech"], lambda r: r["sweep"][t]["kept"]),
            "noise_kept": share([r for r in dev if r["label"] == "nonspeech"], lambda r: r["sweep"][t]["kept"]),
        }
        for t in thresholds
    }

    def wer_of(group, key):
        values = [r[key]["wer"] for r in group if "wer" in r[key]]
        return 100.0 * statistics.mean(values) if values else None

    levels = {}
    for r in speech:
        levels.setdefault(group_label(r), []).append(r)
    order = [k for k in ("as recorded",) if k in levels]
    order += sorted((k for k in levels if k.lstrip("-").isdigit()), key=lambda k: -int(k))
    order += [k for k in ("whisper",) if k in levels]
    order += sorted((k for k in levels if k.startswith("whisper ")), key=lambda k: -int(k.split()[1]))
    by_level = {
        k: {
            "clips": len(levels[k]),
            "old_kept": share(levels[k], lambda r: r["old"]["kept"]),
            "new_kept": share(levels[k], lambda r: r["new"]["kept"]),
            "old_wer": wer_of(levels[k], "old"),
            "new_wer": wer_of(levels[k], "new"),
        }
        for k in order
    }
    kinds = {}
    for r in noise:
        kinds.setdefault(r["kind"], []).append(r)
    by_kind = {
        k: {
            "clips": len(v),
            "old_kept": share(v, lambda r: r["old"]["kept"]),
            "new_kept": share(v, lambda r: r["new"]["kept"]),
        }
        for k, v in sorted(kinds.items())
    }
    summary = {
        "clips": len(rows),
        "dev_clips": len(dev),
        "report_clips": len(report),
        "threshold": CHOSEN,
        "sweep_dev": sweep,
        "speech": {
            "clips": len(speech),
            "old_kept": share(speech, lambda r: r["old"]["kept"]),
            "new_kept": share(speech, lambda r: r["new"]["kept"]),
            "old_wer": wer_of(speech, "old"),
            "new_wer": wer_of(speech, "new"),
        },
        "nonspeech": {
            "clips": len(noise),
            "old_kept": share(noise, lambda r: r["old"]["kept"]),
            "new_kept": share(noise, lambda r: r["new"]["kept"]),
        },
        "background_speech": {"clips": len(background), "new_kept": share(background, lambda r: r["new"]["kept"])},
        "by_level": by_level,
        "by_kind": by_kind,
        "detector_ms_median": statistics.median(r["new"]["ms"] for r in rows) if rows else None,
        "asr_ms_median": statistics.median(r["new"]["asr_ms"] for r in speech if r["new"].get("asr_ms")) if speech else None,
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")

    plt.rcParams.update({"font.family": "DejaVu Sans", "figure.facecolor": SURFACE})

    if not order or not by_kind:
        print("Not enough clips in the report half to chart", file=sys.stderr)
        return

    # 1. Speech kept, by loudness.
    fig, ax = plt.subplots(figsize=(8, 4.2), dpi=150)
    style(ax, "Quiet speech the app keeps", f"Share of speech recordings passed to the recogniser, by speech level (report half, {len(speech)} clips)")
    xs = range(len(order))
    for key, color, name, lift in (("old_kept", OLD, "Before: loudness trim", -12), ("new_kept", NEW, "After: voice detector + gain", 6)):
        ys = [by_level[k][key] for k in order]
        ax.plot(xs, ys, color=color, linewidth=2, marker="o", markersize=7, label=name, markeredgecolor=SURFACE, markeredgewidth=2)
        ax.annotate(f"{ys[-1]:.0f}%", (len(order) - 1, ys[-1]), textcoords="offset points", xytext=(8, lift), color=INK, fontsize=9)
    ax.set_xticks(list(xs), [tick(k) for k in order])
    ax.set_ylim(0, 105)
    ax.set_ylabel("Speech kept (%)", color=MUTED, fontsize=9)
    ax.set_xlabel("Quieter →", color=MUTED, fontsize=9, loc="right")
    ax.legend(frameon=False, fontsize=9, loc="lower left")
    fig.tight_layout()
    fig.savefig(out / "speech-kept.png", facecolor=SURFACE)
    plt.close(fig)

    # 2. Recognition accuracy, by loudness.
    fig, ax = plt.subplots(figsize=(8, 4.2), dpi=150)
    style(ax, "Word error rate as speech gets quieter", "Parakeet v3 on CPU; a dropped recording counts as every word missed (report half)")
    for key, color, name, lift in (("old_wer", OLD, "Before: loudness trim", 6), ("new_wer", NEW, "After: voice detector + gain", -12)):
        ys = [by_level[k][key] for k in order]
        ax.plot(xs, ys, color=color, linewidth=2, marker="o", markersize=7, label=name, markeredgecolor=SURFACE, markeredgewidth=2)
        ax.annotate(f"{ys[-1]:.0f}%", (len(order) - 1, ys[-1]), textcoords="offset points", xytext=(8, lift), color=INK, fontsize=9)
    ax.set_xticks(list(xs), [tick(k) for k in order])
    ax.set_ylim(0, max(105, max((v for k in order for v in (by_level[k]["old_wer"] or 0, by_level[k]["new_wer"] or 0)), default=0) + 5))
    ax.set_ylabel("Word error rate (%), lower is better", color=MUTED, fontsize=9)
    ax.set_xlabel("Quieter →", color=MUTED, fontsize=9, loc="right")
    ax.legend(frameon=False, fontsize=9, loc="upper left")
    fig.tight_layout()
    fig.savefig(out / "word-error-rate.png", facecolor=SURFACE)
    plt.close(fig)

    # 3. Non-speech wrongly kept, by kind of sound.
    names = list(by_kind)
    fig, ax = plt.subplots(figsize=(8, max(3.2, 0.42 * len(names) + 1.6)), dpi=150)
    style(ax, "Sounds that are not speech", f"Share wrongly passed to the recogniser; lower is better (report half, {len(noise)} clips)")
    ax.grid(axis="x", color=GRID, linewidth=0.8)
    ax.grid(axis="y", visible=False)
    height = 0.36
    for offset, key, color, name in ((-height / 2 - 0.01, "old_kept", OLD, "Before"), (height / 2 + 0.01, "new_kept", NEW, "After")):
        values = [by_kind[n][key] for n in names]
        bars = ax.barh([i + offset for i in range(len(names))], values, height=height, color=color, label=name)
        for bar, value in zip(bars, values):
            ax.text(bar.get_width() + 1, bar.get_y() + bar.get_height() / 2, f"{value:.0f}%", va="center", fontsize=8, color=INK, fontfamily="DejaVu Sans")
    ax.set_yticks(range(len(names)), [n.replace("_", " ") for n in names], color=INK, fontsize=9)
    ax.invert_yaxis()
    ax.set_xlim(0, 110)
    ax.set_xlabel("Wrongly kept (%)", color=MUTED, fontsize=9)
    ax.legend(frameon=False, fontsize=9, loc="lower right")
    fig.tight_layout()
    fig.savefig(out / "non-speech.png", facecolor=SURFACE)
    plt.close(fig)

    # 4. Threshold trade-off on the development half (why 0.35).
    if thresholds:
        fig, ax = plt.subplots(figsize=(8, 4.2), dpi=150)
        dev_speech = [r for r in dev if r["label"] == "speech"]
        dev_noise = [r for r in dev if r["label"] == "nonspeech"]
        rule_speech = share(dev_speech, lambda r: r["new"]["kept"])
        rule_noise = share(dev_noise, lambda r: r["new"]["kept"])
        style(
            ax,
            "Choosing the detector thresholds",
            f"Development half only ({len(dev)} clips). The app's two-step rule here: {rule_speech:.0f}% speech kept, {rule_noise:.0f}% non-speech",
        )
        tx = [float(t) for t in thresholds]
        ax.plot(tx, [sweep[t]["speech_kept"] for t in thresholds], color=NEW, linewidth=2, marker="o", markersize=7, label="Speech kept (higher is better)", markeredgecolor=SURFACE, markeredgewidth=2)
        ax.plot(tx, [sweep[t]["noise_kept"] for t in thresholds], color=OLD, linewidth=2, marker="o", markersize=7, label="Non-speech wrongly kept (lower is better)", markeredgecolor=SURFACE, markeredgewidth=2)
        ax.axvline(float(CHOSEN), color=MUTED, linewidth=1, linestyle=(0, (3, 3)))
        ax.axvline(float(SECOND), color=MUTED, linewidth=1, linestyle=(0, (1, 3)))
        ax.text(float(CHOSEN) + 0.008, 52, "first look", color=MUTED, fontsize=8)
        ax.text(float(SECOND) + 0.008, 52, "second look,\nonly with sustained sound", color=MUTED, fontsize=8)
        ax.set_ylim(0, 105)
        ax.set_xlabel("Speech probability threshold", color=MUTED, fontsize=9)
        ax.set_ylabel("Clips (%)", color=MUTED, fontsize=9)
        ax.legend(frameon=False, fontsize=9, loc="center right")
        fig.tight_layout()
        fig.savefig(out / "threshold.png", facecolor=SURFACE)
        plt.close(fig)

    examples_path = out / "examples.json"
    if examples_path.exists():
        examples(json.loads(examples_path.read_text()), out / "examples.png")

    lines = [
        f"Report half: {len(speech)} speech clips, {len(noise)} non-speech clips.",
        "",
        "| Speech level | Clips | Kept before | Kept after | WER before | WER after |",
        "|---|---:|---:|---:|---:|---:|",
    ]
    fmt = lambda v: "n/a" if v is None else f"{v:.1f}%"  # noqa: E731
    for k in order:
        b = by_level[k]
        label = tick(k)
        lines.append(f"| {label} | {b['clips']} | {fmt(b['old_kept'])} | {fmt(b['new_kept'])} | {fmt(b['old_wer'])} | {fmt(b['new_wer'])} |")
    lines += ["", "| Non-speech sound | Clips | Wrongly kept before | Wrongly kept after |", "|---|---:|---:|---:|"]
    for k, b in by_kind.items():
        lines.append(f"| {k.replace('_', ' ')} | {b['clips']} | {fmt(b['old_kept'])} | {fmt(b['new_kept'])} |")
    (out / "summary.md").write_text("\n".join(lines) + "\n")
    print(json.dumps({k: summary[k] for k in ("speech", "nonspeech", "background_speech", "detector_ms_median")}, indent=2))


def examples(rows, path):
    """Each recording's loudness over time, with what each method kept."""
    fig, axes = plt.subplots(len(rows), 1, figsize=(9, 1.95 * len(rows)), dpi=150)
    fig.suptitle("What each method kept, on real recordings", x=0.035, ha="left", fontsize=12, fontweight="bold", color=INK)
    for ax, row in zip(axes, rows):
        env = row["envelope_db"]
        times = [i * 0.02 for i in range(len(env))]
        floor = -90
        ax.fill_between(times, floor, env, color="#c3c2b7", linewidth=0)
        for key, color, y, name in (("old", OLD, -100, "Before"), ("new", NEW, -110, "After")):
            cut = row[key]
            if cut["kept"]:
                ax.plot([cut["start_s"], cut["end_s"]], [y, y], color=color, linewidth=6, solid_capstyle="butt")
            heard = cut["text"] if cut["kept"] else "(rejected as no speech)"
            if cut["kept"] and not heard:
                heard = "(kept; recogniser heard nothing)"
            ax.text(row["duration_s"] + 0.15, y, f"{name}: {heard[:70]}", va="center", fontsize=7.5, color=INK)
        level = row["level"]
        level = f"{level} dBFS" if isinstance(level, (int, float)) and level != 0 else "as recorded"
        kind = row["kind"].replace("_", " ").replace("soft-speech", "speech, made quieter")
        said = f'  said: "{row["reference"][:60]}"' if row["reference"] else ""
        ax.set_title(f"{kind}, {level}{said}", loc="left", fontsize=8.5, color=INK, pad=3)
        ax.set_xlim(0, max(row["duration_s"], 0.1) * 2.1)
        ax.set_ylim(-116, -5)
        ax.set_yticks([-80, -60, -40, -20])
        ax.tick_params(colors=MUTED, labelsize=7)
        for side in ("top", "right"):
            ax.spines[side].set_visible(False)
        for side in ("left", "bottom"):
            ax.spines[side].set_color(GRID)
        ax.set_xticks([t for t in range(int(row["duration_s"]) + 1)])
        ax.set_facecolor(SURFACE)
    axes[-1].set_xlabel("Seconds (grey: loudness in dBFS; bars: the part passed to the recogniser)", color=MUTED, fontsize=8)
    fig.tight_layout(rect=(0, 0, 1, 0.97))
    fig.savefig(path, facecolor=SURFACE)
    plt.close(fig)


if __name__ == "__main__":
    main()
