import { button, escapeHtml } from '../ui';
import type { Config, Snapshot } from '../types';

// Form controls are the untyped boundary. Interpret each value using the
// persisted setting's type, so checkboxes and numeric fields retain theirs.
export function readSettings(form: HTMLFormElement, config: Config): Config {
  const changes: Record<string, string | number | boolean> = {};
  for (const control of form.elements) {
    if (
      !(control instanceof HTMLInputElement || control instanceof HTMLSelectElement) ||
      !control.name
    )
      continue;
    const key = control.name as keyof Config;
    changes[key] =
      control instanceof HTMLInputElement && control.type === 'checkbox'
        ? control.checked
        : typeof config[key] === 'number'
          ? Number(control.value)
          : control.value;
  }
  return { ...config, ...changes };
}

const labels: Record<keyof Config, string> = {
  model: 'Model',
  ptt_key: 'Hold-to-talk key or combination',
  correct_hotkey: 'Correction shortcut',
  history_hotkey: 'History shortcut',
  models_hotkey: 'Models shortcut',
  input_device: 'Input device',
  keep_mic_open: 'Keep microphone open with 300 ms pre-roll',
  min_hold_ms: 'Minimum hold (ms)',
  device: 'Inference device',
  compute_type: 'Whisper compute type',
  language: 'Language code (empty for auto)',
  beam_size: 'Whisper beam size',
  onnx_threads: 'CPU inference threads',
  autocorrect_after: 'Corrections before automatic replacement',
  keep_all_audio: 'Retain dictation audio',
  llm_url: 'Optional OpenAI-compatible cleanup URL',
  llm_model: 'Cleanup model',
  learn_auto: 'Enable idle learning',
  teacher: 'Teacher model ID, auto, or off',
  idle_minutes: 'Idle minutes before learning',
  teacher_on_cpu: 'Allow teacher inference on CPU',
  teacher_min_confidence: 'Minimum teacher confidence',
  retrain_after_labels: 'New labels before training',
  retrain_after_days: 'Days between training',
  max_regression_wer: 'Maximum LibriSpeech regression (WER points)',
  keep_versions: 'Trained versions to retain',
  theme: 'Theme',
};
const choices: Partial<Record<keyof Config, string[]>> = {
  device: ['auto', 'cuda', 'vulkan', 'metal', 'cpu'],
  compute_type: ['auto', 'float16', 'float32', 'int8', 'int8_float16'],
  theme: ['system', 'dark', 'light'],
};

function field(config: Config, key: keyof Config): string {
  const value = config[key];
  const options = choices[key];
  if (options) {
    return /* HTML */ `<label class="">
      ${escapeHtml(labels[key])}
      <select name="${key}">
        ${options.map((option) => /* HTML */ `<option ${option === value ? 'selected' : ''}>${option}</option>`).join('')}
      </select>
    </label>`;
  }
  if (typeof value === 'boolean') {
    return /* HTML */ `<label class="check">
      <input name="${key}" type="checkbox" ${value ? 'checked' : ''} />
      ${escapeHtml(labels[key])}
    </label>`;
  }
  return /* HTML */ `<label class="">
    ${escapeHtml(labels[key])}
    <input
      name="${key}"
      ${key === 'input_device' ? 'list="inputs"' : ''}
      type="${typeof value === 'number' ? 'number' : 'text'}"
      ${typeof value === 'number' ? 'step="any"' : ''}
      value="${escapeHtml(value)}"
    />
  </label>`;
}

export function renderSettings(snapshot: Snapshot): string {
  const groups: [string, (keyof Config)[]][] = [
    ['General', ['theme', 'learn_auto']],
    ['Microphone', ['input_device', 'keep_mic_open']],
    ['Shortcuts', ['ptt_key', 'min_hold_ms', 'correct_hotkey', 'history_hotkey', 'models_hotkey']],
    ['Inference', ['device', 'compute_type', 'language', 'beam_size', 'onnx_threads']],
    [
      'Learning',
      [
        'autocorrect_after',
        'teacher',
        'teacher_on_cpu',
        'teacher_min_confidence',
        'idle_minutes',
        'retrain_after_labels',
        'retrain_after_days',
        'max_regression_wer',
        'keep_versions',
      ],
    ],
    ['Privacy / Storage', ['keep_all_audio', 'llm_url', 'llm_model']],
  ];
  return /* HTML */ `<p class="notice visible">${escapeHtml(snapshot.diagnostics.guidance)}</p>
    <form id="settings">
      <datalist id="inputs"></datalist>${groups
        .map(
          ([title, keys]) =>
            /* HTML */ `<fieldset>
              <legend>${title}</legend>
              <div class="settings-grid">
                ${keys.map((key) => field(snapshot.config, key)).join('')}
              </div>
            </fieldset>`,
        )
        .join('')}
      <div class="actions sticky">
        <button class="primary" type="submit">Apply settings</button
        >${button('refresh-inputs', 'Refresh microphones', 'type="button"')}
      </div>
    </form>
    <section>
      <h2>Startup at login</h2>
      <p>Starts Speechify when you sign in.</p>
      <div class="actions">
        ${button('autostart-on', 'Enable')} ${button('autostart-off', 'Disable')}
      </div>
    </section>
    <section>
      <h2>Data and diagnostics</h2>
      <p>Speechify ${escapeHtml(snapshot.version)}</p>
      <p class="path">${escapeHtml(snapshot.data_dir)}</p>
      <p>
        Timings: <code>logs/timings.jsonl</code>. Rotates at 2 MB; transcripts are excluded from
        generic logs.
      </p>
      <p>
        Clipboard snapshots preserve supported text and rich formats. Unsupported or oversized
        formats refuse automatic paste. Newer copies are protected by ownership or generation
        checks. macOS has no atomic compare-and-restore API.
      </p>
      <div class="actions">
        ${button('export-config', 'Export TOML')} ${button('import-config', 'Import TOML')}
        ${button('import-data', 'Copy existing Speechify data')}
      </div>
      <textarea
        id="config-transfer"
        rows="6"
        aria-label="TOML import or export"
        placeholder="Export settings here, or paste TOML to import"
      ></textarea>
    </section>`;
}
