"""Write the measured CPU optimization report from the saved verification."""
import hashlib
import json
from pathlib import Path
import statistics as st

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[1]
raw=json.loads((HERE/'verification.json').read_text())
sweep=json.loads((HERE/'screen.json').read_text())
smoke=json.loads((HERE/'smoke.json').read_text())
names={'short_3s':3.0,'sample':10.435,'long_30s':30.0}
summary=[]
for name,seconds in names.items():
    row={'fixture':name,'seconds':seconds}
    for variant in ('before','after'):
        runs=[r for r in raw['runs'] if r['variant']==variant]
        samples=[m for r in runs for m in r['measurements'] if m['fixture']==name]
        row[variant]={'median_ms':st.median(m['ns']/1e6 for m in samples),'median_cpu_ms':st.median(m['cpu_ns']/1e6 for m in samples),'min_ms':min(m['ns']/1e6 for m in samples),'max_ms':max(m['ns']/1e6 for m in samples),'block_medians_ms':[st.median(m['ns']/1e6 for m in r['measurements'] if m['fixture']==name) for r in runs]}
    before,after=row['before'],row['after']
    row['latency_reduction_pct']=100*(1-after['median_ms']/before['median_ms'])
    row['cpu_reduction_pct']=100*(1-after['median_cpu_ms']/before['median_cpu_ms'])
    row['speed_ratio']=before['median_ms']/after['median_ms']
    # Pair neighboring opposite-variant processes; report the actual range of
    # effects rather than an over-precise CI based on just three block pairs.
    pair_changes=[]
    for i in range(0,len(raw['runs']),2):
        pair={r['variant']:st.median(m['ns']/1e6 for m in r['measurements'] if m['fixture']==name) for r in raw['runs'][i:i+2]}
        pair_changes.append(100*(1-pair['after']/pair['before']))
    row['paired_block_latency_reductions_pct']=pair_changes
    summary.append(row)
resource={}
for variant in ('before','after'):
    runs=[r for r in raw['runs'] if r['variant']==variant]
    resource[variant]={'load_s':st.median(r['load_ns']/1e9 for r in runs),'warm_pss_mib':st.median(r['warm_memory']['Pss']/2**20 for r in runs),'peak_rss_mib':st.median(r['after_memory']['VmHWM']/2**20 for r in runs),'threads':[r['warm_memory']['Threads'] for r in runs]}
result={'timings':summary,'resources':resource,'final_backend_sha256':smoke['backend_sha256']}
(HERE/'summary.json').write_text(json.dumps(result,indent=2)+'\n')
md='''# CPU Parakeet backend optimization

The application now uses a shared, bounded ONNX CPU thread pool for Parakeet where supported. The default is four threads, capped to process CPU affinity. This replaces separate automatic thread pools for the encoder, decoder and auxiliary ONNX sessions in the isolated worker. No model weights, precision, decoding settings or dependencies were changed. Accelerated providers and other speech models keep their existing session setup.

The strongest repeatable gain on this machine is CPU efficiency: transcription consumed about half as much CPU time and the worker had 5–6 threads instead of 65–66. Observed median latency also fell, but the alternating blocks vary substantially; a latency gain is not established across all conditions. Resident RAM did not improve materially.

## Verification

AMD Ryzen 5 3500U, 4 physical / 8 logical CPUs, Linux, Python 3.12.15, ONNX Runtime 1.30.0, ONNX-ASR 0.12.0, NumPy 2.5.3. CPUExecutionProvider, Parakeet TDT 0.6B v3 int8, English. Same pinned model revision `8f23f0c03c8761650bdb5b40aaf3e40d2c15f1ce` and bundled 16 kHz mono float32 speech recording throughout. The 3-second clip is a prefix; the 30-second clip repeats the sample. These are three fixtures from one recording, not a representative accuracy corpus.

Nine scheduling variants were screened on two fixtures with three requests per fixture. The winner was then tested against the original committed backend in six fresh processes, in B/A/A/B/B/A order. Each process warmed up with one second of zero audio and each fixture once, then ran three timed requests per fixture in shuffled order. That gives nine timed samples per fixture per variant, 54 verification requests total. Every transcript matched the baseline exactly, as did all 54 screening requests. A final smoke check passed through the production worker protocol and EngineHost.

Times below are model/backend latency, not full desktop release-to-paste latency. Process launch, imports before the constructor, file reads, initial model warmup and UI/clipboard work are outside the timed transcription interval. CPU time sums work by all process threads. It is not an energy measurement.

| Audio | Before ms | After ms | Speed ratio | Observed latency reduction | CPU-time reduction |
|---|---:|---:|---:|---:|---:|
'''
for r in summary:
    md+=f"| {r['seconds']:.3f} s | {r['before']['median_ms']:.3f} | {r['after']['median_ms']:.3f} | {r['speed_ratio']:.3f}× | {r['latency_reduction_pct']:.2f}% | {r['cpu_reduction_pct']:.2f}% |\n"
