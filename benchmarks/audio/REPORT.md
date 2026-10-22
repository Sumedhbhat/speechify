# Speechify audio preprocessing benchmark

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

## 1-second clip · silence

| Implementation | Median µs/op | Min–max µs/op | Speed ratio | Latency reduction | Throughput increase |
|---|---:|---:|---:|---:|---:|
| Python/NumPy | 14.427 | 13.788–15.408 | 1.000× | +0.00% | +0.00% |
| Rust | 15.408 | 13.971–15.768 | 0.936× | -6.80% | -6.36% |
| Go | 15.802 | 14.105–16.017 | 0.913× | -9.53% | -8.70% |
| TypeScript/Node | 23.917 | 22.750–26.168 | 0.603× | -65.78% | -39.68% |
| TypeScript/Bun | 14.849 | 13.985–15.619 | 0.972× | -2.92% | -2.84% |

## 1-second clip · pcm16

| Implementation | Median µs/op | Min–max µs/op | Speed ratio | Latency reduction | Throughput increase |
|---|---:|---:|---:|---:|---:|
| Python/NumPy | 13.730 | 13.238–17.003 | 1.000× | +0.00% | +0.00% |
| Rust | 26.977 | 24.306–31.971 | 0.509× | -96.48% | -49.10% |
| Go | 33.567 | 32.043–35.693 | 0.409× | -144.47% | -59.10% |
| TypeScript/Node | 51.956 | 49.773–65.723 | 0.264× | -278.40% | -73.57% |
| TypeScript/Bun | 53.133 | 51.033–71.128 | 0.258× | -286.98% | -74.16% |

## 10-second clip · silence

| Implementation | Median µs/op | Min–max µs/op | Speed ratio | Latency reduction | Throughput increase |
|---|---:|---:|---:|---:|---:|
| Python/NumPy | 63.323 | 58.017–97.495 | 1.000× | +0.00% | +0.00% |
| Rust | 142.132 | 139.617–151.615 | 0.446× | -124.46% | -55.45% |
| Go | 142.284 | 141.121–160.388 | 0.445× | -124.70% | -55.50% |
| TypeScript/Node | 232.072 | 221.367–443.364 | 0.273× | -266.49% | -72.71% |
| TypeScript/Bun | 147.899 | 146.934–153.751 | 0.428× | -133.56% | -57.19% |

## 10-second clip · pcm16

| Implementation | Median µs/op | Min–max µs/op | Speed ratio | Latency reduction | Throughput increase |
|---|---:|---:|---:|---:|---:|
| Python/NumPy | 427.693 | 409.021–569.824 | 1.000× | +0.00% | +0.00% |
| Rust | 261.224 | 244.523–313.148 | 1.637× | +38.92% | +63.73% |
| Go | 325.427 | 309.797–347.193 | 1.314× | +23.91% | +31.43% |
| TypeScript/Node | 522.093 | 493.241–579.594 | 0.819× | -22.07% | -18.08% |
| TypeScript/Bun | 506.732 | 483.205–517.493 | 0.844× | -18.48% | -15.60% |

## 60-second clip · silence

| Implementation | Median µs/op | Min–max µs/op | Speed ratio | Latency reduction | Throughput increase |
|---|---:|---:|---:|---:|---:|
| Python/NumPy | 1198.562 | 1173.659–1288.823 | 1.000× | +0.00% | +0.00% |
| Rust | 852.960 | 844.913–882.345 | 1.405× | +28.83% | +40.52% |
| Go | 878.981 | 863.630–915.474 | 1.364× | +26.66% | +36.36% |
| TypeScript/Node | 1231.199 | 1176.892–1284.756 | 0.973× | -2.72% | -2.65% |
| TypeScript/Bun | 938.359 | 895.132–974.098 | 1.277× | +21.71% | +27.73% |

## 60-second clip · pcm16

| Implementation | Median µs/op | Min–max µs/op | Speed ratio | Latency reduction | Throughput increase |
|---|---:|---:|---:|---:|---:|
| Python/NumPy | 4842.448 | 4530.862–5191.749 | 1.000× | +0.00% | +0.00% |
| Rust | 1587.494 | 1522.328–1855.970 | 3.050× | +67.22% | +205.04% |
| Go | 1954.585 | 1881.507–2069.893 | 2.477× | +59.64% | +147.75% |
| TypeScript/Node | 3235.492 | 2879.920–3458.995 | 1.497× | +33.18% | +49.67% |
| TypeScript/Bun | 3087.247 | 2915.304–4016.371 | 1.569× | +36.25% | +56.85% |

## What this establishes

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
{
  "platform": "Linux-7.0.0-38-generic-x86_64-with-glibc2.43",
  "cpu": "AMD Ryzen 5 3500U with Radeon Vega Mobile Gfx",
  "logical_cpus": 8,
  "affinity": [
    0,
    1,
    2,
    3,
    4,
    5,
    6,
    7
  ],
  "python": "3.12.15 (main, Oct 14 2026, 01:05:27) [Clang 23.1.3 ]",
  "numpy": "2.5.3",
  "rust": "rustc 1.99.0 (b940084d7 2026-09-28)",
  "go": "go version go1.26.0 linux/amd64",
  "node": "v24.21.0",
  "bun": "1.4.3",
  "commit": "498358d0a18c7e23e1c49826a9383ad7aa3d097f",
  "fixture_sha256": "a7a1b5c9815084cc71f52260186f8144610e73b61a94482ee995535cd0e7bdb3",
  "rounds": 9,
  "warmup_iterations_per_process": 30,
  "sample_rate": 16000,
  "channels": 1,
  "dtype": "little-endian float32",
  "threads": {
    "OMP_NUM_THREADS": "1",
    "OPENBLAS_NUM_THREADS": "1",
    "MKL_NUM_THREADS": "1",
    "GOMAXPROCS": "1"
  },
  "rust_flags": "--edition=2021 -O -C target-cpu=x86-64 -L /tmp/speechify-benchmark-tools -C linker=/usr/bin/ld -C linker-flavor=ld -C link-arg=/usr/lib/x86_64-linux-gnu/Scrt1.o -C link-arg=/usr/lib/x86_64-linux-gnu/crti.o -C link-arg=/usr/lib/x86_64-linux-gnu/crtn.o -C link-arg=--dynamic-linker=/lib64/ld-linux-x86-64.so.2",
  "go_flags": "build (default optimizations)",
  "seed": 20261022,
  "report_generated_at_utc": "2026-10-22T15:21:36.807758+00:00",
  "source_sha256_at_report_generation": {
    "rust.rs": "9cc412b42c0b61239be0498c3c83ba29cc278f9ae8d56d6fb0ed8d04ddd16f16",
    "run.py": "25d4a400ad73a1e9abb61701861b303bd78ad893683097a08afc336c5372e891",
    "python.py": "e1f32cf9d2a697aac41bbf3892a739251c176131287d15058e674ebd90b731ba",
    "go.go": "7cfde4220063707727df880fa452ab2c3fbdb69639a0a47a125a6331d17a96ed",
    "typescript.ts": "141db0dc2018e77df679047334d572a637a40dc273b4c8297d9d489f862355ff"
  },
  "source_note": "Rust and Go sources were formatted after measurement; formatting does not change behavior. Source hashes captured at report generation."
}
```
