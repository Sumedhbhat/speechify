# Architecture

Speechify is a Cargo workspace with three crates and a TypeScript interface.

```
speechify/
  core/       speechify-core   all decisions; no devices, no windows
  os/         speechify-os     microphone, key state, clipboard, focused window
  src-tauri/  speechify        window, tray and threads; the binary
  src/        TypeScript UI (main window and the floating bar), built with Vite
  scripts/    fetch-native.py, coverage.sh, Linux bootstrap helpers
```

## Why the logic lives in `core`

Dictation touches a microphone, global key state, the clipboard and other people's windows. None of that can run in a headless test. So `core` never touches any of it. It asks for what it needs through two traits in `core/src/app.rs`, and the tests supply fakes.

- **`Platform`** is the operating system: start and stop capture, report the focused app and whether it takes text, read the selection, paste, copy, press Enter, play a chime, mute audio, show the bar, save and load WAV files, and tell the time.
- **`Engines`** is the loaded models: the everyday recogniser and rewriter, the teacher's larger pair, and a `warm` hook.

`Controller<P: Platform, E: Engines>` owns the state machine and drives the pipeline. Its tests run whole dictations ("hold, speak, paste") with a fake `Platform` and fake engines, so the behaviour of the app is tested without a screen. The clock is also passed in (`Clock { now, offset_min }`), so nothing in `core` reads the time itself.

This is why the coverage gate can demand every line of `core`. The other two crates are kept thin on purpose.

`os` has no dependency on `core`, so its Windows and macOS code can be compile-checked from any machine. `src-tauri` joins them: `desktop.rs` implements `Platform` with the `os` adapters, and `models.rs` implements `Engines` with the real runtime and Ollama client.

## Modules in `core`

| Module | Role |
| --- | --- |
| `app` | `Controller`, `Platform`, `Engines`, the `Bar` states |
| `session` | The state machine: hold, tap, double-tap lock, cancel, limits |
| `hotkey` | Parses shortcuts, tracks chords over raw key state |
| `pipeline` | One dictation from audio to text; `compose` turns words into typed text; `Draft` cleans up long dictations in blocks while they are spoken |
| `live` | Recognises a recording piece by piece, at pauses, while it is still being made |
| `format/*` | The deterministic formatter, one file per pass |
| `llm` | Prompts, the Ollama/OpenAI-compatible client, vetting of answers |
| `asr` | `Recognizer` trait, remote server client, prompt, phantom-phrase guard |
| `native` | On-device recognition through the transcribe.cpp library, loaded at run time |
| `catalog` | Pinned model list, download with resume and checksum |
| `store` | SQLite: settings, history, dictionary, snippets, notes, examples |
| `insights` | Statistics from history |
| `learn`, `diff` | Word alignment, finding corrections worth keeping, word error rate |
| `teacher` | The nightly review, learning from edits, training export |
| `context` | Sorting apps into categories, paste keys, clipboard restore delay |
| `settings` | Every setting and its default |
| `audio`, `http` | Sample conversion, trimming, WAV, chimes; a small blocking HTTP client |

## Threads

Three threads matter, all started in `src-tauri/src/worker.rs`.

1. **UI thread.** Tauri's event loop and webviews. Commands from the UI (`src-tauri/src/commands.rs`, mirrored one-to-one by `src/api.ts`) run here or on Tauri's async pool, and use the UI's own database connection (`Shared::db`).
2. **Keyboard thread.** Polls the key state every 12 ms. When it changes, the `Tracker` turns it into `Down`, `Up` and `Released` edges and sends them to the worker. Polling state instead of listening for events means a missed key-up cannot leave dictation stuck on. It does not read keys at all on Wayland.
3. **Worker thread.** Owns the microphone, the models and the `Controller`, with its own database connection. It waits on a channel for input events, settings reloads and "run the teacher". With nothing to do it sends a `Tick` (every 100 ms while recording or processing) to drive tap timeouts and the 20-minute limit, and once a minute checks whether the nightly review is due.

Model downloads and Ollama pulls run on their own short-lived threads and report progress as events. The mute call and the clipboard restore also run off-thread.

`Shared.wanted` is set whenever an input event arrives. The teacher polls it between dictations, so a review yields as soon as you want to dictate.

The app starts a second instance only to talk to the first one: `speechify --toggle` sends the bar-click event and `--cancel` sends cancel (`tauri-plugin-single-instance`). Launch at login passes `--background`, which keeps the main window hidden and leaves the app in the tray.

## The dictation, key-down to paste