md+='''
## Variability and limits

CPU clocks, thermals and other system activity were not controlled. A diagnostic reading during verification showed 95 °C and a current clock of about 1.87 GHz; that is one observation, not a thermal trace. Repeated continuous inference puts this laptop under a sustained load that differs from intermittent dictation. The preliminary sweep suggested a larger latency gain than the subsequent verification. The full comparison, including regressions in individual paired blocks, is retained below.

| Audio | Before block medians ms | After block medians ms | Three neighboring-pair latency reductions |
|---|---|---|---|
'''
for r in summary:
    md+=f"| {r['seconds']:.3f} s | {', '.join(f'{x:.3f}' for x in r['before']['block_medians_ms'])} | {', '.join(f'{x:.3f}' for x in r['after']['block_medians_ms'])} | {', '.join(f'{x:+.2f}%' for x in r['paired_block_latency_reductions_pct'])} |\n"
md+='''
Each paired range includes a slowdown in one block. Do not treat the pooled median improvements as statistically reliable speedups, universal CPU gains, or GPU results. CPU-time reductions were much larger than the latency differences. Changing the thread limit is a deliberate runtime-configuration optimization; all recognition/model settings were held fixed.

## CPU and memory

Median across three fresh processes per variant, measured after warming all fixtures. PSS apportions shared pages; RSS counts all resident mappings, and VmHWM is the per-process kernel peak RSS. These figures include the model and measurement process, not the GUI, audio device, clipboard or complete desktop app. Small memory differences are not isolated from allocator/shared-page variation.

| Metric | Before | After |
|---|---:|---:|
'''
for label,key,unit in [('Model constructor','load_s','s'),('Warm model/process PSS','warm_pss_mib','MiB'),('Process peak RSS','peak_rss_mib','MiB')]:
    md+=f"| {label} | {resource['before'][key]:.3f} {unit} | {resource['after'][key]:.3f} {unit} |\n"
