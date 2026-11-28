# Working in this repository

Speechify is a [moon](https://moonrepo.dev) workspace over a Cargo workspace and one TypeScript project. Each top-level directory with a `moon.yml` is one moon project, listed in `.moon/workspace.yml`:

| Project | Directory | What it is | Gate |
| --- | --- | --- | --- |
| `core` | `core/` | The engine: recognition, cleanup prompt and vetting, formatting rules, storage, learning, default models (`core/defaults.json`) | `lint`, `test`, `coverage` (100% of lines) |
| `controller` | `controller/` | Shortcuts, the dictation session, the controller, bar click areas, insights | `lint`, `test`, `coverage` (100% of lines) |
| `os` | `os/` | Microphone, key state, clipboard, focused window | `lint`, `test` |
| `desktop` | `src-tauri/` | The Tauri app: window, tray, threads, commands | `lint`, `test`, `build` |
| `ui` | `ui/` | The interface (Vite, Vitest) | `lint`, `test`, `build`, `coverage` (100%) |
| `perf` | `perf/` | The performance gate: model speed and cleanup scenarios | `lint`, `test`, `test-report`, `release`, `measure` |
| `bench` | `bench/` | Offline evaluation on recorded clips | `lint`, `test` |

`perf` and `bench` depend on `core` alone. Only `core/`, `perf/`, the Rust toolchain, the native runtime pin, the performance report action and the CI setup can change what the performance gate measures, so CI runs it for nothing else. Keep it that way: model, prompt, formatting and pipeline code goes in `core`; anything about keys, sessions, windows or the interface goes in `controller`, `os`, `desktop` or `ui`. If `core` needs something from those, the design is wrong.

Project edges are `dependsOn` in each `moon.yml`. For the Rust crates they must match the path dependencies in `Cargo.toml`; `node tools/ci/check-graph.mjs` checks that, and CI fails if they drift.

Before you finish a change, see what it reaches and run that:

```
npx moon query affected --downstream deep            # uncommitted changes
npx moon query affected origin/main:HEAD --downstream deep  # the whole branch, as CI sees it
npx moon run :lint :test :build --affected --downstream deep
```

`measure` needs the default models and a language model server; leave it to CI unless you are changing `perf/`.

More: [docs/architecture.md](docs/architecture.md) (projects, edges, modules), [docs/testing.md](docs/testing.md) (tests, coverage, CI, the performance gate), [docs/building.md](docs/building.md).
