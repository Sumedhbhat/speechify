# Speechify

Speechify is an open-source voice-dictation app that runs entirely on your computer. Hold a global shortcut, speak, and cleaned-up text is typed into whichever app has focus. It is inspired by Wispr Flow, written from scratch, and tuned for Indian English by default.

## Status and known gaps

This is a first draft. It has passed its automated tests on Linux. A person has not yet run it on Windows or macOS, and its windows (the main window and the floating bar) have never been looked at on a real screen. Expect rough edges.

Known gaps:

- **Wayland.** The app cannot watch global shortcuts or type into other apps there. Click the bar, or bind `speechify --toggle` to a system shortcut. The text is put on the clipboard and shown on a copy card.
- **No field watching.** The app does not watch a text field after pasting. It learns from corrections when you edit a dictation in History, and from the nightly review.
- **Language model is separate.** The optional cleanup model needs Ollama (or another server) installed on its own. The app can ask Ollama to download a model, but it cannot install Ollama.
- **Command Mode needs the language model.** Without it, a command shows an error.
- **Clipboard restore covers text only.** If you had an image or files on the clipboard, they are not put back after a paste.
- **Mac Fn key.** It cannot be used as a shortcut. The default on every system is Right Shift.
- **Unsigned installers.** Windows will warn about an unknown publisher. The macOS app is signed with Speechify's own certificate, not an Apple Developer ID, so the first time you open it macOS asks you to allow it under System Settings › Privacy & Security › Open Anyway. Updates after that keep the Accessibility permission.
- **Not built yet:** retrying a dictation from History, dragging or docking the bar, a scratchpad shortcut, mouse-button shortcuts, IDE file tagging, and detecting playing media before muting. The mute option mutes all system output while you dictate.

## What it does

**Dictation**
- Push-to-talk on Right Shift, hands-free by double-tapping it (or with a shortcut), and a click on the bar.
- Dictations up to 20 minutes, with a warning at 19.
- Paste last, copy last, and cancel with Undo.
- Optional sounds, optional muting of system audio, pick your microphone.

**Cleanup**
- Rule-based formatter that always runs: fillers, self-corrections, spoken punctuation and line breaks, numbers, lists, spelling, tone.
- Optional local language model (Ollama or any OpenAI-compatible server) for natural rewriting. Its answer is checked, and the rule-based text is used if the answer looks wrong or late.
- Indian English by default: colour/organise spelling, lakh and crore grouping, the rupee sign. All of it is switchable.
- Tone per destination: casual in WhatsApp, formal in email, literal in terminals and code editors.
- Dictionary, snippets, and "press enter" to send.
- Command Mode: select text, speak an instruction, and the selection is replaced.

**Learning**
- Edits you make in History become dictionary entries and worked examples.
- A nightly teacher (a larger speech model and a larger language model) re-checks the day's recordings.
- Reviewed dictations can be exported as JSON Lines for fine-tuning elsewhere. Nothing is fine-tuned on your machine.

**Insights and history**
- Searchable history with playback, editing, flagging and deletion.
- Words, speed, streaks, an activity calendar, per-app and per-category breakdowns, latency, and accuracy against the teacher.
- Notes page, retention settings, and a Models page for downloads.

Details and examples are in [docs/features.md](docs/features.md).

## Using it

Default shortcuts. To change one, click Record next to it in Settings and press the new keys.

| Action | Windows and Linux | macOS |
| --- | --- | --- |
| Push to talk (hold) | Right Shift | Right Shift |
| Hands-free | Double-tap Right Shift | Double-tap Right Shift |
| Command Mode | Ctrl+Meta+Alt | Ctrl+Alt+Meta |
| Paste last dictation | Alt+Shift+Z | Ctrl+Meta+V |
| Copy last dictation | Alt+Shift+X | Ctrl+Meta+C |
| Cancel | Esc | Esc |

Meta is the Windows key on Windows, Super on Linux and Command on a Mac. A shortcut can name one side of a modifier (Right Shift, Left Ctrl, Right Alt, Left Meta); plain Shift, Ctrl, Alt or Meta means either side. Upgrading keeps the shortcuts you already have. Any shortcut can also be a double-tap: tap the keys twice quickly while recording it, and Settings shows it as "Right Shift twice" (stored as `Right Shift x2`). It goes off on the second press, which must start within half a second of letting go of the first.

