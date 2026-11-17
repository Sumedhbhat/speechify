# Diagnosing a recording quit

Speechify writes `diagnostics.log` in Tauri's app log directory. It starts before the keyboard and dictation workers. Logging failures go to stderr and do not prevent startup.

- macOS: `~/Library/Logs/org.sumedhbhat.speechify/`
- Windows: `%LOCALAPPDATA%\org.sumedhbhat.speechify\logs\`
- Linux: `$XDG_DATA_HOME/org.sumedhbhat.speechify/logs/`, or `~/.local/share/org.sumedhbhat.speechify/logs/`

The file rotates at 1 MiB, retaining one `diagnostics.previous.log`. Each line has Unix time in milliseconds, process ID, thread ID, stage and outcome. Each write is flushed to the OS before returning. This preserves completed lines through a process crash, but does not guarantee survival of power loss. Unix files are created with mode 0600.

Logs record shortcut binding events, availability of global key reading, stage boundaries and timing for capture, focus lookup, speech detection, transcription, rewriting, selection, copy, paste and Enter, plus bar state and explicit exit requests. An operation returning an error records `error`; the message is deliberately omitted because library errors can include server responses or user content. Rust panics record the source location, without the panic payload. Speechify never logs audio, transcripts, selected or clipboard text, window titles, raw key state, remote URLs or credentials through this logger. Native libraries and the default Rust panic hook may independently print more detail to stderr; inspect redirected stderr before sharing it.

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

This PR adds evidence collection first. It does not fix or claim to fix the quit. A follow-up fix should reproduce the actual Ctrl+Alt recording-to-paste path, identify the caller of the macOS input-source API, and include a real recording or screenshot of a successful dictation with the patched build and its logs.