md+=f"| Process threads, three runs | {resource['before']['threads']} | {resource['after']['threads']} |\n"
md+='''
The model weights and installed dependencies are unchanged. The prior measured model cache remains about 639.5 MiB. The two changed Python source files add 2,174 bytes before compilation/compression; this is not a new package or model-size saving. There is no demonstrated storage or RAM reduction from this optimization. Recordings still use the same 16 kHz mono PCM16 format (1.92 MB per minute, plus WAV/SQLite/filesystem overhead).

## Configuration and compatibility

`onnx_threads = 4` is the new default in config.toml. Existing configs without the field also use 4. Positive values select a CPU thread budget, clamped to available CPU affinity where supported, otherwise logical CPU count. Set `onnx_threads = 0` to restore ONNX Runtime's automatic per-session threading. Other CPUs and usage patterns may benefit from a different budget.

Shared-pool setup only runs for CPU Parakeet models. CUDA and DirectML keep their prior options; GPU-load fallback uses the CPU policy. If the installed runtime lacks the public shared-pool API, or its global environment has already been initialized, bounded per-session pools are used instead. This compatibility path is unit-tested; its performance is not inferred from the shared-pool result. Windows/macOS/GPU hardware were not benchmarked here.

After verification, a cleanup stopped setting per-session thread counts that ORT ignores when a global pool is used. This removes startup warnings without changing the configured global pool. The source hash used in verification is recorded in verification.json; the final source hash and successful production worker/protocol check are in smoke.json.

## Checks

- 77 application tests passed; three Linux input-permission/device tests skipped on this host.
- Compilation, CLI version and app/history/model-picker imports passed.
- Unit tests cover affinity limits, unavailable CPU counts, zero/invalid settings, older runtimes, initialized global environments, unchanged CUDA/DirectML options, GPU-to-CPU fallback and config propagation to the worker.
- All 108 screening/verification transcripts matched their baseline; the final worker smoke transcript also matched.

## Reproduce

Use the isolated Python 3.12 environment with the recorded package versions and model cache from the earlier dictation benchmark. No additional dependencies are required. Run the sweep and verification serially; never run model measurements concurrently.

```bash
HF_HOME=/path/to/model-cache /path/to/engine-env/bin/python benchmarks/onnx_cpu/tune.py
HF_HOME=/path/to/model-cache /path/to/engine-env/bin/python benchmarks/onnx_cpu/verify.py
HF_HOME=/path/to/model-cache .venv/bin/python benchmarks/onnx_cpu/smoke.py /path/to/engine-env/bin/python
.venv/bin/python benchmarks/onnx_cpu/report.py
.venv/bin/python -m pytest -q
```

The scripts set the model worker offline and set OMP/OpenBLAS/MKL environment limits to one; ONNX uses its separate explicit thread-pool settings. `verify.py` loads the baseline backend from commit `498358d0a18c7e23e1c49826a9383ad7aa3d097f` and the optimized backend from the checkout, with model processes created and destroyed sequentially. Raw evidence: [screen.json](screen.json), [verification.json](verification.json), [smoke.json](smoke.json), [summary.json](summary.json). The baseline/model hashes and package versions are retained. The earlier language-comparison reports describe the pre-optimization code and have not been relabeled as results of this change.
'''
(HERE/'REPORT.md').write_text(md)
page='''<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Speechify CPU backend optimization</title><style>:root{color-scheme:light dark;font-family:system-ui,sans-serif}body{margin:0;background:transparent;color:var(--foreground,light-dark(#182532,#e5edf6));line-height:1.55}h2{font-size:24px;margin:0 0 8px}h3{font-size:17px;margin:24px 0 8px}p{margin:8px 0 16px}.muted{font-size:13px;color:var(--muted-foreground,light-dark(#546778,#adbdca))}.note{padding:16px;border-radius:12px;background:var(--secondary,light-dark(#edf3f7,#20303c));margin:16px 0}.table{overflow:auto}table{width:100%;border-collapse:collapse;font-size:13px;font-variant-numeric:tabular-nums}th,td{padding:10px 7px;border-bottom:1px solid #8884;text-align:right;white-space:nowrap}th:first-child,td:first-child{text-align:left}th{font-weight:500}.row{display:grid;grid-template-columns:90px 1fr 85px;align-items:center;gap:8px;font-size:13px;margin:10px 0}.row span:last-child{text-align:right}.bar{height:22px;border-radius:3px;background:var(--chart-1,#4286b5)}details{margin:16px 0}summary{cursor:pointer}code{font-size:12px}a{color:var(--primary,#4286b5)}@media(max-width:480px){h2{font-size:21px}.row{grid-template-columns:70px 1fr 78px;font-size:12px}}</style></head><body><h2>Speechify CPU backend optimization</h2><p class="muted">Parakeet v3 int8 · Ryzen 5 3500U · same weights and decoding · no Rust rewrite</p><div class="note"><strong>Implemented:</strong> share a bounded ONNX CPU thread pool across Parakeet sessions. Default budget: four threads. The strongest verified improvement is lower CPU consumption; a repeatable latency gain across conditions remains unestablished.</div><h3>Observed backend latency</h3><div class="table"><table><thead><tr><th>Audio</th><th>Before ms</th><th>After ms</th><th>Latency change</th></tr></thead><tbody id="latency"></tbody></table></div><p class="muted">Medians of nine requests per fixture/variant across three fresh processes each. All 54 transcripts match. These exclude GUI/clipboard latency. Model inference was run serially.</p><h3>CPU time saved per transcription</h3><div id="cpu"></div><p class="muted">CPU time sums all process threads. This measures computational resource use, not energy. Shared-pool process thread count fell from 65–66 to 5–6.</p><div class="note"><strong>Timing uncertainty:</strong> each fixture had a slowdown in one neighboring before/after block pair. Clocks and background activity were uncontrolled; one diagnostic CPU temperature reading was 95 °C. Treat latency percentages as observations on this run, not guaranteed speedups.</div><h3>RAM and storage</h3><div class="table"><table><thead><tr><th>Metric</th><th>Before</th><th>After</th></tr></thead><tbody id="ram"></tbody></table></div><p>Resident RAM did not improve materially. Model weights and dependencies are unchanged; the model cache remains about 639.5 MiB. Changed Python source adds about 2.1 KiB. Saved recordings still grow by about 1.92 MB/minute.</p><h3>Use and compatibility</h3><p>Restart the app to use the new backend. <code>onnx_threads = 4</code> is the default; <code>0</code> restores ONNX Runtime's automatic per-session pools. Try another count for other CPUs. Accelerated providers and other models retain their existing setup.</p><details><summary>Verification and limits</summary><p>Nine scheduling variants screened, then six fresh model processes in before/after/after/before/before/after order. Each warmed on zeros and all fixtures before timed requests. Fifty-four screening and fifty-four verification transcripts matched the baseline; the final production worker/protocol check also passed.</p><p>77 tests passed; three Linux input-device/permission checks skipped. Affinity limits, older runtimes, initialized global environments, configuration propagation and GPU fallback are covered by tests. Shared-pool support is optional, with bounded per-session pools as fallback. Other hardware, GPU speed, a broader accuracy corpus and complete desktop UI latency remain untested.</p><p>Full raw timings, per-block regressions, package/model/source hashes and reproduction commands are in <code>benchmarks/onnx_cpu/REPORT.md</code> and the adjacent JSON files. The earlier Rust comparison describes the original backend.</p></details><script>const d=DATA;const r=RESOURCE;document.querySelector('#latency').innerHTML=d.map(x=>`<tr><td>${x.seconds.toFixed(3)} s</td><td>${x.before.median_ms.toFixed(3)}</td><td>${x.after.median_ms.toFixed(3)}</td><td>${x.latency_reduction_pct.toFixed(2)}% lower</td></tr>`).join('');document.querySelector('#cpu').innerHTML=d.map(x=>`<div class="row"><span>${x.seconds.toFixed(3)} s</span><div><div class="bar" style="width:${x.cpu_reduction_pct}%"></div></div><span>${x.cpu_reduction_pct.toFixed(2)}% less</span></div>`).join('');document.querySelector('#ram').innerHTML=[['Warm process PSS','warm_pss_mib','MiB'],['Process peak RSS','peak_rss_mib','MiB'],['Model constructor','load_s','s']].map(([label,key,unit])=>`<tr><td>${label}</td><td>${r.before[key].toFixed(3)} ${unit}</td><td>${r.after[key].toFixed(3)} ${unit}</td></tr>`).join('');</script></body></html>'''.replace('DATA',json.dumps(summary)).replace('RESOURCE',json.dumps(resource))
Path('/tmp/speechify-cpu-optimization.html').write_text(page)
print(json.dumps(result,indent=2))
