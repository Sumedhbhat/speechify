# Would rewriting Speechify in Rust make dictation faster?

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
| 3.000 s | Python | 818.043 | 719.400–1573.239 | 817.500 | 0.567 |
| 3.000 s | Rust | 799.683 | 719.804–1217.783 | 799.500 | 0.517 |
| 10.435 s | Python | 2435.969 | 2154.553–3896.481 | 2434.000 | 2.255 |
| 10.435 s | Rust | 2358.236 | 2167.052–3508.765 | 2357.000 | 1.662 |
| 30.000 s | Python | 8045.241 | 6921.577–9730.750 | 8039.500 | 5.903 |
| 30.000 s | Rust | 7611.198 | 7212.687–10820.035 | 7607.000 | 4.546 |

The worker/model timer covers `engine.transcribe` and its input view setup; it includes feature extraction, neural inference and Python decoding. It is rounded to whole milliseconds by production code. Each outside-model residual therefore has approximately ±0.5 ms quantization uncertainty in addition to timing noise. Residuals also include IPC, worker scheduling and response handling, not just Python interpreter work. Those costs cannot all be removed by changing languages.

## Raw comparison and variability

Positive latency reduction means Rust's observed median was lower. Negative means higher. The 95% interval resamples paired rounds 10,000 times with a fixed seed and recomputes the ratio of medians. It is a descriptive bootstrap on this run, not a guarantee across hardware or workloads. It does not isolate language effects from inference variation.

| Audio | Observed speed ratio | Observed latency reduction | Paired bootstrap 95% interval |
|---|---:|---:|---:|
| 3.000 s | 1.0230× | +2.244% | -2.327% to +9.282% |
| 10.435 s | 1.0330× | +3.191% | -5.210% to +8.225% |
| 30.000 s | 1.0570× | +5.395% | -2.275% to +15.456% |

## How much could a host rewrite save?

The app deliberately waits 20 ms for clipboard ownership to settle before sending paste keys (`output.py`). That policy was preserved analytically by adding 20 ms to the measured text-ready path; no actual clipboard action was performed. OS clipboard/key-delivery time, microphone input latency, hotkey scheduling and UI queue delay are not measured. They are not assumed to be zero in the real application.

An optimistic host-only ceiling treats **all** measured time outside the worker/model timer as removable. Rust cannot actually eliminate all of it. With the model/backend and clipboard policy unchanged:

| Audio | Outside-model time available to remove | Optimistic reduction of measured path + 20 ms |
|---|---:|---:|
| 3.000 s | 0.567 ms | 0.0677% |
| 10.435 s | 2.255 ms | 0.0918% |
| 30.000 s | 5.903 ms | 0.0732% |

This is a ceiling estimate for the measured warm path, not a proof about every configuration or omitted UI/OS stage. It uses per-request residuals and rounded worker timing. It should not be read as a guaranteed Rust improvement.

## No-inference control

Both hosts also send the identical audio payload through the production worker protocol to a no-inference backend returning the same fixed transcript. This isolates audio assembly, silence checking, protocol serialization/copying and IPC, using 30 paired rounds per fixture. These numbers are controls; they are not speech recognition results.

| Audio | Python median | Rust median | Python minus Rust |
|---|---:|---:|---:|
| 3.000 s | 1.022 ms | 0.820 ms | +0.201 ms |
| 10.435 s | 3.278 ms | 3.025 ms | +0.252 ms |
| 30.000 s | 10.010 ms | 8.962 ms | +1.048 ms |

## What if the Python speech backend were rewritten too?

A separate cProfile experiment measures the production `OnnxEngine.transcribe` implementation, including the Python ONNX-ASR decoder, on the same fixtures. Each has three profiled requests after initial zero-audio warmup. Profiling overhead and system load can affect these times, so they are attribution diagnostics rather than additional comparison samples.

“Native ONNX” uses a separate perf_counter_ns timer around the native session object’s `_sess.run` call. This avoids treating the Python ONNX wrapper’s self time as native, since this build does not expose the pybind call separately to cProfile. The time includes native inference, Python/native binding conversion, and any scheduling/waiting there; it is not a CPU-cycle measurement or a kernel-only measurement. All remaining time is conservatively counted as rewrite opportunity even though it includes NumPy native operations and other unavoidable work.

| Audio | Profile median | Native ONNX median | Native ONNX share | All remaining share |
|---|---:|---:|---:|---:|
| 3.000 s | 668.302 ms | 660.558 ms | 98.841% | 1.159% |
| 10.435 s | 2193.695 ms | 2164.404 ms | 98.737% | 1.263% |
| 30.000 s | 6932.076 ms | 6844.554 ms | 98.807% | 1.193% |

