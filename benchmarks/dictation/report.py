"""Generate a timing report and self-contained visual from recorded evidence."""
import html
import json
import random
import statistics as st
from pathlib import Path

HERE=Path(__file__).resolve().parent
raw=json.loads((HERE/'results.json').read_text())
profile_path=HERE/'engine_profile.json'
profile=json.loads(profile_path.read_text()) if profile_path.exists() else None
resource_path=HERE/'resources.json'
resources=json.loads(resource_path.read_text()) if resource_path.exists() else None
names=list(raw['metadata']['fixture_seconds'])

def median_ci(python,rust):
    rng=random.Random(20261022)
    pairs=list(zip(python,rust))
    values=[]
    for _ in range(10000):
        chosen=rng.choices(pairs,k=len(pairs))
        values.append(100*(1-st.median(y for x,y in chosen)/st.median(x for x,y in chosen)))
    values.sort()
    return [values[249],values[9749]]

summary=[]
for name in names:
    entry={'fixture':name,'seconds':raw['metadata']['fixture_seconds'][name]}
    for mode in ('real','no_inference_control'):
        med={}
        for runtime in ('Python','Rust'):
            rows=[r for r in raw['measurements'] if r['fixture']==name and r['runtime']==runtime and r['mode']==mode]
            rows.sort(key=lambda r:r['round'])
            ready=[r['ready_ns']/1e6 for r in rows]
            residual=[r['ready_ns']/1e6-r['model_ms'] for r in rows]
            med[runtime]={'ready_ms':st.median(ready),'min_ms':min(ready),'max_ms':max(ready),'model_ms':st.median(r['model_ms'] for r in rows),'outside_model_ms':st.median(residual),'audio_ms':st.median(r['audio_ns']/1e6 for r in rows),'n':len(rows),'samples_ms':ready}
        med['raw_latency_reduction_pct']=100*(1-med['Rust']['ready_ms']/med['Python']['ready_ms'])
        med['speed_ratio']=med['Python']['ready_ms']/med['Rust']['ready_ms']
        med['latency_reduction_ci95']=median_ci(med['Python']['samples_ms'],med['Rust']['samples_ms'])
        entry[mode]=med
    py=entry['real']['Python']
    entry['optimistic_host_elimination_ms']=max(0,py['outside_model_ms'])
    entry['optimistic_host_elimination_pct']=100*max(0,py['outside_model_ms'])/(py['ready_ms']+20)
    if profile:
        records=[r for r in profile['measurements'] if r['fixture']==name]
        if records:
            for r in records:
                r['native_ort_s']=r['native_call_ns']/1e9
            entry['profile']={'native_ort_fraction':st.median(r['native_ort_s']/r['profile_total_s'] for r in records),'profile_ms':st.median(r['profile_total_s']*1000 for r in records),'native_ort_ms':st.median(r['native_ort_s']*1000 for r in records),'samples':len(records)}
            entry['profile']['optimistic_non_ort_fraction']=1-entry['profile']['native_ort_fraction']
    summary.append(entry)
