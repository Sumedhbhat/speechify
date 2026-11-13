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
- **Mac Fn key.** It cannot be used as a shortcut. The Mac default is Ctrl+Alt.
- **Unsigned installers.** Windows will warn about an unknown publisher. The macOS app is only ad-hoc signed.
- **Not built yet:** retrying a dictation from History, dragging or docking the bar, a scratchpad shortcut, mouse-button shortcuts, IDE file tagging, and detecting playing media before muting. The mute option mutes all system output while you dictate.

## What it does

**Dictation**
- Push-to-talk, hands-free (double-tap or shortcut plus Space), and a click on the bar.
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

Default shortcuts (change them in Settings):

| Action | Windows and Linux | macOS |
| --- | --- | --- |
| Push to talk (hold) | Ctrl+Meta | Ctrl+Alt |
| Hands-free | Ctrl+Meta+Space | Ctrl+Alt+Space |
| Command Mode | Ctrl+Meta+Alt | Ctrl+Alt+Meta |
| Paste last dictation | Alt+Shift+Z | Ctrl+Meta+V |
| Copy last dictation | Alt+Shift+X | Ctrl+Meta+C |
| Cancel | Esc | Esc |

Meta is the Windows key on Windows, Super on Linux and Command on a Mac.

- **Hold** the push-to-talk shortcut, speak, let go. A press shorter than half a second is a tap and records nothing.
- **Double-tap** push-to-talk, or press the hands-free shortcut, to keep recording without holding. Press any dictation shortcut again (or the stop button on the bar) to finish.
- **Hold push-to-talk, then add Space** to switch to hands-free without letting go.
- **Esc** throws the recording away. The bar offers Undo, which transcribes it after all.
- **Command Mode:** select text, hold the command shortcut, say what to do ("make this shorter"), let go.
- `speechify --toggle` starts or stops dictation in the running app. `speechify --cancel` cancels.

### The bar

A small pill sits at the bottom centre of the screen. At rest it reads "Click to dictate". While you speak it shows a waveform, a cancel button and, in hands-free mode, a stop button. Command Mode adds a "Command" label. While working it shimmers, and on success it shows a check and the word count.

If no text field has focus, or the paste fails, the bar shows a **copy card** above the pill. The card holds the text with a Copy button, so nothing you said is lost. Short notices ("No speech heard", "Dictation cancelled") appear in the same place.

## First-run setup

1. Open the Models page and download a speech model. Whisper large-v3 turbo (about 0.9 GB) is the default and needs a graphics card to feel fast. Parakeet TDT 0.6B v3 is the recommended choice without one.
2. Optional: install [Ollama](https://ollama.com), then on the Models page pull the cleanup model (`gemma3:4b` by default). Without it, the rule-based formatter does the cleanup.
3. Optional: pull a larger model for the nightly teacher (`qwen3.5:9b-q4_K_M` by default) and download Whisper large-v3 for it.
4. On macOS, allow the microphone when asked. Accessibility access is needed to type into other apps.
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

The logic lives in `core/`, which never touches a device. `os/` talks to the microphone, keyboard, clipboard and focused window. `src-tauri/` is the window, tray and thread wiring. See [docs/architecture.md](docs/architecture.md).

## Privacy

Everything stays on the machine: one SQLite database plus WAV recordings in the app data folder. There is no telemetry and there are no accounts. The app only uses the network to download speech models from Hugging Face, to ask Ollama to pull a model, and to reach servers you set up yourself (a language-model server or an optional remote speech server). In Settings you can keep history forever, for a day, or not at all, and choose whether recordings are kept.

## Building and testing

```
python3 scripts/fetch-native.py      # speech runtime, checksum-pinned
npm ci
npm run tauri -- dev
cargo test --workspace && npm test
```

See [docs/building.md](docs/building.md), [docs/testing.md](docs/testing.md) and [docs/releases.md](docs/releases.md).

## Documentation

- [docs/features.md](docs/features.md): what the formatter, dictionary, snippets, Command Mode, teacher and insights do, with examples
- [docs/architecture.md](docs/architecture.md): crates, threads, data model, how to extend
- [docs/building.md](docs/building.md): toolchain and build steps
- [docs/testing.md](docs/testing.md): tests and the coverage gate
- [docs/releases.md](docs/releases.md): nightly and stable releases, dependency policy

## Licence

MIT. See [LICENSE](LICENSE).
