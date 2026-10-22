# CPU Parakeet backend optimization

The application now uses a shared, bounded ONNX CPU thread pool for Parakeet where supported. The default is four threads, capped to process CPU affinity. This replaces separate automatic thread pools for the encoder, decoder and auxiliary ONNX sessions in the isolated worker. No model weights, precision, decoding settings or dependencies were changed. Accelerated providers and other speech models keep their existing session setup.

The strongest repeatable gain on this machine is CPU efficiency: transcription consumed about half as much CPU time and the worker had 5–6 threads instead of 65–66. Observed median latency also fell, but the alternating blocks vary substantially; a latency gain is not established across all conditions. Resident RAM did not improve materially.

## Verification

AMD Ryzen 5 3500U, 4 physical / 8 logical CPUs, Linux, Python 3.12.15, ONNX Runtime 1.30.0, ONNX-ASR 0.12.0, NumPy 2.5.3. CPUExecutionProvider, Parakeet TDT 0.6B v3 int8, English. Same pinned model revision `8f23f0c03c8761650bdb5b40aaf3e40d2c15f1ce` and bundled 16 kHz mono float32 speech recording throughout. The 3-second clip is a prefix; the 30-second clip repeats the sample. These are three fixtures from one recording, not a representative accuracy corpus.

Nine scheduling variants were screened on two fixtures with three requests per fixture. The winner was then tested against the original committed backend in six fresh processes, in B/A/A/B/B/A order. Each process warmed up with one second of zero audio and each fixture once, then ran three timed requests per fixture in shuffled order. That gives nine timed samples per fixture per variant, 54 verification requests total. Every transcript matched the baseline exactly, as did all 54 screening requests. A final smoke check passed through the production worker protocol and EngineHost.

Times below are model/backend latency, not full desktop release-to-paste latency. Process launch, imports before the constructor, file reads, initial model warmup and UI/clipboard work are outside the timed transcription interval. CPU time sums work by all process threads. It is not an energy measurement.

| Audio | Before ms | After ms | Speed ratio | Observed latency reduction | CPU-time reduction |
|---|---:|---:|---:|---:|---:|
| 3.000 s | 761.484 | 639.435 | 1.191× | 16.03% | 56.89% |
| 10.435 s | 2288.416 | 2232.458 | 1.025× | 2.45% | 49.75% |
| 30.000 s | 7081.278 | 6808.666 | 1.040× | 3.85% | 53.99% |

## Variability and limits

CPU clocks, thermals and other system activity were not controlled. A diagnostic reading during verification showed 95 °C and a current clock of about 1.87 GHz; that is one observation, not a thermal trace. Repeated continuous inference puts this laptop under a sustained load that differs from intermittent dictation. The preliminary sweep suggested a larger latency gain than the subsequent verification. The full comparison, including regressions in individual paired blocks, is retained below.

| Audio | Before block medians ms | After block medians ms | Three neighboring-pair latency reductions |
|---|---|---|---|
| 3.000 s | 603.678, 1067.620, 732.923 | 635.978, 647.974, 591.469 | -5.35%, +39.31%, +19.30% |
| 10.435 s | 1852.254, 3533.269, 2288.416 | 2263.757, 2232.458, 1832.814 | -22.22%, +36.82%, +19.91% |
| 30.000 s | 6038.846, 9556.175, 7081.278 | 6667.093, 6878.048, 6199.861 | -10.40%, +28.03%, +12.45% |

Each paired range includes a slowdown in one block. Do not treat the pooled median improvements as statistically reliable speedups, universal CPU gains, or GPU results. CPU-time reductions were much larger than the latency differences. Changing the thread limit is a deliberate runtime-configuration optimization; all recognition/model settings were held fixed.

## CPU and memory

Median across three fresh processes per variant, measured after warming all fixtures. PSS apportions shared pages; RSS counts all resident mappings, and VmHWM is the per-process kernel peak RSS. These figures include the model and measurement process, not the GUI, audio device, clipboard or complete desktop app. Small memory differences are not isolated from allocator/shared-page variation.

| Metric | Before | After |
|---|---:|---:|
| Model constructor | 3.377 s | 3.227 s |
| Warm model/process PSS | 1463.781 MiB | 1470.936 MiB |
| Process peak RSS | 1601.414 MiB | 1606.109 MiB |
| Process threads, three runs | [66, 65, 65] | [6, 6, 5] |

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
