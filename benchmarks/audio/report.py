"""Generate Markdown and a standalone HTML report from raw measurements."""
import datetime
import hashlib
import html
import json
from pathlib import Path
import statistics
import sys

folder = Path(__file__).resolve().parent
raw = json.loads((folder / 'results.json').read_text())
# Older runner snapshots did not record these fields; supplement transparently.
if 'source_sha256' not in raw['metadata']:
    raw['metadata']['report_generated_at_utc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    raw['metadata']['source_sha256_at_report_generation'] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in folder.iterdir() if p.name in ('python.py','rust.rs','go.go','typescript.ts','run.py')}
    (folder / 'results.json').write_text(json.dumps(raw,indent=2)+'\n')
runtimes = ['Python/NumPy','Rust','Go','TypeScript/Node','TypeScript/Bun']
rows=[]
for duration in (1,10,60):
    for task in ('silence','pcm16'):
        samples = {r: [m['ns_per_op']/1000 for m in raw['measurements'] if m['runtime']==r and m['seconds']==duration and m['task']==task] for r in runtimes}
        baseline = statistics.median(samples[runtimes[0]])
        for r in runtimes:
            med=statistics.median(samples[r])
            rows.append(dict(seconds=duration,task=task,runtime=r,us=med,min=min(samples[r]),max=max(samples[r]),speedup=baseline/med,latency_reduction=100*(1-med/baseline),throughput_increase=100*(baseline/med-1),saved_us=baseline-med))
md='''# Speechify audio preprocessing benchmark

Measured on 22 October 2026. **These are component microbenchmarks, not full application or speech-recognition benchmarks.** No complete Rust, Go or TypeScript application ports exist in this checkout. These results cannot establish which language would make the complete application faster.

The Go entry in the request was duplicated; Go is measured once. TypeScript is the same source executed separately by Node.js and Bun. Bun is a runtime, not a language.

## Workload and equivalence

- Silence detection: production `speechify.audio.is_silent`, RMS threshold 0.004, compared with equivalent ports.
- PCM16: the exact NumPy expression in production `save_wav`, `(np.clip(audio, -1.0, 1.0) * 32767).astype(np.int16).tobytes()`, compared with equivalent finite-input clipping, scaling, truncation and little-endian output. This excludes WAV headers and filesystem writes.
- Identical 16 kHz, mono, little-endian float32 input. The bundled LibriSpeech sample is truncated or cyclically repeated to 1, 10 and 60 seconds, without resampling.
- Fresh output is allocated each conversion. Python uses NumPy array operations; ports use fused loops. NumPy reduction and the ports' float64 accumulation of float32 squares have different rounding orders; equivalence is verified on the specified fixtures, not claimed for every possible boundary input.
- All five implementations passed byte-for-byte output checks for eight fixtures × two tasks: empty input, clipping/truncation edges, silence, below/above the silence threshold, and all three speech durations. This is 80 implementation/fixture/task checks, including the reference.

## Measurement method

Nine shuffled rounds, seed 20261022. Each job runs in a fresh process with 30 untimed warmup calls. Timed loop iterations are 6,000 / 600 / 100 for 1 / 10 / 60 seconds of audio respectively, identical across runtimes. Each round produces one mean duration per operation; reported central values are medians of the nine round means. Min–max shows the nine round means, not individual-operation tail latency.

Loading, validation, compilation, process startup, input reads, result writes and checksum checks are outside the timed interval. Allocations, conversion and runtime GC occurring during the loop are included. Jobs run serially. Numerical kernels use one calling thread, and OMP/OpenBLAS/MKL and Go parallelism limits are set to 1. CPU affinity is inherited, the governor and frequency are not fixed, and unrelated system activity is not controlled. Small differences should not be treated as decisive.

Speed ratio = Python median / candidate median. Values above 1 are faster. Latency reduction = (1 − candidate/Python) × 100%; negative means slower. Throughput increase = (speed ratio − 1) × 100%. Values are observed, rounded estimates, not exact guarantees about a language.

'''
for duration in (1,10,60):
    for task in ('silence','pcm16'):
        md+=f'## {duration}-second clip · {task}\n\n| Implementation | Median µs/op | Min–max µs/op | Speed ratio | Latency reduction | Throughput increase |\n|---|---:|---:|---:|---:|---:|\n'
        for r in rows:
            if r['seconds']==duration and r['task']==task:
                md+=f"| {r['runtime']} | {r['us']:.3f} | {r['min']:.3f}–{r['max']:.3f} | {r['speedup']:.3f}× | {r['latency_reduction']:+.2f}% | {r['throughput_increase']:+.2f}% |\n"
        md+='\n'
md+='''## What this establishes

At 10 seconds, Python/NumPy wins silence detection. Rust PCM conversion saves 0.166 ms (1.637×, 38.92% lower latency); Go saves 0.102 ms (1.314×, 23.91% lower latency). Both TypeScript runtimes are slower for both 10-second tasks. At 60 seconds, all ports win PCM conversion, with Rust saving 3.255 ms (3.050×, 67.22% lower latency). At 1 second, NumPy wins both tasks by the measured medians; near ties on silence detection overlap the observed variability.

There is no evidence here that a whole application rewrite is warranted. Speechify keeps its ASR model in a separate Python worker; keeping that backend and model configuration preserves the inference work regardless of host language. These tests exclude model inference, IPC, learned replacement rules, SQLite, microphone capture, UI, clipboard output, cold start, memory usage and model loading. The PCM saving also occurs after the app's printed paste-latency timer, so it should not be reported as a measured improvement to that timer. No model weights or engine environments were installed and this machine has no NVIDIA GPU. A full comparison needs equivalent application ports using the same model weights, backend/version, compute type, beam size, thread settings, corpus and output validation. No end-to-end speedup has been measured.

## Reproduce

Prerequisites: Python 3.12 with NumPy, Node.js with native TypeScript type stripping, Bun, Rust with a working linker, and Go. No third-party Rust/Go/TypeScript packages are needed. This run used NumPy 2.5.3, already selected by the repository lock for Python 3.12. The isolated environment was installed with uv's 14-day package cutoff; project manifests and lockfiles were not modified.

```bash
.venv/bin/python benchmarks/audio/run.py --go /path/to/go
.venv/bin/python benchmarks/audio/report.py
```

On this host, `cc` was absent, so Rust linked directly with GNU ld and the existing system C runtime. The exact additional linker flags are in results.json. Build scratch space was moved off quota-limited `/tmp` with `TMPDIR=/home/bhatsumedh/.cache/speechify-benchmark-tmp`. With a normal working C toolchain no linker override is needed. The Go 1.26.0 toolchain was downloaded to `/tmp/speechify-benchmark-tools/go`; an installed Go can be supplied instead.

## Environment and raw evidence

All 270 timing observations and validation hashes are in [results.json](results.json). Source implementations and the runner are in this directory. Numbers characterize these specific implementations, compiler settings and runtime versions; they are not a ranking of every possible implementation in these languages.

```json
'''+json.dumps(raw['metadata'],indent=2)+'\n```\n'
(folder/'REPORT.md').write_text(md)
content='''<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Speechify benchmark · 22 Oct 2026</title><style>
:root{color-scheme:light dark;font-family:system-ui,sans-serif}body{margin:0;color:var(--foreground,light-dark(#17212b,#e3eaf1));background:transparent;line-height:1.5}h2{font-size:24px;margin:0 0 8px}p{margin:8px 0 16px}.muted{color:var(--muted-foreground,light-dark(#586570,#a4b2bf));font-size:13px}.scope{padding:16px;border-radius:12px;background:var(--secondary,light-dark(#eef3f7,#1d2a35));margin:16px 0}label{display:inline-flex;gap:8px;align-items:center;margin:0 12px 12px 0}select{font:inherit;padding:6px;border:1px solid #8886;border-radius:6px;background:var(--background,light-dark(white,#17212b));color:inherit}.table{overflow:auto}table{border-collapse:collapse;width:100%;font-size:13px}th,td{text-align:right;padding:10px 7px;border-bottom:1px solid #8884;font-variant-numeric:tabular-nums;white-space:nowrap}th:first-child,td:first-child{text-align:left}th{font-weight:500}.bar{height:22px;background:var(--chart-1,#4285b5);border-radius:3px;min-width:2px}.plotrow{display:grid;grid-template-columns:140px 1fr 86px;align-items:center;gap:8px;margin:10px 0;font-size:13px}.plotrow span:last-child{text-align:right;font-variant-numeric:tabular-nums}#plot{margin:20px 0}details{margin:16px 0}pre{overflow:auto;font-size:12px}summary{cursor:pointer}a{color:var(--primary,#4285b5)}@media(max-width:480px){.plotrow{grid-template-columns:114px 1fr 70px;font-size:12px}h2{font-size:21px}}
</style></head><body>
<h2>Speechify: measured audio preprocessing</h2><p class="muted">22 October 2026 · Ryzen 5 3500U · 9 shuffled rounds · single calling thread</p>
<div class="scope"><strong>Component results only.</strong> These compare the current Python/NumPy audio kernels with Rust, Go and the same TypeScript source on Node.js and Bun. Full dictation and speech-model inference were not benchmarked. Go was listed twice in the request and is measured once.</div>
<label>Audio <select id="duration"><option value="1">1 second</option><option value="10" selected>10 seconds</option><option value="60">60 seconds</option></select></label><label>Operation <select id="task"><option value="silence">Silence detection</option><option value="pcm16" selected>PCM16 conversion</option></select></label>
<div id="plot" aria-label="Median operation times; shorter is faster"></div><p class="muted">Median µs per operation. Shorter bars are faster.</p>
<div class="table"><table><thead><tr><th>Implementation</th><th>Median µs</th><th>Min–max µs</th><th>Speed ratio</th><th>Latency reduction</th></tr></thead><tbody id="rows"></tbody></table></div>
<p class="muted">Speed ratio = Python time ÷ candidate time. Above 1× is faster; negative latency reduction means slower. Min–max is across nine round means.</p>
<p><strong>Measured takeaway:</strong> Rust and Go improve 10-second PCM conversion by 0.166 ms and 0.102 ms respectively. Python wins 10-second silence detection. At 60 seconds Rust PCM is 3.050× faster, saving 3.255 ms. This does not measure a whole-application speedup.</p>
<details><summary>Identical inputs, timing and correctness</summary><p>16 kHz mono float32. The bundled speech recording is truncated or repeated to each duration. RMS threshold 0.004; PCM16 clips to [−1, 1], scales by 32767 and truncates. All five outputs match Python on eight fixtures for each task, including empty audio, clipping, silence and threshold cases.</p><p>30 warmup calls per process; 6,000 / 600 / 100 timed iterations for 1 / 10 / 60 seconds. Each row is a median of nine round means. Build, startup, reads and writes are excluded. Output allocations and GC during loops are included. Rust uses optimized x86-64 code; Go default optimizations; TypeScript uses runtime JITs. NumPy uses native kernels; ports use fused loops. Reduction rounding orders differ. CPU frequency and unrelated system activity were not controlled.</p><p>Excluded: ASR, IPC, UI, microphone, clipboard, learning database and WAV file writes. The application saves WAV after its printed paste-latency timer. No speech models were installed; no NVIDIA GPU is available. Full-app rankings require complete equivalent ports and the same ASR backend and model configuration.</p></details>
<details><summary>Runtime versions and reproduction</summary><pre id="meta"></pre><p>Run <code>.venv/bin/python benchmarks/audio/run.py --go /path/to/go</code> in the checkout. Full methodology, all 270 observations and validation hashes are in <code>benchmarks/audio/REPORT.md</code> and <code>results.json</code>.</p></details>
<script>const data=DATA;const meta=META;function update(){const selected=data.filter(x=>x.seconds===Number(document.querySelector('#duration').value)&&x.task===document.querySelector('#task').value);const max=Math.max(...selected.map(x=>x.us));document.querySelector('#plot').innerHTML=selected.map(x=>`<div class="plotrow"><span>${x.runtime}</span><div><div class="bar" style="width:${100*x.us/max}%"></div></div><span>${x.us.toFixed(3)} µs</span></div>`).join('');document.querySelector('#rows').innerHTML=selected.map(x=>`<tr><td>${x.runtime}</td><td>${x.us.toFixed(3)}</td><td>${x.min.toFixed(3)}–${x.max.toFixed(3)}</td><td>${x.speedup.toFixed(3)}×</td><td>${x.latency_reduction.toFixed(2)}%</td></tr>`).join('')}document.querySelectorAll('select').forEach(x=>x.addEventListener('change',update));document.querySelector('#meta').textContent=JSON.stringify(meta,null,2);update();</script></body></html>'''.replace('DATA',json.dumps(rows)).replace('META',json.dumps(raw['metadata']))
page=Path(sys.argv[1]) if len(sys.argv)>1 else Path('/tmp/speechify-audio-benchmark.html')
page.write_text(content)
print(page)
