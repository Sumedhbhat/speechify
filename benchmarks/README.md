# Speechify performance benchmarks

Run model benchmarks one at a time. Concurrent inference changes the CPU,
memory and scheduling conditions being measured.

- [Audio kernels](audio/REPORT.md): the production NumPy audio operations and
  equivalent Rust, Go and TypeScript implementations, with Node.js and Bun
  measured separately. These are component timings, not application timings.
- [Dictation and resources](dictation/REPORT.md): a Python/Rust host comparison
  using the same loaded speech worker, plus backend profiling, RAM and disk
  measurements. The Rust executable is an experimental host, not an application
  rewrite.
- [CPU Parakeet tuning](onnx_cpu/REPORT.md): the scheduling sweep and alternating
  before/after verification behind the shared ONNX thread-pool change. This
  includes unchanged transcripts, CPU time, memory and slower individual blocks.

Each report describes its environment, timing boundaries, limitations and
reproduction commands. The JSON files retain the raw measurements and hashes.
They are marked as generated data so code review can focus on the scripts and
application changes.

Recorded results belong to the source versions identified in each experiment.
Rerunning a script against the current checkout can produce different results;
the earlier audio and Rust comparisons have not been relabeled as measurements
of the optimized backend. The CPU tuning scripts explicitly load the original
backend from its recorded Git commit for their baseline.

Python model environments and downloaded weights are external prerequisites.
The reports identify their versions and model revision. Rust, Go, Node.js and
Bun are needed only by the experiments that compare those runtimes. Benchmark
build products are not part of the installed Speechify application.
