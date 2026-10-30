# How Speechify is built

The Rust process owns dictation and keeps running when the settings window
closes. TypeScript renders settings and sends requests to Rust. The command
names and JSON payloads are the contract between them.

## Where things live

| Path | What it is |
| --- | --- |
| `src/` | The TypeScript settings and indicator windows. |
| `src-tauri/src/` | The Rust service: recording, inference, history, learning and platform adapters. |
| `src-tauri/resources/` | Defaults, the model catalog, and the optional Python worker, installer and trainer. |
| `src-tauri/tests/unit/` | Rust behavioral tests, included by the modules they exercise. |
| `tests/` | Frontend tests, the CLI and desktop process tests, and their fixtures. |
| `scripts/` | Build, packaging, fixture and coverage tools. |
| `.github/workflows/` | `ci.yml` runs the tests; `release.yml` builds installers and publishes releases. |

## Frontend

| File | Responsibility |
| --- | --- |
| `src/main.ts` | Find the app element and start the settings or indicator window. |
| `src/app.ts` | Own window state, navigation, event handlers and request ordering. |
| `src/api.ts` | Send a request through Tauri's `request` command. |
| `src/types.d.ts` | Describe the data the UI consumes; configuration keys and types come from `defaults.json`. |
| `src/pages/` | Render Dictation, History, Models, Learning and Settings from explicit inputs. |
| `src/shell.ts` | Render navigation and update live status without replacing an edited form. |
| `src/ui.ts` | Escape displayed data, render action buttons and show messages. |
| `src/style.css` | Theme tokens, controls, shell, page layouts and responsive rules. |

Page renderers return HTML. They do not fetch data or change application state.
Keep displayed service data escaped with `escapeHtml`; use `textContent` for
live text updates. `app.ts` owns the effects and the two ordering guards:

- `renderVersion` prevents an old page's response from replacing a newer page.
- `historyRequest` prevents an older search from replacing newer results.

Model filters and unsaved edits live for the lifetime of the window, inside
`startApplication`. A status refresh updates the status controls rather than
rebuilding a form. Navigate events from Rust respect unsaved edits.

## Rust

| Module | Responsibility |
| --- | --- |
| `main.rs` | Select desktop startup, `--version`, diagnostics, fixture inference or an internal worker. |
| `desktop/mod.rs` | Compose Tauri windows, tray, single-instance behavior and shutdown. |
| `desktop/commands.rs` | Validate IPC arguments and dispatch application operations. |
| `desktop/autostart.rs` | Manage Speechify's own startup-at-login entry. |
| `desktop/runtime/` | Wire the bounded queues and run recording, inference and keyboard loops. |
| `service/mod.rs` | Own shared state, settings updates, cancellation and shutdown. |
| `service/model.rs` | Select, verify, load and reuse the active worker. |
| `service/inference.rs` | Transcribe, apply learned corrections, optionally clean text and save results. |
| `service/diagnostics.rs` | Produce platform setup guidance. |
| `store/mod.rs` | Own the SQLite connection, latest in-memory audio and learning state. |
| `store/schema.rs` | Create and migrate the database schema. |
| `store/history.rs` | Record, search, correct, exclude and delete recordings. |
| `store/vocabulary.rs` | Apply learned rules and build vocabulary hints. |
| `store/labels.rs` | Select training labels and calculate bounded word edits. |
| `store/dataset.rs` | Export approved labels and import a consistent database/audio backup. |
| `learning/mod.rs` | Schedule work while idle and on external power. |
| `learning/power.rs` | Read native power status. |
| `learning/teacher.rs` | Select a permitted teacher and label an eligible recording. |
| `learning/training.rs` | Run training, check both adoption gates, retain versions and roll back. |
| `platform/mod.rs` | Enforce shared focus and held-key checks before pasting. |
| `platform/{linux,macos,windows}.rs` | Implement native key state, focus and clipboard operations. |

The remaining modules are focused adapters: `audio` captures and converts
samples; `keys` interprets key combinations; `evdev` reads granted Linux input
devices; `models` verifies/downloads model artifacts; `installer` supervises
optional Python setup; `worker` owns the child protocol; `native` wraps inference
FFI; `child_owner` owns process trees; and the clipboard/replacement modules
preserve native data and guard text replacement.

`ffi.rs` is generated from the pinned native header by
`scripts/generate-ffi.py`. Optional inference and training Python resources are
under `src-tauri/resources`; dependency locks and model checksums stay there too.

## A dictation's path

1. A held key or an in-app action sends a `CaptureCommand` containing the action,
   target and cancellation generation.
2. The recording loop owns the microphone and pre-roll. A submitted clip becomes
   a `Dictation` in the bounded queue, carrying the same target and generation.
3. The inference loop writes a temporary WAV and calls the service. The service
   runs the worker, applies corrections and saves a successful result.
4. The platform adapter pastes only while the original focus and key-state
   guards still hold. A clip without a safe target remains available for copying.

Cancellation advances the service generation. Work from an earlier generation
cannot paste a result. Each loop owns its local capture, key or queue state;
cross-thread state remains in `Service`.

## Making a change

Put rendering changes in the relevant page. Put native behavior in its platform
adapter. Put history and learning rules in their domain modules. Keep Tauri
setup concerned with wiring these pieces together.

For a new operation, add its dispatch case in `desktop/commands.rs`, call it
through `api.ts`, and exercise it through the rendered controls or service
boundary. Keep the transport payload stable during structural refactors.
The process-based tests address some scenarios by their Rust module names;
preserve those names when moving their files.

Install the developer tools in your Python environment, then run from the
repository root:

```sh
npm ci
python -m pip install -r requirements-dev.txt
npm run format
npm run check
```

`check` verifies formatting, the full frontend coverage thresholds, the frontend
build, coverage-gate behavior, platform adapter type checks, optional installer
behavior and both ordinary Rust suites. It prepares the pinned native runtime;
the desktop build prerequisites in [building.md](building.md) must already be available.

The native coverage matrix also runs disposable desktop, microphone and process
fixtures on Linux, macOS and Windows. Its combined coverage gate remains required.
See [testing.md](testing.md) for the full runner and fixture prerequisites.
