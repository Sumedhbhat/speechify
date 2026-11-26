# Diagnosing a recording quit

Speechify writes `diagnostics.log` in Tauri's app log directory. It starts before the keyboard and dictation workers. Logging failures go to stderr and do not prevent startup.

- macOS: `~/Library/Logs/org.sumedhbhat.speechify/`
- Windows: `%LOCALAPPDATA%\org.sumedhbhat.speechify\logs\`
- Linux: `$XDG_DATA_HOME/org.sumedhbhat.speechify/logs/`, or `~/.local/share/org.sumedhbhat.speechify/logs/`

The file rotates at 1 MiB, retaining one `diagnostics.previous.log`. Each line has Unix time in milliseconds, process ID, thread ID, stage and outcome. Each write is flushed to the OS before returning. This preserves completed lines through a process crash, but does not guarantee survival of power loss. Unix files are created with mode 0600.

Logs record shortcut binding events, initial and live changes in global-key access, stage boundaries and timing for capture, focus lookup, speech detection, transcription, rewriting, selection, copy, paste and Enter, plus bar state and explicit exit requests. An operation returning an error records `error`; the message is deliberately omitted because library errors can include server responses or user content. Rust panics record only the source filename and line, without the panic payload or absolute build path. Panic logging uses a nonblocking lock so it cannot deadlock behind a failed logger write. Speechify never logs audio, transcripts, selected or clipboard text, window titles, raw key state, remote URLs or credentials through this logger. Native libraries and the default Rust panic hook may independently print more detail to stderr; inspect redirected stderr before sharing it.

## Timing traces

Every dictation saved to History has a `trace` column in `speechify.db` (in the app data directory, `org.sumedhbhat.speechify`). It is a JSON array of spans. Each span has:

- `stage`: what ran (see below).
- `at_ms`: when it started, in milliseconds from the key release. Work done while you were still speaking is negative.
- `ms`: how long it took.
- `chunk`: its number among spans of the same stage, from 0. A piece's chunk is its order in the recording; a model span's chunk is its order among requests.
- `audio_ms`: the audio it covered. For pieces this is the speech in them; for speech detection and stream feeds it is the whole stretch.
- `words`: words recognised (pieces) or sent to the model (model requests).
- `model`: what the language model reported: `load_ms`, `prompt_ms`, `answer_ms`, `prompt_tokens`, `answer_tokens`. Missing fields were not reported.

Steps after the release, which run one after another and add up to the wait:

| Stage | What it covers |
| --- | --- |
| `stop` | Stopping the microphone, and reading the selection in Command Mode |
| `load` | Getting the models ready (normally instant; a model loaded late shows here) |
| `speech` | Finishing recognition: a piece still in progress, speech detection and the audio after the last pause |
| `settle` | Waiting for a block the language model was still cleaning up |
| `cleanup` | The last clean-up: rules, and the language model request if it was used |
| `paste` | Checking the focused app and pasting |

Work spans, which can run alongside the steps or before the release:

| Stage | What it covers |
| --- | --- |
| `piece` | Recognising one piece of the recording (or finishing a stream) |
| `scan` | Looking for a pause to end a piece at, while you speak (about every 400 ms once a piece could end) |
| `detect` | Finding the speech in a piece before it is recognised |
| `feed` | Giving a streaming model more audio |
| `model` | One language model request |

Dictations that never reach History keep their trace too, in the `attempts` table: `ending` is `silent` (no speech heard), `nothing` (speech that came to nothing, such as fillers), `failed` (with the error shown in `error`) or `cancelled`. Each row also has `created_at`, `offset_min` and `duration_ms` (the recording's length), and no text. A model request whose answer came to nothing is still in the trace. These rows follow the History retention setting: kept, deleted after a day, or never stored. Deleting all history deletes them as well.

To look at recent traces:

```sh
sqlite3 speechify.db "SELECT id, total_ms, trace FROM dictations ORDER BY id DESC LIMIT 5"
sqlite3 speechify.db "SELECT id, ending, error, trace FROM attempts ORDER BY id DESC LIMIT 5"
```

## Downloading a support file

Open Settings → Diagnostics → Download diagnostics. The app writes a uniquely named `speechify-diagnostics-*.json` file in Downloads and shows the saved path. Nothing is uploaded. If Downloads is unavailable or unwritable, the page shows an error and lets you retry.

The bundle includes version/platform information, both stage logs, and sanitized summaries of the three newest readable Speechify macOS crash reports. Crash summaries contain time, PID, app version, signal/exception, and up to 64 frames with symbol, library name and image offset. Raw reports, machine/user identifiers, filesystem paths, register values and application-specific messages are excluded. Each log read is capped at 1 MiB; a log is marked missing, unreadable or truncated when appropriate. The 64 newest matching crash files are considered; reports larger than 2 MiB, unreadable files and malformed reports are skipped while searching for up to three readable summaries. An empty crash list does not prove that no native crash occurred.

You can open the JSON yourself or share it with Claude or another assistant. It excludes your recordings, transcripts, selected/clipboard text, settings and credentials. Stage timestamps and PIDs let you correlate a failure with its native crash summary. Keep the raw report locally if deeper symbolication is needed.

## Permission changes after an update

On macOS the keyboard reader checks Accessibility access every second. Granting it reconnects the reader; revoking it cancels recording and resets the shortcut tracker. Reopen Settings or Home to refresh their explanatory text. The reader does not repeatedly open permission prompts.

An enabled System Settings entry can still refer to an older ad-hoc signing identity after an app update. In this session, restart alone continued to report no access; removing Speechify and adding the installed `/Applications/Speechify.app` again restored it. The runtime check reflects what macOS grants the current process, rather than assuming the visible checkbox is authoritative.

## Collecting evidence

1. Run the build containing these diagnostics and note the time.
2. Hold Ctrl+Alt, speak, then release. If the process quits, keep both log files before repeated launches rotate away the incident.
3. On macOS, find the matching `speechify-*.ips` under `~/Library/Logs/DiagnosticReports/`. Match its capture time and PID to the log. Native traps and aborts do not unwind Rust, so the panic hook cannot catch them.
4. Read the last `begin` without an `end` on the same thread. For example, `speech.transcribe ok` followed by `typing.paste begin` with no matching end narrows the quit to paste. Check the native crash stack to identify the failing API. A missing end alone can also mean a hang or external termination.
5. Share the relevant log interval and a reviewed crash stack. macOS reports include machine and filesystem details, so review them before posting publicly.

To follow the log on macOS:

```sh
tail -f "$HOME/Library/Logs/org.sumedhbhat.speechify/diagnostics.log"
```

## Initial investigation, 17 November 2026

Two existing reports, captured at 20:55:17 and 20:55:34 +0530, both show `EXC_BREAKPOINT` / `SIGTRAP` on a background thread with this stack:

```text
_dispatch_assert_queue_fail
 dispatch_assert_queue
 islGetInputSourceListWithAdditions
 isValidateInputSourceRef
 TSMGetInputSourceProperty
 [unsymbolicated Speechify frames]
 _pthread_start
