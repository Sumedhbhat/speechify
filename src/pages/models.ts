import { button, escapeHtml } from '../ui';
import type { Snapshot } from '../types';

export function renderModels(
  snapshot: Snapshot,
  downloaded: boolean,
  modelLanguage: string,
): string {
  const models = snapshot.models.filter(
    (m) =>
      (!downloaded || m.installed) &&
      (!modelLanguage || languageGroup(m.languages) === modelLanguage),
  );
  return /* HTML */ `<div class="toolbar">
      <p>One active model, kept loaded between dictations.</p>
      <label
        >Languages<select id="model-language">
          ${[
            ['', 'All languages'],
            ['english', 'English only'],
            ['multilingual', 'Multilingual'],
            ['other', 'Other languages'],
          ]
            .map(
              ([value, label]) =>
                /* HTML */ `<option value="${value}" ${modelLanguage === value ? 'selected' : ''}>
                  ${label}
                </option>`,
            )
            .join('')}
        </select></label
      ><label class="check"
        ><input type="checkbox" id="downloaded" ${downloaded ? 'checked' : ''} /> Downloaded
        only</label
      >
    </div>
    <div id="download-progress" role="status"></div>
    <div class="model-list">
      ${
        models
          .map(
            (m) =>
              /* HTML */ `<article class="model-row">
                <div>
                  <h2>${escapeHtml(m.name)}</h2>
                  <p>
                    ${escapeHtml(m.languages)} · ${m.vram_gb ?? '—'} GB memory ·
                    ${m.gguf ? (m.gguf.size / 1e9).toFixed(2) : (m.download_gb ?? '—')} GB download
                  </p>
                  <p class="metadata">
                    ${escapeHtml(m.runtime)}${m.gated && !m.gguf ? ' · Upstream terms required' : ''}${m.punctuates === false ? ' · Punctuation is unreliable' : ''}
                  </p>
                  <p class="metadata">${escapeHtml(m.notes)}</p>
                  <span class="quiet"
                    >${m.id === snapshot.config.model ? 'Selected' : m.installed ? 'Installed' : 'Not installed'}</span
                  >
                </div>
                <div class="model-actions">
                  ${m.installed ? button('use-model', 'Use model', `data-id="${escapeHtml(m.id)}"`) : button('install', m.gguf ? 'Install' : 'Install optional model', `data-id="${escapeHtml(m.id)}"`)}${m.gguf ? button('reuse', 'Copy existing GGUF', `data-id="${escapeHtml(m.id)}"`) : ''}
                </div>
              </article>`,
          )
          .join('') ||
        '<p class="empty" role="status">No models match these filters. Try All languages or turn off Downloaded only.</p>'
      }
    </div>
    <div class="actions">${button('cancel', 'Cancel download or inference')}</div>
    <p class="quiet">
      GPU choices depend on the bundled native library. CUDA-specific libraries require a separate
      native build. Beam and compute-type settings apply to the Python Whisper exception; native
      GGUF quantization is fixed.
    </p>`;
}

function languageGroup(description: string): string {
  const languages = description.trim().toLowerCase();
  if (languages === 'english' || languages === 'en') return 'english';
  if (!languages || languages === 'as trained') return 'unknown';
  if (languages.includes(',') || /\b\d+\b.*\blanguages\b/.test(languages)) return 'multilingual';
  return 'other';
}