- **Hold** the push-to-talk shortcut, speak, let go. A press shorter than half a second is a tap and records nothing.
- **Double-tap** push-to-talk, or press the hands-free shortcut, to keep recording without holding. Press any dictation shortcut again (or the stop button on the bar) to finish. A second press within half a second of that stop only completes a double-tap; it does not start a new recording.
- **Hold push-to-talk, then add the hands-free shortcut's extra key** (for example Ctrl+Meta, then Space, with Ctrl+Meta+Space as hands-free) to switch without letting go.
- **Shortcuts made only of modifiers** (Right Shift, Ctrl+Meta) open the microphone once held alone for 150 ms, or on a clean tap. The chime and the bar wait until half a second after the press. If another key joins before then, as in Shift+A or Ctrl+Meta+D, it was typing or another app's shortcut: the recording is dropped without a sound.
- **Esc** throws the recording away. The bar offers Undo, which transcribes it after all.
- **Command Mode:** select text, hold the command shortcut, say what to do ("make this shorter"), let go.
- `speechify --toggle` starts or stops dictation in the running app. `speechify --cancel` cancels.

### The bar

A small pill sits at the bottom centre of the screen. At rest it is a thin handle that reads "Click to dictate" only while the pointer is on it; clicks anywhere else, even right next to it, go to the app underneath (on Wayland, which does not tell apps where the pointer is, the whole bar area takes clicks). While you speak it shows a waveform, a cancel button and, in hands-free mode, a stop button. Command Mode adds a "Command" label. While working it shimmers, and on success it shows a check and the word count.

If no text field has focus, or the paste fails, the bar shows a **copy card** above the pill. The card holds the text with a Copy button, so nothing you said is lost. Short notices ("No speech heard", "Dictation cancelled") appear in the same place.

## First-run setup

1. Open the Models page and download a speech model. Whisper large-v3 turbo (about 0.9 GB) is the default and needs a graphics card to feel fast. Parakeet TDT 0.6B v3 is the recommended choice without one.
2. Optional: install [Ollama](https://ollama.com), then on the Models page pull the cleanup model (`qwen3:1.7b` by default). Without it, the rule-based formatter does the cleanup.
3. Optional: pull a larger model for the nightly teacher (`qwen3.5:9b-q4_K_M` by default) and download Whisper large-v3 for it.
4. On macOS, allow the microphone when asked. Accessibility access is needed to type into other apps. You allow it once: Speechify updates itself from Settings or the menu bar, and updates keep the permission.
5. Hold the shortcut and talk.

## How it works

```
 shortcut        microphone        speech model         cleanup            paste
 ─────────▶ record ─────▶ trim ─────▶ (on device) ─────▶ rules, or ─────▶ clipboard + Ctrl/Cmd+V
 (key state)   16 kHz    silence      Whisper/Parakeet    language model    then restore clipboard
                                                          + rules
                                                               │
                         SQLite + WAV on disk ◀────────────────┘
                                   │
                          nightly teacher ──▶ dictionary, examples, export
```

The engine (recognition, cleanup, storage, learning) lives in `core/` and the dictation state machine in `controller/`; neither touches a device. `os/` talks to the microphone, keyboard, clipboard and focused window. `src-tauri/` is the window, tray and thread wiring, and `ui/` is the interface. `perf/` is the performance gate, which CI runs only when the engine changes. See [docs/architecture.md](docs/architecture.md).

## Privacy

Everything stays on the machine unless you point Speechify at a server on another computer: one SQLite database plus WAV recordings in the app data folder. There is no telemetry and there are no accounts. The app only uses the network to download speech models from Hugging Face, to ask Ollama to pull a model, and to reach servers you set up yourself. Ollama on this machine keeps everything local. A remote speech server receives the recording of each dictation. A remote language-model server receives the words you dictated, the name and window title of the app you are typing into, and your dictionary terms and worked examples. In Settings you can keep history forever, for a day, or not at all, and choose whether recordings are kept.

## Building and testing

```
python3 scripts/fetch-native.py      # speech runtime, checksum-pinned
npm ci
npm run tauri -- dev
cargo test --workspace && npm test
npx moon run :lint :test :build --affected --downstream deep   # only what your uncommitted changes reach
```

See [docs/building.md](docs/building.md), [docs/testing.md](docs/testing.md) and [docs/releases.md](docs/releases.md).

## Documentation

- [docs/diagnostics.md](docs/diagnostics.md): recording crash logs, collection steps and initial macOS findings

- [docs/features.md](docs/features.md): what the formatter, dictionary, snippets, Command Mode, teacher and insights do, with examples
- [docs/architecture.md](docs/architecture.md): crates, threads, data model, how to extend
- [docs/building.md](docs/building.md): toolchain and build steps
- [docs/testing.md](docs/testing.md): tests and the coverage gate
- [docs/releases.md](docs/releases.md): nightly and stable releases, dependency policy

## Licence

MIT. See [LICENSE](LICENSE).
