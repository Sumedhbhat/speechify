import { button, escapeHtml } from '../ui';
import type { Recording } from '../types';

export function renderHistory(search: string, filter: string): string {
  return /* HTML */ `<div class="toolbar">
      <label class="search"
        >Search recordings<input id="search" type="search" value="${escapeHtml(search)}" /></label
      ><label
        >Show<select id="filter">
          ${[
            ['', 'All'],
            ['edited', 'Edited'],
            ['unedited', 'Unedited'],
            ['excluded', 'Excluded'],
          ]
            .map(
              ([v, l]) =>
                /* HTML */ `<option value="${v}" ${filter === v ? 'selected' : ''}>${l}</option>`,
            )
            .join('')}
        </select></label
      >${button('export', 'Export labelled WAVs')}
    </div>
    <div class="history-split">
      <div id="history-list" aria-label="Recordings"></div>
      <section id="history-detail"></section>
    </div>`;
}

export function renderHistoryList(rows: Recording[], selected: number | null): string {
  return /* HTML */ rows.length
    ? rows
        .map(
          (r) =>
            /* HTML */ `<button
              class="sample ${r.id === selected ? 'selected' : ''}"
              data-sample="${r.id}"
            >
              <span>${escapeHtml((r.corrected ?? r.pasted).slice(0, 150))}</span
              ><small
                >${new Date(r.ts * 1000).toLocaleString()}${r.corrected ? ' · Edited' : ''}${r.excluded ? ' · Excluded' : ''}</small
              >
            </button>`,
        )
        .join('')
    : '<p class="empty">No recordings match. Dictate to start your history.</p>';
}

export function renderHistoryDetail(recording: Recording | undefined): string {
  if (!recording) return '<p class="empty">Select a recording to listen or correct it.</p>';
  const r = recording;
  return /* HTML */ `<h2>Correction</h2>
    <p class="metadata">${new Date(r.ts * 1000).toLocaleString()}</p>
    ${r.audio ? button('play', 'Play recording') + '<audio id="playback" controls hidden></audio>' : '<p class="quiet">Audio retention is off. The latest clip stays in memory until the next dictation or quit; saving a correction preserves that audio.</p>'}<label
      for="correction"
      >Corrected transcript</label
    ><textarea id="correction" rows="8">${escapeHtml(r.corrected ?? r.pasted)}</textarea>
    <div class="actions">
      ${button('save', 'Save only', 'class="primary"')}
      ${button('replace', 'Save and replace', 'title="Requires a proven original text range and unchanged caret; unsupported targets save only"')}
      ${button('copy', 'Copy')}
    </div>
    <p class="metadata">
      Shift+Enter saves only. Ctrl+Enter requests replacement. Unsafe replacement is refused.
    </p>
    <details>
      <summary>Original pasted text</summary>
      <p>${escapeHtml(r.pasted)}</p>
    </details>
    <div class="actions">
      ${button('revert', 'Revert correction')}
      ${button('exclude', r.excluded ? 'Include in training' : 'Exclude from training')}
      ${button('delete', 'Delete recording', 'class="danger"')}
    </div>
    ${
      r.teacher_text
        ? /* HTML */ `<details>
            <summary>
              Teacher label · ${Number(r.teacher_confidence ?? 0).toFixed(2)} confidence
            </summary>
            <p>${escapeHtml(r.teacher_text)}</p>
            <p>${escapeHtml(r.teacher_note ?? 'Accepted subject to configured threshold')}</p>
          </details>`
        : ''
    }`;
}
