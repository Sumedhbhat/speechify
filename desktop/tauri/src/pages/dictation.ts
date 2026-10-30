import { button, escapeHtml } from '../ui';
import type { Snapshot } from '../types';

export function renderDictation(snapshot: Snapshot, manualRecording: boolean): string {
  const microphone =
    snapshot.state.actual_input || snapshot.config.input_device || 'System default microphone';
  const microphoneMode = snapshot.config.keep_mic_open
    ? 'Always open, 300 ms pre-roll'
    : 'Microphone closes while idle';
  const microphoneDescription = `${escapeHtml(microphone)} · ${microphoneMode}`;
  return /* HTML */ `<section class="dictation">
    <div class="workbench">
      <span class="eyebrow">Background service</span>
      <h2 id="dictation-stage">${escapeHtml(snapshot.state.stage)}</h2>
      <div class="wave" role="img" aria-label="Live microphone level">
        ${Array.from({ length: 45 }, () => '<span></span>').join('')}
      </div>
      <p>Hold <kbd>${escapeHtml(snapshot.config.ptt_key)}</kbd> to speak. Release to transcribe.</p>
      <p class="quiet">${microphoneDescription}</p>
      <div class="actions">
        ${button('manual-record', manualRecording ? 'Stop and transcribe' : 'Record in app')}
        ${button('load', 'Load selected model')} ${button('cancel', 'Cancel work')}
      </div>
    </div>
    <div class="notice" id="service-error" role="alert"></div>
    <section class="recent">
      <h2>Most recent dictation</h2>
      <p id="recent"></p>
      ${button('copy-recent', 'Copy text')}
    </section>
    <details open>
      <summary>First-run permissions</summary>
      <p>${escapeHtml(snapshot.diagnostics.guidance)}</p>
    </details>
  </section>`;
}