```

The installed build identifies itself as `0.1.0-nightly.142`. The existing `~/Library/Logs/Speechify.log` contains native runtime output and a completed transcription, but no application stage logs. The source uses Enigo for paste/selection key injection on the dictation worker. Keyboard input-source access from that thread is a lead consistent with the native assertion, not a verified root cause. The stripped application frames have not been symbolicated, and this investigation has not reproduced the quit under a debugger.

## Follow-up investigation

Installing source commit `e16c00e` with a distinct diagnostic build version allowed real microphone capture, detection, transcription and paste. A physical Ctrl+Alt test also pasted successfully after Accessibility was re-authorized. The original SIGTRAP was not reproduced in this session. An isolated worker-thread Enigo layout lookup, both with and without AppKit initialization, also returned successfully. The historical native assertion therefore remains timing/state dependent; successful trials do not disprove it.

Enigo's `Key::Unicode` → macOS key-code conversion calls `TISGetInputSourceProperty`, matching the failing native API in the historical background-thread stack. Shortcut injection now runs on the macOS main queue, including Enigo creation and layout resolution. Clipboard waits and restoration remain on the dictation worker. An off-main guard prevents native keyboard access if the dispatch boundary is accidentally bypassed. This removes the observed unsafe call pattern without replacing layout-aware shortcuts with hard-coded US keys.

Cmd+Q produced a separate, reproducible SIGABRT in `ggml_abort` during native global teardown. Cocoa's predefined Quit item called `NSApplication terminate:` directly, bypassing the app's worker shutdown. The app menu now routes Cmd+Q through the existing tray Quit path, closing the recorder, freeing the models on their owner thread, and logging `worker shutdown_complete` before exiting. No screenshot of a successful transcription establishes that the historical SIGTRAP is impossible; use the bundle to report any recurrence.


The installed candidate passed a physical Ctrl+Alt dictation test, detected permission grants without restart, exported a real JSON bundle from Settings, and exited cleanly with Cmd+Q without a new crash report. See [candidate validation and screenshots](evidence/macos-validation.md) for the observed stage records and limits.

## Quits outside the menu, 18 November 2026

Routing Cmd+Q through the worker left every other way out unprotected. Dock › Quit, logging out, an installer's or script's quit Apple event, and the in-app update's restart still went from `NSApplication terminate:` (or `exit`) straight to native teardown, and macOS showed "Speechify quit unexpectedly". Running the binary with stderr captured and sending a quit Apple event shows the failing assertion:

```text
ggml/src/ggml-metal/ggml-metal-device.m:1040: GGML_ASSERT([rsets->data count] == 0) failed
  ggml_abort ← libggml-metal ← __cxa_finalize_ranges ← exit ← -[NSApplication terminate:] ← _handleAEQuit
```

ggml's Metal device is destroyed by a static destructor at exit and asserts that no GPU buffers are still registered. A loaded speech model's buffers still are, so the process aborts (exit status 134).

Every one of those paths emits `RunEvent::Exit` first (tao's `applicationWillTerminate`, `AppHandle::exit`, and `restart` from a background thread). The app now stops the worker there and waits up to 5 s until it has dropped the models, logging `app.release_models released` (or `timed out` if a native call is still busy). On the installed release build, a quit Apple event left one new crash report before the change and none in six runs after it; Cmd+Q and an update restart from 0.1.0-nightly.1001 to .1002 also left none.