1. The keyboard thread sees the push-to-talk chord held. `Tracker` emits `Down(PushToTalk)`. Extra keys are accepted only when they extend the shortcut into another one (Ctrl+Meta into Ctrl+Meta+Space). Ctrl+Meta+Left is somebody else's shortcut.
2. The worker feeds the event to `Session::handle`, which returns effects: `Start` and `Mode`.
3. `Controller::start` asks the `Platform` for the focused window (`Target`: app, process, title) before anything else can take focus, opens the microphone, optionally mutes output, plays the start chime and calls `Engines::warm`, which nudges Ollama so its model is loaded by the time you stop.
4. The bar shows "listening". Capture sends a loudness value about 20 times a second for the waveform. On every `Tick` while recording, `Controller::listen` asks for the audio recorded since the last piece (`Platform::recorded`) and hands it to `live::Live`. When the voice detector finds a pause of 400 ms or more after at least 3 s of new audio, everything up to the middle of the pause is recognised there and then (see "Recognising while recording" below).
5. Key-up of the chord after at least 500 ms gives `Stop`. Under 500 ms it is a tap: the session waits 500 ms for a second press, which locks hands-free, and discards the recording otherwise.
6. `Controller::process` shows "processing". For Command Mode it first reads the selection by copying it.
7. `pipeline::run_live` recognises what is left since the last pause (for a short dictation, all of it): it trims silence, builds the glossary from the dictionary, and calls the recogniser. Pieces that come out empty or as phantom phrases are dropped. It sorts the target into a `Category`.
8. `pipeline::compose` makes the text (see below). Command Mode instead sends the selection and the instruction to the rewriter.
9. `Controller::deliver` checks `Platform::focus`. If the focus is clearly not a text field, the bar shows a copy card. Otherwise it calls `Platform::paste` with the paste keys for the category and OS. A failed paste also shows the card.
10. `paste` puts the text on the clipboard (excluded from clipboard history where the OS supports it), waits for modifier keys to be released, sends the paste keys, and restores your previous text clipboard after a delay if nothing else was copied in the meantime.
11. If "press enter" was said, Enter is sent.
12. `Controller::remember` saves the recording as a WAV and the dictation row, then prunes by the retention setting. The bar shows a check and the word count.

`compose` does this:

```
raw words
  ├─ cleanup None  ──▶ dictionary + snippets only
  └─ otherwise
       ├─ format::format(raw)                 the rule-based result, always computed
       └─ if the model is on, reachable, and the dictation has >= min_words:
            cleanup_prompt(raw, brief) ─▶ model ─▶ vet_cleanup
              ok         ─▶ format::finish(answer)      numbers, spelling, dictionary, tidy, style
              off the rails or failed ─▶ keep the rule-based result, record why
```

The model gets the raw words, not the rule-based text. The brief carries the style, category, locale, the focused app and title, the vocabulary and up to three worked examples.

## Data model

One SQLite file, `speechify.db`, in the app data folder. The schema is the `SCHEMA` constant in `core/src/store.rs`, created with `IF NOT EXISTS`. Settings use partial JSON with defaults for every field, so older files keep loading.

| Table | Holds |
| --- | --- |
| `kv` | Settings (JSON), category rules, `teacher_last_run`, `teacher_last_report` |
| `dictations` | One row per dictation: time and UTC offset; app, process, title, category; Command Mode flag; `duration_ms`, `speech_ms`; `raw` and `text`; `words`; `rewritten`; speech and rewrite model names; `asr_ms`, `rewrite_ms`, `total_ms`; `outcome` (pasted, copied, cancelled, failed); `fixes`; the audio file name; `edited`; the teacher's `teacher_raw`, `teacher_text`, `teacher_wer`, `reviewed_at`; `flagged` |
| `dictionary` | `heard`, `term`, `source` (manual, edit, teacher), `hits`; unique on the pair. An empty `heard` is a plain word |
| `forgotten` | Corrections the user deleted, so they are not suggested again |
| `snippets` | `trigger` (unique), `expansion`, `hits` |
| `notes` | Title, body, update time |
| `suggestions` | Corrections seen but not yet seen `learn_after` times, with a count and a status |
| `examples` | Worked examples: category, `raw`, `gold`. The newest 200 are kept |

Next to the database, in the same folder: `audio/` (16 kHz mono WAV files), `models/` (GGUF files), and `training.jsonl` once you export. The folder is the one Tauri picks for the identifier `org.sumedhbhat.speechify`: `%APPDATA%\org.sumedhbhat.speechify` on Windows, `~/Library/Application Support/org.sumedhbhat.speechify` on macOS, `~/.local/share/org.sumedhbhat.speechify` on Linux.

Times are stored as Unix seconds plus the local offset at the time, so day boundaries in Insights and the nightly schedule follow the user's own clock.

## Recognising while recording

Waiting for the whole recording before recognising any of it made the wait after a long dictation grow with its length: on the development laptop's CPU, about 18 s after a minute of speech with Parakeet. `live::Live` recognises the recording in pieces while it is still being made, so when the user stops only the words since the last pause are left. The wait is then about the same however long the dictation was (about 1.3 s after a minute on the same CPU).