A Rust wrapper around the same ONNX Runtime and model retains the native inference work. Removing every profiled non-ONNX cost is an intentionally unrealistic opportunity estimate under this observed call budget. Rust might also change binding/conversion costs inside the native API boundary; the profile does not isolate those from native compute, so this is not a strict mathematical upper bound on a complete native rewrite. A different decoder, model, quantization, threading policy or inference engine could change speed, but would be an algorithm/backend/configuration change and requires separate accuracy and latency validation.

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

## RAM, startup and disk footprint

Three fresh host processes per runtime use one shared, warmed worker. All three clip lengths were pre-warmed in that worker before measurement, before the host measurements. Pools may continue growing in the shared worker, so the combined-memory difference is an observation, not a pure language effect. Each host loads all three fixtures. Measurements sample Linux smaps_rollup every 100 ms during requests. RSS is resident mapped memory; PSS apportions shared pages across processes and is more useful for combined memory use. Private memory is also preserved in raw records. The coordinator is excluded. Peak PSS below is the median of the three per-run sampled combined peaks; short allocation spikes may be missed. Per-process kernel lifetime peak RSS is in resources.json. No UI, microphone, hotkeys, clipboard or recording database workload is active. These are dictation-host/model measurements, not the full desktop app RAM footprint.

| Metric | Python host + worker | Rust prototype + same worker |
|---|---:|---:|
| Host idle PSS | 24.648 MiB | 4.374 MiB |
| Host idle RSS | 34.781 MiB | 6.227 MiB |
| Worker idle PSS | 1543.684 MiB | 1548.149 MiB |
| Sampled combined peak PSS | 1574.773 MiB | 1556.523 MiB |
| Sampled combined peak RSS | 1595.637 MiB | 1562.938 MiB |
| Host launch-to-ready | 135.496 ms | 10.543 ms |

Host startup is process creation, imports and fixture setup with the model already running. It excludes the desktop UI, device setup, model loading, downloads and a full app startup. One separately measured worker startup (including its initial model warmup) took 3.755 seconds. That is a single observation, not a Python/Rust startup comparison. The production Python dependencies were installed with the repository lock, with evdev omitted because this headless benchmark never uses Linux keyboard hooks.

| Disk component | Logical file size | Allocated filesystem blocks |
|---|---:|---:|
| app_source | 0.549 MiB | 0.629 MiB |
| app_python_env | 57.859 MiB | 60.750 MiB |
| engine_python_env | 149.244 MiB | 154.449 MiB |
| shared_python_distribution | 99.783 MiB | 108.977 MiB |
| model_cache | 639.515 MiB | 639.551 MiB |
| rust_prototype_executable | 0.590 MiB | 0.594 MiB |
| benchmark_rust_build_outputs | 19.922 MiB | 20.176 MiB |

Each disk row deduplicates hard links within that row and excludes symlink targets. Rows can overlap or share hard links; summing them can double-count storage. Logical size is not necessarily additional disk space used, especially with uv shared caches and filesystem reflinks. The shared Python distribution is counted separately because venv interpreter symlinks do not include it. App source includes the bundled sample; model cache includes actual weight blobs and metadata. Development/compiler caches, Go toolchain, benchmark build outputs and pip/uv caches are not production runtime requirements. The Rust build-output row is shown separately as development storage.

The Rust executable is a small host prototype, not a complete feature-equivalent app or standalone native ASR runtime. In this tested design it still needs the Python model worker, engine environment, interpreter and model files. Its executable size cannot be used as a full rewrite storage estimate. Raw memory samples and paths are in [resources.json](resources.json). Run `python3 benchmarks/dictation/resources.py --engine-python /path/to/engine-env/bin/python --app-python /path/to/app-env/bin/python --hf-home /path/to/model-cache` after building the Rust binary to reproduce this resource experiment. The app environment was prepared with `UV_PROJECT_ENVIRONMENT=/path/to/app-env uv sync --frozen --no-dev --no-install-package evdev --exclude-newer '14 days'`.

Hard-link-deduplicated totals across the measured stack components are below. These omit shared OS libraries, evdev, caches and development tools. Allocated blocks do not account for cross-inode reflink sharing.

| Tested stack | Unique logical bytes | Allocated blocks |
|---|---:|---:|
| Python | 893.278 MiB | 908.457 MiB |
| Rust_host_with_Python_worker | 889.681 MiB | 904.199 MiB |

NumPy files are hard-linked across these particular uv environments, so removing the Python app environment does not remove all its apparent bytes from the installed stack. This sharing is specific to this installation. With keep_all_audio enabled, saved PCM16 audio grows by 32,000 bytes/second: 1.92 MB/minute or 115.2 MB/hour (decimal), plus 44 bytes per WAV and SQLite/filesystem overhead. A language change does not reduce that recording format or the model weights.