(HERE/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
md='''# Would rewriting Speechify in Rust make dictation faster?

The measurements do not support a substantial dictation-speed gain from a Rust rewrite at the same model/backend settings. The Rust host control saves 0.252 ms on the 10.435-second sample, and the real-model comparison has no consistent improvement outside the observed variability. Roughly 98.7–98.8% of the profiled backend time is inside native ONNX API calls. I would keep Python for now and optimize measured bottlenecks before choosing a rewrite.

This experiment tests the warm dictation path with a real speech model and a Rust host prototype. It also profiles the speech backend, including its Python decoder. It does not implement or benchmark a complete replacement desktop application.

A language-wide yes/no answer cannot be guaranteed without a full port. The useful question is how much time in the current configuration is available for a Rust rewrite to save, and whether a Rust host actually saves it.

## Configuration

CPU: AMD Ryzen 5 3500U, no NVIDIA GPU. CPU-default Parakeet TDT 0.6B v3, int8, ONNX Runtime CPUExecutionProvider. English, no learned rules, empty glossary, optional LLM disabled. Beam size is 5 in the configuration but is a Whisper setting and is unused by this model. The model is already loaded and warmed before timing. Both hosts take turns using the **same live worker and the same pipe file descriptors**. They therefore share weights, precision, ONNX sessions, threading policy and warmed model state. ONNX uses its production library thread defaults; OMP/BLAS environment limits are 1, which does not set ONNX intra-op threads to 1.

The Rust executable assembles float32 audio from 30 ms chunks, detects silence, serializes a production-protocol request, writes the audio, reads and parses the response, and produces paste-ready text. The Python path runs production `np.concatenate`, `is_silent`, `EngineHost.transcribe`, `Learner.prompt`, and `Learner.apply`. Only one host reads/writes the shared worker at a time. The Rust process command round trip used by the test driver is outside Rust's timer. This favors an optimistic estimate of a Rust host's potential, not a complete desktop product comparison.

Each of three fixtures has two untimed warmup requests per runtime, then 12 randomized paired rounds. Timed requests start with pre-captured 30 ms audio chunks and finish with paste-ready text. Every result is checked for identical text across hosts and repeats. The 3-second fixture is a prefix of the bundled sample; the 30-second fixture cyclically repeats it. This is a small deterministic corpus, not a representative voice dataset.

## Real model latency

Times below are milliseconds. The native model still runs in the production Python worker for both hosts. Raw host-to-host differences include variation in that same worker's inference time and must not all be attributed to Rust.

| Audio | Host | Median text-ready | Min–max | Median worker/model | Median outside worker/model timer |
|---|---|---:|---:|---:|---:|
'''
for s in summary:
    for r in ('Python','Rust'):
        v=s['real'][r]
        md+=f"| {s['seconds']:.3f} s | {r} | {v['ready_ms']:.3f} | {v['min_ms']:.3f}–{v['max_ms']:.3f} | {v['model_ms']:.3f} | {v['outside_model_ms']:.3f} |\n"
md+='''
The worker/model timer covers `engine.transcribe` and its input view setup; it includes feature extraction, neural inference and Python decoding. It is rounded to whole milliseconds by production code. Each outside-model residual therefore has approximately ±0.5 ms quantization uncertainty in addition to timing noise. Residuals also include IPC, worker scheduling and response handling, not just Python interpreter work. Those costs cannot all be removed by changing languages.

## Raw comparison and variability

Positive latency reduction means Rust's observed median was lower. Negative means higher. The 95% interval resamples paired rounds 10,000 times with a fixed seed and recomputes the ratio of medians. It is a descriptive bootstrap on this run, not a guarantee across hardware or workloads. It does not isolate language effects from inference variation.

| Audio | Observed speed ratio | Observed latency reduction | Paired bootstrap 95% interval |
|---|---:|---:|---:|
'''
for s in summary:
    v=s['real']; low,high=v['latency_reduction_ci95']
    md+=f"| {s['seconds']:.3f} s | {v['speed_ratio']:.4f}× | {v['raw_latency_reduction_pct']:+.3f}% | {low:+.3f}% to {high:+.3f}% |\n"
md+='''
## How much could a host rewrite save?

The app deliberately waits 20 ms for clipboard ownership to settle before sending paste keys (`output.py`). That policy was preserved analytically by adding 20 ms to the measured text-ready path; no actual clipboard action was performed. OS clipboard/key-delivery time, microphone input latency, hotkey scheduling and UI queue delay are not measured. They are not assumed to be zero in the real application.

An optimistic host-only ceiling treats **all** measured time outside the worker/model timer as removable. Rust cannot actually eliminate all of it. With the model/backend and clipboard policy unchanged:

| Audio | Outside-model time available to remove | Optimistic reduction of measured path + 20 ms |
|---|---:|---:|
'''
for s in summary:
    md+=f"| {s['seconds']:.3f} s | {s['optimistic_host_elimination_ms']:.3f} ms | {s['optimistic_host_elimination_pct']:.4f}% |\n"
md+='''
This is a ceiling estimate for the measured warm path, not a proof about every configuration or omitted UI/OS stage. It uses per-request residuals and rounded worker timing. It should not be read as a guaranteed Rust improvement.

## No-inference control

Both hosts also send the identical audio payload through the production worker protocol to a no-inference backend returning the same fixed transcript. This isolates audio assembly, silence checking, protocol serialization/copying and IPC, using 30 paired rounds per fixture. These numbers are controls; they are not speech recognition results.

| Audio | Python median | Rust median | Python minus Rust |
|---|---:|---:|---:|
'''
for s in summary:
    p=s['no_inference_control']['Python']['ready_ms']; r=s['no_inference_control']['Rust']['ready_ms']
    md+=f"| {s['seconds']:.3f} s | {p:.3f} ms | {r:.3f} ms | {p-r:+.3f} ms |\n"
if profile:
    md+='''
## What if the Python speech backend were rewritten too?

A separate cProfile experiment measures the production `OnnxEngine.transcribe` implementation, including the Python ONNX-ASR decoder, on the same fixtures. Each has three profiled requests after initial zero-audio warmup. Profiling overhead and system load can affect these times, so they are attribution diagnostics rather than additional comparison samples.

“Native ONNX” uses a separate perf_counter_ns timer around the native session object’s `_sess.run` call. This avoids treating the Python ONNX wrapper’s self time as native, since this build does not expose the pybind call separately to cProfile. The time includes native inference, Python/native binding conversion, and any scheduling/waiting there; it is not a CPU-cycle measurement or a kernel-only measurement. All remaining time is conservatively counted as rewrite opportunity even though it includes NumPy native operations and other unavoidable work.

| Audio | Profile median | Native ONNX median | Native ONNX share | All remaining share |
|---|---:|---:|---:|---:|
'''
    for s in summary:
        if 'profile' in s:
            p=s['profile']; md+=f"| {s['seconds']:.3f} s | {p['profile_ms']:.3f} ms | {p['native_ort_ms']:.3f} ms | {p['native_ort_fraction']*100:.3f}% | {p['optimistic_non_ort_fraction']*100:.3f}% |\n"
    md+='''
A Rust wrapper around the same ONNX Runtime and model retains the native inference work. Removing every profiled non-ONNX cost is an intentionally unrealistic opportunity estimate under this observed call budget. Rust might also change binding/conversion costs inside the native API boundary; the profile does not isolate those from native compute, so this is not a strict mathematical upper bound on a complete native rewrite. A different decoder, model, quantization, threading policy or inference engine could change speed, but would be an algorithm/backend/configuration change and requires separate accuracy and latency validation.
'''
md+='''
## What remains unestablished

This is a real-model dictation-core comparison and backend profile, not a full Rust application port. It does not establish startup speed, idle/peak memory, recording/hotkey reliability, clipboard behavior, UI responsiveness, model-install speed, corrections/history performance, performance with large learned rule sets, other models, or GPU performance. The model's Python import/setup cost also remains in the Rust-host test because the worker is shared. A complete native worker could have different startup and memory costs.

The earlier PCM benchmark measured WAV conversion after the app's paste timer. Its 3× kernel result cannot be applied to release-to-paste latency. Model selection, compute configuration and the explicit clipboard delay are separate optimization targets; none was changed in this comparison.

## Reproduce

Dependencies follow the 14-day release cutoff. Rust's only direct dependency is pinned serde_json; the lock also pins transitive packages. Rust uses `cargo build --release --locked`. Python model packages were installed in an isolated environment, without changing the application's manifests. All measured work runs serially with no concurrent benchmark workers.

```bash
# Prepare Python 3.12 environments with NumPy and the app's CPU ONNX backend.
uv venv --python 3.12 /path/to/engine-env
uv pip install --python /path/to/engine-env/bin/python --exclude-newer '14 days' 'onnx-asr[hub]>=0.12' onnxruntime numpy
# Download the model once using OnnxEngine with the config above.
# Then run the comparison (Linux, working Rust linker required).
HF_HOME=/path/to/model-cache .venv/bin/python benchmarks/dictation/run.py --engine-python /path/to/engine-env/bin/python
# Profile only after the comparison completes.
PYTHONPATH=src HF_HOME=/path/to/model-cache OMP_NUM_THREADS=1 OPENBLAS_NUM_THREADS=1 MKL_NUM_THREADS=1 /path/to/engine-env/bin/python benchmarks/dictation/profile_engine.py
.venv/bin/python benchmarks/dictation/report.py
```

On this host, `cc` was absent; the exact GNU ld workaround is recorded as `rustflags` in results.json. Build scratch was moved off `/tmp` with TMPDIR. Model contents/revision and Python package versions are recorded in engine_profile.json. CPU affinity is inherited and recorded; frequency, temperature and unrelated system activity are not isolated; results are specific to this machine and this run.

Raw host timings, transcripts and source hashes: [results.json](results.json). Per-call native/Python profiling and model hashes: [engine_profile.json](engine_profile.json). Computed medians and intervals: [summary.json](summary.json). The application itself has not been modified.
'''
resource_summary={}
if resources and resources.get('disk'):
    for runtime in ('Python','Rust'):
        runs=[r for r in resources['host_runs'] if r['runtime']==runtime]
        resource_summary[runtime]={
            'idle_host_pss_mib':st.median(r['host_idle']['pss_bytes']/2**20 for r in runs),
            'idle_host_rss_mib':st.median(r['host_idle']['rss_bytes']/2**20 for r in runs),
            'idle_worker_pss_mib':st.median(r['worker_idle']['pss_bytes']/2**20 for r in runs),
            'peak_combined_pss_mib':st.median(max(x['combined_pss_bytes'] for x in r['samples'])/2**20 for r in runs),
            'peak_combined_rss_mib':st.median(max(x['combined_rss_bytes'] for x in r['samples'])/2**20 for r in runs),
            'host_startup_ms':st.median(r['host_startup_ns']/1e6 for r in runs),
        }
    md+='\n## RAM, startup and disk footprint\n\n'
    md+='Three fresh host processes per runtime use one shared, warmed worker. All three clip lengths were pre-warmed in that worker before measurement, before the host measurements. Pools may continue growing in the shared worker, so the combined-memory difference is an observation, not a pure language effect. Each host loads all three fixtures. Measurements sample Linux smaps_rollup every 100 ms during requests. RSS is resident mapped memory; PSS apportions shared pages across processes and is more useful for combined memory use. Private memory is also preserved in raw records. The coordinator is excluded. Peak PSS below is the median of the three per-run sampled combined peaks; short allocation spikes may be missed. Per-process kernel lifetime peak RSS is in resources.json. No UI, microphone, hotkeys, clipboard or recording database workload is active. These are dictation-host/model measurements, not the full desktop app RAM footprint.\n\n'
    md+='| Metric | Python host + worker | Rust prototype + same worker |\n|---|---:|---:|\n'
    for label,key,unit in [('Host idle PSS','idle_host_pss_mib','MiB'),('Host idle RSS','idle_host_rss_mib','MiB'),('Worker idle PSS','idle_worker_pss_mib','MiB'),('Sampled combined peak PSS','peak_combined_pss_mib','MiB'),('Sampled combined peak RSS','peak_combined_rss_mib','MiB'),('Host launch-to-ready','host_startup_ms','ms')]:
        md+=f"| {label} | {resource_summary['Python'][key]:.3f} {unit} | {resource_summary['Rust'][key]:.3f} {unit} |\n"
    md+='\nHost startup is process creation, imports and fixture setup with the model already running. It excludes the desktop UI, device setup, model loading, downloads and a full app startup. One separately measured worker startup (including its initial model warmup) took '+f"{resources['model_startup_ns']/1e9:.3f} seconds"+'. That is a single observation, not a Python/Rust startup comparison. The production Python dependencies were installed with the repository lock, with evdev omitted because this headless benchmark never uses Linux keyboard hooks.\n\n'
    md+='| Disk component | Logical file size | Allocated filesystem blocks |\n|---|---:|---:|\n'
    for key,record in resources['disk'].items():
        md+=f"| {key} | {record['logical_bytes']/2**20:.3f} MiB | {record['allocated_bytes']/2**20:.3f} MiB |\n"
    md+='\nEach disk row deduplicates hard links within that row and excludes symlink targets. Rows can overlap or share hard links; summing them can double-count storage. Logical size is not necessarily additional disk space used, especially with uv shared caches and filesystem reflinks. The shared Python distribution is counted separately because venv interpreter symlinks do not include it. App source includes the bundled sample; model cache includes actual weight blobs and metadata. Development/compiler caches, Go toolchain, benchmark build outputs and pip/uv caches are not production runtime requirements. The Rust build-output row is shown separately as development storage.\n\nThe Rust executable is a small host prototype, not a complete feature-equivalent app or standalone native ASR runtime. In this tested design it still needs the Python model worker, engine environment, interpreter and model files. Its executable size cannot be used as a full rewrite storage estimate. Raw memory samples and paths are in [resources.json](resources.json). Run `python3 benchmarks/dictation/resources.py --engine-python /path/to/engine-env/bin/python --app-python /path/to/app-env/bin/python --hf-home /path/to/model-cache` after building the Rust binary to reproduce this resource experiment. The app environment was prepared with `UV_PROJECT_ENVIRONMENT=/path/to/app-env uv sync --frozen --no-dev --no-install-package evdev --exclude-newer \'14 days\'`.\n'
    md+='\nHard-link-deduplicated totals across the measured stack components are below. These omit shared OS libraries, evdev, caches and development tools. Allocated blocks do not account for cross-inode reflink sharing.\n\n| Tested stack | Unique logical bytes | Allocated blocks |\n|---|---:|---:|\n'
    for label,record in resources['installed_stack_totals'].items():
        md+=f"| {label} | {record['logical_bytes']/2**20:.3f} MiB | {record['allocated_bytes']/2**20:.3f} MiB |\n"
    md+='\nNumPy files are hard-linked across these particular uv environments, so removing the Python app environment does not remove all its apparent bytes from the installed stack. This sharing is specific to this installation. With keep_all_audio enabled, saved PCM16 audio grows by 32,000 bytes/second: 1.92 MB/minute or 115.2 MB/hour (decimal), plus 44 bytes per WAV and SQLite/filesystem overhead. A language change does not reduce that recording format or the model weights.\n'
    (HERE/'resource_summary.json').write_text(json.dumps(resource_summary,indent=2)+'\n')
(HERE/'REPORT.md').write_text(md)
# Self-contained, fluid visual. The full evidence stays in the checked-out report.
data=[]
for s in summary:
    row={'seconds':s['seconds'],'python_ms':s['real']['Python']['ready_ms'],'rust_ms':s['real']['Rust']['ready_ms'],'python_model_ms':s['real']['Python']['model_ms'],'rust_model_ms':s['real']['Rust']['model_ms'],'ceiling_ms':s['optimistic_host_elimination_ms'],'ceiling_pct':s['optimistic_host_elimination_pct'],'ci':s['real']['latency_reduction_ci95'],'change_pct':s['real']['raw_latency_reduction_pct'],'control_saved_ms':s['no_inference_control']['Python']['ready_ms']-s['no_inference_control']['Rust']['ready_ms']}
    if 'profile' in s:row.update(native_pct=s['profile']['native_ort_fraction']*100,other_pct=s['profile']['optimistic_non_ort_fraction']*100)
    data.append(row)
page='''<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Speechify: is a Rust rewrite faster?</title><style>:root{font-family:system-ui,sans-serif;color-scheme:light dark}body{margin:0;line-height:1.55;background:transparent;color:var(--foreground,light-dark(#18232d,#e4edf5))}h2{font-size:24px;margin:0 0 8px}h3{font-size:17px;margin:24px 0 8px}p{margin:8px 0 16px}.muted{font-size:13px;color:var(--muted-foreground,light-dark(#536474,#afbdca))}.scope{padding:16px;border-radius:12px;background:var(--secondary,light-dark(#edf2f6,#1e2b35));margin:16px 0}table{border-collapse:collapse;width:100%;font-size:13px;font-variant-numeric:tabular-nums}th,td{padding:10px 7px;text-align:right;border-bottom:1px solid #8884;white-space:nowrap}th:first-child,td:first-child{text-align:left}.table{overflow:auto}th{font-weight:500}.stack{height:22px;display:flex;background:#e39d4144;border-radius:4px;overflow:hidden;margin:8px 0}.native{background:var(--chart-1,#3c83b2);height:22px}.other{background:var(--chart-2,#d39b36);height:22px}.plotrow{display:grid;grid-template-columns:95px 1fr 95px;gap:8px;align-items:center;font-size:13px}.plotrow span:last-child{text-align:right}details{margin:16px 0}summary{cursor:pointer}a{color:var(--primary,#4386b6)}@media(max-width:480px){h2{font-size:21px}.plotrow{grid-template-columns:70px 1fr 85px;font-size:12px}}</style></head><body><h2>Would a Rust rewrite make Speechify faster?</h2><p class="muted">Real Parakeet v3 int8 model · Ryzen 5 3500U CPU · same live worker · 12 paired rounds</p><div class="scope"><strong>Measured question:</strong> how much warm dictation latency can a language rewrite remove while preserving the speech model and backend? A Rust host prototype and a profile of the Python decoder test both parts. This is not a complete Rust desktop application.</div><h3>Actual audio-to-text path</h3><p class="muted">Median milliseconds, starting with captured audio chunks and ending with paste-ready text. No real clipboard action. The same worker serves both hosts; its timing variation is included.</p><div class="table"><table><thead><tr><th>Audio</th><th>Python</th><th>Rust</th><th>Python model</th><th>Rust model</th></tr></thead><tbody id="real"></tbody></table></div><p id="comparison" class="muted"></p><h3>Maximum opportunity outside inference</h3><p>Even eliminating all measured work outside the speech-worker timer would save only the amounts below. This is an optimistic estimate, not an achieved Rust speedup.</p><div class="table"><table><thead><tr><th>Audio</th><th>Available to remove</th><th>Path reduction*</th><th>Control: saved by Rust</th></tr></thead><tbody id="ceiling"></tbody></table></div><p class="muted">*Measured path plus the app's unchanged 20 ms clipboard settling delay. The delay is modeled, not measured as an actual paste. Control uses real IPC with inference replaced by a fixed transcript.</p><h3>Would rewriting the Python decoder help?</h3><div id="profile"></div><p class="muted">Blue: elapsed time inside native ONNX API calls, including binding/conversion costs. Gold: everything else, conservatively treated as rewrite opportunity. Three cProfile samples per clip; profiling is diagnostic and adds overhead. Rust using the same native engine retains that native work.</p><details><summary>Method and limits</summary><p>Identical 16 kHz mono float32 samples, Parakeet TDT 0.6B v3 int8, ONNX Runtime CPU provider, fresh learning profile, no LLM. Exact same loaded worker and pipe FDs; serialized access. Two warmups per runtime/fixture, then 12 randomized paired rounds; 30 rounds for the no-inference control. All transcripts must match.</p><p>The app's production model timer is rounded to milliseconds, so per-request outside-model time has ±0.5 ms quantization uncertainty. CPU frequency and unrelated activity are uncontrolled. Raw median differences also include inference variability. Microphone, hotkey, UI and OS clipboard delivery, startup, memory, learned-rule workloads, other models and GPU behavior remain unmeasured. A full product rewrite might change those costs; these tests do not promise a universal result.</p><p>Reproduction code and complete evidence are in <code>benchmarks/dictation/REPORT.md</code>, <code>results.json</code>, <code>engine_profile.json</code> and <code>summary.json</code>. The application is unchanged.</p></details><script>const d=DATA;document.querySelector('#real').innerHTML=d.map(x=>`<tr><td>${x.seconds.toFixed(3)} s</td><td>${x.python_ms.toFixed(3)}</td><td>${x.rust_ms.toFixed(3)}</td><td>${x.python_model_ms.toFixed(3)}</td><td>${x.rust_model_ms.toFixed(3)}</td></tr>`).join('');document.querySelector('#ceiling').innerHTML=d.map(x=>`<tr><td>${x.seconds.toFixed(3)} s</td><td>${x.ceiling_ms.toFixed(3)} ms</td><td>${x.ceiling_pct.toFixed(4)}%</td><td>${x.control_saved_ms.toFixed(3)} ms</td></tr>`).join('');document.querySelector('#comparison').textContent='Observed Rust latency changes (positive = lower): '+d.map(x=>`${x.seconds.toFixed(3)} s: ${x.change_pct.toFixed(2)}%, paired 95% interval ${x.ci[0].toFixed(2)}% to ${x.ci[1].toFixed(2)}%`).join('; ')+'. These include model-time variation and do not isolate a language effect.';document.querySelector('#profile').innerHTML=d.filter(x=>'native_pct'in x).map(x=>`<div class="plotrow"><span>${x.seconds.toFixed(3)} s</span><div class="stack"><div class="native" style="width:${x.native_pct}%"></div><div class="other" style="width:${x.other_pct}%"></div></div><span>${x.native_pct.toFixed(3)}% native</span></div>`).join('');</script></body></html>'''.replace('DATA',json.dumps(data))
if resource_summary:
    ramhtml='<h3>RAM and host startup</h3><p class="muted">Three fresh hosts per runtime; same warmed model worker. MiB = 1,048,576 bytes. PSS apportions shared pages. Sampled combined peaks, not a complete desktop app footprint.</p><div class="table"><table><thead><tr><th>Metric</th><th>Python</th><th>Rust host</th></tr></thead><tbody>'
    for label,key,unit in [('Host idle PSS','idle_host_pss_mib','MiB'),('Worker idle PSS','idle_worker_pss_mib','MiB'),('Combined peak PSS','peak_combined_pss_mib','MiB'),('Combined peak RSS','peak_combined_rss_mib','MiB'),('Host launch-to-ready','host_startup_ms','ms')]:
        ramhtml+=f"<tr><td>{label}</td><td>{resource_summary['Python'][key]:.3f} {unit}</td><td>{resource_summary['Rust'][key]:.3f} {unit}</td></tr>"
    ramhtml+=f'</tbody></table></div><p class="muted">Shared worker load + initial warmup took {resources["model_startup_ns"]/1e9:.3f} s in one observation; host startup excludes it. Worker pools can grow across runs, so combined RAM differences include that variation.</p><h3>Storage used by the tested components</h3><div class="table"><table><thead><tr><th>Component</th><th>Logical MiB</th></tr></thead><tbody>'
    for label,key in [('Model weights/cache','model_cache'),('Python engine dependencies','engine_python_env'),('Python app dependencies*','app_python_env'),('Shared Python interpreter/stdlib','shared_python_distribution'),('App source + bundled sample','app_source'),('Rust host executable†','rust_prototype_executable')]:
        record=resources['disk'][key]
        ramhtml+=f"<tr><td>{label}</td><td>{record['logical_bytes']/2**20:.3f}</td></tr>"
    for label,key in [('Measured Python stack, deduplicated','Python'),('Measured Rust-host stack, deduplicated','Rust_host_with_Python_worker')]:
        record=resources['installed_stack_totals'][key]
        ramhtml+=f"<tr><td>{label}</td><td>{record['logical_bytes']/2**20:.3f}</td></tr>"
    ramhtml+='</tbody></table></div><p class="muted">*Locked production dependencies, evdev omitted for this headless test. †Prototype only; it still needs the Python worker, interpreter, engine dependencies and weights. Totals deduplicate hard links across components; exclude OS libraries, caches and tools. Saved recordings grow by 1.92 MB/min or 115.2 MB/hour, plus database overhead. Complete app startup/UI memory and a full native rewrite remain unmeasured.</p>'
    page=page.replace('<details>',ramhtml+'<details>',1)
Path('/tmp/speechify-rust-rewrite.html').write_text(page)
print(json.dumps(data,indent=2))