- A piece ends only in a pause the voice detector found: at the middle of a silence of at least 400 ms, once the piece is at least 3 s long, or, after 20 s without such a pause, at the longest silence there is. Never inside a word. These values were chosen on the Svarah recordings in `bench/` (`speechify-bench latency … pieces`): longer pauses and pieces gave no better accuracy and left more to do at the end.
- Each piece is normalised, trimmed and checked for phantom phrases on its own. Whisper is given the end of the text so far as its prompt, so it carries on in the same style.
- A recogniser without a voice detector (a remote server) gets the whole recording at the end, as before; `Live` stops asking for audio once it knows.
- Recognising in pieces also fixes long recordings: Parakeet given a minute or more at once loses words (half of one 67 s recording in the evaluation).
- The worker thread never recognises a piece itself. It finds the cut, then hands the piece to a `job::Job` on a thread of its own, holding another handle on the same model (`asr::Shared`, which lets one thread at a time use the model). Later ticks collect the words. Key events therefore never wait for a piece. A cancel drops the job, which sets its stop flag; transcribe.cpp checks it between decoding steps (it cannot stop in the middle of the encoder), and the answer is thrown away. A key-up stops the microphone straight away and then waits only for the piece already under way, since its words are needed, before recognising the rest.

When the language model is on, `pipeline::Draft` does the same for cleanup: once at least 40 words ending in a full sentence have been recognised, they are cleaned up as a block while the user goes on speaking. The model sees the end of the text cleaned up so far, so it can carry on a numbered list. Blocks are joined (list items on their own lines) and the destination's style is applied once to the whole, so a style that drops the closing full stop does not drop one in the middle. Command Mode is never cleaned up early: the instruction only means something whole.

The model is asked on a thread of its own too. A cancel forgets the block. A key-up waits for its answer, which it would need anyway. When the worker stops, `Models` waits (up to `RELEASE_WAIT`, and the exit path a little longer) for a piece still under way to release the model, so that the model is freed before the process exits (Metal checks at exit that nothing is left on the GPU).

### Streaming models

Some models are trained to recognise speech as it arrives, rather than a finished recording: Parakeet Unified is the one in the catalog (`streaming: true`). They are a separate setup. On the Models page they have their own section; choosing one sets `speech.streaming` and `speech.streaming_model`, and choosing an ordinary model turns streaming off again. `Speech::active_model` says which model dictation uses, and only that one is loaded. Streaming models are never offered as the teacher.

`Live` notices a streaming recogniser (`Recognizer::streams`) on its first tick and then does not look for pauses at all. Every 480 ms of new audio goes to the model through transcribe.cpp's stream API (`transcribe_stream_begin`, `_feed`, `_finalize`), on a job thread like a piece. The model reports the text it has settled on, which later audio cannot change. Each settled sentence, once the next one has begun, becomes a piece, so `Draft` cleans up blocks exactly as before. On key-up only the audio since the last feed is left: it is fed, the stream is finalised (which flushes the model's one-second lookahead), and the final text is used.

- The finalised text is the model's last word. If it no longer begins with the sentences handed out while recording, the transcript is marked `revised` and the whole dictation is cleaned up again, as after a "scratch that".
- If feeding fails, or ending the stream does, the whole recording is recognised again at the end with the same model (streaming models also work on a finished recording).
- A model more than 4 s behind the speaker (`BEHIND_MS`) cannot keep up, as on a laptop CPU. It is given up the same way, so the wait after key-up is one run over the recording rather than however long the model needs to catch up.
- A cancel drops the feed in flight, and the abort callback stops it at the next decoding step. A stream left unfinished is reset before the model is used again.
- The voice detector still runs on each fed chunk, only to count the speech (words per minute, the phantom check).

Parakeet Unified re-encodes 5.6 s of left context plus 1.04 s of lookahead for every 1.04 s of new audio, so it does about seven times the work of recognising the recording once. The stream API lets the caller choose the left context, but this model file offers only 5.6 s, so it cannot be lowered. A graphics card does this faster than real time; a laptop CPU does not, which is why the model is marked `wants_gpu`.

Measured on the development laptop's CPU (Ryzen 5 3500U) with the same Svarah recordings as the pieces work, word error rate pooled over each set:

| | ~60 s (6 recordings) | ~20 s (8 recordings) |
| --- | --- | --- |
| Parakeet v3, whole recording | 23.2% | 14.9% |
| Parakeet v3, pieces | 15.7% | 19.0% |
| Parakeet Unified, whole recording | 13.0% | 15.4% |
| Parakeet Unified, streaming | **10.9%** | **9.9%** |

Streaming took 2.6 to 3.5 s of CPU per second of audio, so on this CPU it cannot keep up in real time. Fed as fast as it could go, finishing the stream after the last word took a median of 3 s on the CPU, mostly the final 7.7 s window. GPU timings are not measured yet.

## Speech recognition

`core/src/native.rs` loads the transcribe.cpp runtime with `libloading`, from the directory `src-tauri/resources/native` (bundled into the installer), or `SPEECHIFY_NATIVE_DIR` when set. It declares the C structs the runtime uses, with spare space at the end of each, and checks the ABI at load time. That way one binary uses the CPU, Vulkan or Metal backend depending on the machine, and the app needs no C++ toolchain. `scripts/fetch-native.py` downloads the pinned wheel (version 0.4.1) for the platform and verifies its SHA-256 before unpacking.

The device setting is "auto", "cpu", "vulkan", "metal" or "cuda". With "auto" the runtime picks its preferred device. If that turns out to be a virtual-machine Metal device ("paravirtual"), which produces garbage, the model is reloaded on the CPU. The runtime registers its backends once per process, so switching from CPU to a graphics device needs a restart of the app; the error says so.

`Recognizer` is the trait the pipeline sees. `native::Engine` and `asr::Remote` (an OpenAI-compatible `/audio/transcriptions` server) implement it.

## The teacher loop

`Controller::review_if_due` is called by the worker. Every minute it checks `teacher::due`: the teacher is enabled, the local hour is at or past the set hour, and it has not run today. It also requires the app to be idle and retention not to be "never". If nothing is waiting for review, it records the run and returns without loading anything.

Otherwise `Engines::teacher` loads the larger speech model on the same device as live dictation and, if the server has it, the larger language model. `teacher::review` then goes through up to `batch` pending dictations (those with audio, not yet reviewed, pasted or copied):

```
for each dictation, oldest first
  stop if the user wants to dictate
  load WAV ─▶ teacher transcribes ─▶ WER against the everyday transcript
           ─▶ compose(teacher words, rewrite always on) = the ideal text
           ─▶ save_review
  corrections(everyday raw, teacher raw) ─▶ suggest(); at learn_after, accept into the dictionary
  gold = user's edit, else ideal; if it differs from what was typed ─▶ add_example
```

When the run ends, the teacher's models are released. `learn_from_edit` is the same machinery, triggered by editing a dictation in History, and it accepts a correction at once because a hand edit needs no second opinion.

## The interface

`src/` is plain TypeScript with no framework: `app.ts` is the shell with the sidebar and the page host, `pages/*.ts` are the nine pages (Home with History, Insights, Dictionary, Snippets, Style, Notes, Models, Learning, Settings), and `bar-view.ts` draws the floating bar. `api.ts` is the contract with the Rust side: one function per Tauri command and one listener per event. Everything else receives `api` through `deps.ts`, so the tests give it fakes (`tests/fakes.ts`).

The bar is its own window (`bar.html`), transparent, always on top, not focusable. Its size follows its state so it never covers more of the screen than it needs: 220 by 48 at rest, taller for notices and the copy card. It is placed at the bottom centre of the monitor's work area.

## Adding a speech model

1. Find the GGUF file in the handy-computer Hugging Face collection (`<id>-gguf`, file `<id>-Q8_0.gguf`) and pick a commit hash. Never point at `main`.
2. Add a `Pin` to `PINS` in `core/src/catalog.rs` with the id, display name, revision, SHA-256, exact byte size, `wants_gpu`, and a one-line note.
3. Run `cargo test -p speechify-core catalog`. `catalog_is_consistent` checks the checksum length, that the URL is pinned, the size, and that the default model ids in `Settings` exist.
4. The Models page lists whatever `catalog::models()` returns, so no UI change is needed. If the new model needs a newer runtime, update the version and wheel hashes in `scripts/fetch-native.py` and the struct mirrors in `native.rs`.

## Adding a formatter rule

1. Pick the pass in `core/src/format/` that owns the behaviour, or add a file and declare it in `format/mod.rs`. Order matters: the passes run in the order written in `format()`.
2. A pass takes text and `&mut Edits`, and returns text. Increment the matching counter in `Edits` for each change, and add a field there if it is a new kind of change. `Edits::total` is the "fixes made" figure.
3. Write the tests first, in the same file, as input and output pairs. Include cases the rule must leave alone. The corrections and list passes show how: a table of ordinary sentences that must come out unchanged.
4. If the language model could do the same job, add a sentence to `cleanup_prompt` in `llm.rs`. The rule still matters, because it is the fallback.
5. If the rule must hold even after the model has written the text (spelling, grouping, tone), put it in `format::finish`, not only in `format::format`.
6. Run `cargo test -p speechify-core` and `bash scripts/coverage.sh`. Every line must be covered.
7. Add an example to [features.md](features.md), copied from the test.
