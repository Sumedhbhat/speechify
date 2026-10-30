import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './style.css';
type Obj = Record<string, any>;
const api = <T = any>(op: string, args: unknown = {}): Promise<T> => invoke('request', { op, args });
const esc = (s: unknown): string => String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
const app = document.querySelector<HTMLDivElement>('#app')!;
let manualRecording = false;
let page = 'Dictation', snapshot: Obj | null = null, rows: Obj[] = [], selected: number | null = null, downloaded = false;
let search = '', filter = '', dirty = false, refreshBusy = false;
let initialized = false, renderVersion = 0, historyRequest = 0;
let modelLanguage = '';
function languageGroup(description: string): string {
  const languages = description.trim().toLowerCase();
  if (languages === 'english' || languages === 'en') return 'english';
  if (!languages || languages === 'as trained') return 'unknown';
  if (languages.includes(',') || /\b\d+\b.*\blanguages\b/.test(languages)) return 'multilingual';
  return 'other';
}
const overlay = new URLSearchParams(location.search).has('overlay');
function error(e: unknown) { const el = document.querySelector('#message'); if (el) { el.textContent = String(e); el.classList.add('visible'); } }
function notify(s: string) { error(s); }
const button = (action: string, text: string, attrs = '') => `<button data-action="${action}" ${attrs}>${text}</button>`;
function shell() {
  app.innerHTML = `<aside><div class="brand"><svg viewBox="0 0 30 30" aria-hidden="true"><path d="M4 12v6m6-12v18m6-21v24m6-18v12m6-9v6"/></svg><span>Speechify<small>Tauri candidate</small></span></div><nav aria-label="Main">${['Dictation', 'History', 'Models', 'Learning', 'Settings'].map(p => `<button data-page="${p}" aria-current="${p === page ? 'page' : 'false'}">${p}</button>`).join('')}</nav><div class="sidebar-status"><span class="dot"></span><span id="readiness">Connecting to service</span><small id="active-model"></small></div></aside><main><header><h1 id="heading">${page}</h1><span class="quiet" id="device"></span></header><div id="message" role="status" aria-live="polite"></div><div id="view"></div></main>`;
}
function theme() { const t = snapshot!.config.theme; document.documentElement.dataset.theme = t === 'system' ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light') : t; }
function status() {
  const current = snapshot!; theme();
  const state = current.state;
  manualRecording = current.manual_recording === true;
  const record = document.querySelector('[data-action="manual-record"]'); if (record) record.textContent = manualRecording ? 'Stop and transcribe' : 'Record in app';
  if (overlay) { app.innerHTML = `<div class="overlay"><span class="dot ${state.stage === 'Recording' ? 'recording' : ''}"></span><strong>${esc(state.stage)}</strong><div class="meter"><i style="width:${Math.min(100, state.level * 400)}%"></i></div></div>`; return; }
  document.querySelector('#readiness')!.textContent = state.stage;
  document.querySelector('#active-model')!.textContent = current.models.find((m: Obj) => m.id === current.config.model)?.name ?? current.config.model;
  document.querySelector('#device')!.textContent = state.device;
  const stage = document.querySelector('#dictation-stage'); if (stage) stage.textContent = state.stage;
  const recent = document.querySelector('#recent'); if (recent) recent.textContent = state.recent || 'Your next dictation will appear here.';
  const wave = document.querySelectorAll<HTMLElement>('.wave span'); wave.forEach((bar, i) => { bar.style.height = `${4 + Math.min(90, state.level * 500) * (0.45 + 0.55 * Math.sin(i * 0.7) ** 2)}px`; });
  const progress = document.querySelector('#download-progress'); if (progress && state.download) { const p = state.download; progress.innerHTML = `<strong>${esc(p.id)}</strong> — ${esc(p.stage)} ${p.bytes ? `${(p.bytes / 1e6).toFixed(1)} / ${(p.total / 1e6).toFixed(1)} MB` : ''}${p.error ? `<p class="error">${esc(p.error)}</p>` : ''}`; }
  const err = document.querySelector('#service-error'); if (err) { err.textContent = state.error ?? ''; err.classList.toggle('visible', !!state.error); }
}
async function render() {
  if (!snapshot || overlay) return; const view = document.querySelector('#view')!;
  const version = ++renderVersion;
  document.querySelector('#heading')!.textContent = page;
  document.querySelectorAll<HTMLElement>('[data-page]').forEach(b => b.setAttribute('aria-current', b.dataset.page === page ? 'page' : 'false'));
  if (page === 'Dictation') {
    view.innerHTML = `<section class="dictation"><div class="workbench"><span class="eyebrow">Background service</span><h2 id="dictation-stage">${esc(snapshot.state.stage)}</h2><div class="wave" role="img" aria-label="Live microphone level">${Array.from({ length: 45 }, () => '<span></span>').join('')}</div><p>Hold <kbd>${esc(snapshot.config.ptt_key)}</kbd> to speak. Release to transcribe.</p><p class="quiet">${esc(snapshot.state.actual_input || snapshot.config.input_device || 'System default microphone')} · ${snapshot.config.keep_mic_open ? 'Always open, 300 ms pre-roll' : 'Microphone closes while idle'}</p><div class="actions">${button('manual-record', manualRecording ? 'Stop and transcribe' : 'Record in app')}${button('load', 'Load selected model')}${button('cancel', 'Cancel work')}</div></div><div class="notice" id="service-error" role="alert"></div><section class="recent"><h2>Most recent dictation</h2><p id="recent"></p>${button('copy-recent', 'Copy text')}</section><details open><summary>First-run permissions</summary><p>${esc(snapshot.diagnostics.guidance)}</p></details></section>`;
  } else if (page === 'Models') {
    const models = snapshot.models.filter((m: Obj) => (!downloaded || m.installed) && (!modelLanguage || languageGroup(m.languages) === modelLanguage));
    view.innerHTML = `<div class="toolbar"><p>One active model, kept loaded between dictations.</p><label>Languages<select id="model-language">${[['', 'All languages'], ['english', 'English only'], ['multilingual', 'Multilingual'], ['other', 'Other languages']].map(([value, label]) => `<option value="${value}" ${modelLanguage === value ? 'selected' : ''}>${label}</option>`).join('')}</select></label><label class="check"><input type="checkbox" id="downloaded" ${downloaded ? 'checked' : ''}> Downloaded only</label></div><div id="download-progress" role="status"></div><div class="model-list">${models.map((m: Obj) => `<article class="model-row"><div><h2>${esc(m.name)}</h2><p>${esc(m.languages)} · ${m.vram_gb ?? '—'} GB memory · ${m.gguf ? (m.gguf.size / 1e9).toFixed(2) : m.download_gb ?? '—'} GB download</p><p class="metadata">${esc(m.runtime)}${m.gated && !m.gguf ? ' · Upstream terms required' : ''}${m.punctuates === false ? ' · Punctuation is unreliable' : ''}</p><p class="metadata">${esc(m.notes)}</p><span class="quiet">${m.id === snapshot!.config.model ? 'Selected' : m.installed ? 'Installed' : 'Not installed'}</span></div><div class="model-actions">${m.installed ? button('use-model', 'Use model', `data-id="${esc(m.id)}"`) : button('install', m.gguf ? 'Install' : 'Install optional model', `data-id="${esc(m.id)}"`)}${m.gguf ? button('reuse', 'Copy existing GGUF', `data-id="${esc(m.id)}"`) : ''}</div></article>`).join('') || '<p class="empty" role="status">No models match these filters. Try All languages or turn off Downloaded only.</p>'}</div><div class="actions">${button('cancel', 'Cancel download or inference')}</div><p class="quiet">GPU choices depend on the bundled native library. CUDA-specific libraries require a separate native build. Beam and compute-type settings apply to the Python Whisper exception; native GGUF quantization is fixed.</p>`;
  } else if (page === 'History') {
    view.innerHTML = `<div class="toolbar"><label class="search">Search recordings<input id="search" type="search" value="${esc(search)}"></label><label>Show<select id="filter">${[['', 'All'], ['edited', 'Edited'], ['unedited', 'Unedited'], ['excluded', 'Excluded']].map(([v, l]) => `<option value="${v}" ${filter === v ? 'selected' : ''}>${l}</option>`).join('')}</select></label>${button('export', 'Export labelled WAVs')}</div><div class="history-split"><div id="history-list" aria-label="Recordings"></div><section id="history-detail"></section></div>`;
    await history();
  } else if (page === 'Learning') {
    const data = await api<Obj>('learning');
    if (version !== renderVersion) return;
    view.innerHTML = `<p>Corrections become vocabulary hints immediately. Repeated word-level fixes become automatic rules after ${snapshot.config.autocorrect_after} corrections.</p><section><h2>Background learning</h2><div id="download-progress" role="status"></div><dl><dt>Teacher</dt><dd>${esc(typeof data.teacher === 'string' ? data.teacher : JSON.stringify(data.teacher))}</dd><dt>Training</dt><dd>${esc(typeof data.training === 'string' ? data.training : JSON.stringify(data.training))}</dd></dl><p class="quiet">Runs while idle and on mains. Human corrections take precedence over confident teacher labels. Uncorrected dictation guesses are never training labels.</p><div class="actions">${button('install', 'Install training environment', 'data-id="training"')}${button('train', 'Run fine-tuning')}${button('cancel', 'Pause current job')}${button('rollback', 'Roll back version')}</div><p class="quiet">Training uses the optional Python/CUDA environment. Adoption requires improved held-out personal WER and the LibriSpeech regression gate.</p></section><section><h2>Word rules</h2>${data.rules.length ? data.rules.map((r: Obj) => `<div class="learning-row"><span>${esc(r.wrong)} → <strong>${esc(r.right)}</strong><small>${r.count} corrections</small></span>${button('forget-rule', 'Forget', `data-key="${esc(r.wrong)}"`)}</div>`).join('') : '<p class="empty">No rules yet. Correct a recording in History to start learning.</p>'}</section><section><h2>Vocabulary</h2>${data.vocab.length ? data.vocab.map((v: Obj) => `<div class="learning-row"><span>${esc(v.term)}</span>${button('forget-vocab', 'Forget', `data-key="${esc(v.term)}"`)}</div>`).join('') : '<p class="empty">Your corrected words will appear here.</p>'}</section><section><h2>Trained versions</h2>${data.versions.length ? data.versions.map((v: Obj) => `<details><summary>${esc(v.name)}</summary><pre>${esc(JSON.stringify({ personal: v.gate1, regression: v.gate2 }, null, 2))}</pre></details>`).join('') : '<p class="empty">No trained versions. The original model remains active.</p>'}</section>`;
  } else settings(view);
  status();
}
async function history() {
  const version = renderVersion, request = ++historyRequest;
  const data = await api<Obj[]>('history', { search, filter });
  if (version !== renderVersion || request !== historyRequest || page !== 'History') return;
  rows = data;
  if (!rows.some(r => r.id === selected)) selected = rows[0]?.id ?? null; document.querySelector('#history-list')!.innerHTML = rows.length ? rows.map(r => `<button class="sample ${r.id === selected ? 'selected' : ''}" data-sample="${r.id}"><span>${esc((r.corrected ?? r.pasted).slice(0, 150))}</span><small>${new Date(r.ts * 1000).toLocaleString()}${r.corrected ? ' · Edited' : ''}${r.excluded ? ' · Excluded' : ''}</small></button>`).join('') : '<p class="empty">No recordings match. Dictate to start your history.</p>'; detail();
}
function detail() { const r = rows.find(r => r.id === selected); const el = document.querySelector('#history-detail')!; if (!r) { el.innerHTML = '<p class="empty">Select a recording to listen or correct it.</p>'; return; } dirty = false; el.innerHTML = `<h2>Correction</h2><p class="metadata">${new Date(r.ts * 1000).toLocaleString()}</p>${r.audio ? button('play', 'Play recording') + '<audio id="playback" controls hidden></audio>' : '<p class="quiet">Audio retention is off. The latest clip stays in memory until the next dictation or quit; saving a correction preserves that audio.</p>'}<label for="correction">Corrected transcript</label><textarea id="correction" rows="8">${esc(r.corrected ?? r.pasted)}</textarea><div class="actions">${button('save', 'Save only', 'class="primary"')}${button('replace', 'Save and replace', 'title="Requires a proven original text range and unchanged caret; unsupported targets save only"')}${button('copy', 'Copy')}</div><p class="metadata">Shift+Enter saves only. Ctrl+Enter requests replacement. Unsafe replacement is refused.</p><details><summary>Original pasted text</summary><p>${esc(r.pasted)}</p></details><div class="actions">${button('revert', 'Revert correction')}${button('exclude', r.excluded ? 'Include in training' : 'Exclude from training')}${button('delete', 'Delete recording', 'class="danger"')}</div>${r.teacher_text ? `<details><summary>Teacher label · ${Number(r.teacher_confidence ?? 0).toFixed(2)} confidence</summary><p>${esc(r.teacher_text)}</p><p>${esc(r.teacher_note ?? 'Accepted subject to configured threshold')}</p></details>` : ''}`; }
const labels: Record<string, string> = { ptt_key: 'Hold-to-talk key or combination', correct_hotkey: 'Correction shortcut', history_hotkey: 'History shortcut', models_hotkey: 'Models shortcut', input_device: 'Input device', keep_mic_open: 'Keep microphone open with 300 ms pre-roll', min_hold_ms: 'Minimum hold (ms)', device: 'Inference device', compute_type: 'Whisper compute type', language: 'Language code (empty for auto)', beam_size: 'Whisper beam size', onnx_threads: 'CPU inference threads', autocorrect_after: 'Corrections before automatic replacement', keep_all_audio: 'Retain dictation audio', llm_url: 'Optional OpenAI-compatible cleanup URL', llm_model: 'Cleanup model', learn_auto: 'Enable idle learning', teacher: 'Teacher model ID, auto, or off', idle_minutes: 'Idle minutes before learning', teacher_on_cpu: 'Allow teacher inference on CPU', teacher_min_confidence: 'Minimum teacher confidence', retrain_after_labels: 'New labels before training', retrain_after_days: 'Days between training', max_regression_wer: 'Maximum LibriSpeech regression (WER points)', keep_versions: 'Trained versions to retain', theme: 'Theme' };
function field(k: string) { const v = snapshot!.config[k]; const options: Record<string, string[]> = { device: ['auto', 'cuda', 'vulkan', 'metal', 'cpu'], compute_type: ['auto', 'float16', 'float32', 'int8', 'int8_float16'], theme: ['system', 'dark', 'light'] }; return `<label class="${typeof v === 'boolean' ? 'check' : ''}">${typeof v !== 'boolean' ? esc(labels[k]) : ''}${options[k] ? `<select name="${k}">${options[k].map(o => `<option ${o === v ? 'selected' : ''}>${o}</option>`).join('')}</select>` : typeof v === 'boolean' ? `<input name="${k}" type="checkbox" ${v ? 'checked' : ''}>${esc(labels[k])}` : `<input name="${k}" ${k === 'input_device' ? 'list="inputs"' : ''} type="${typeof v === 'number' ? 'number' : 'text'}" ${typeof v === 'number' ? 'step="any"' : ''} value="${esc(v)}">`}</label>`; }
function settings(view: Element) { const groups: [string, string[]][] = [['General', ['theme', 'learn_auto']], ['Microphone', ['input_device', 'keep_mic_open']], ['Shortcuts', ['ptt_key', 'min_hold_ms', 'correct_hotkey', 'history_hotkey', 'models_hotkey']], ['Inference', ['device', 'compute_type', 'language', 'beam_size', 'onnx_threads']], ['Learning', ['autocorrect_after', 'teacher', 'teacher_on_cpu', 'teacher_min_confidence', 'idle_minutes', 'retrain_after_labels', 'retrain_after_days', 'max_regression_wer', 'keep_versions']], ['Privacy / Storage', ['keep_all_audio', 'llm_url', 'llm_model']]];
  view.innerHTML = `<p class="notice visible">${esc(snapshot!.diagnostics.guidance)}</p><form id="settings"><datalist id="inputs"></datalist>${groups.map(([title, keys]) => `<fieldset><legend>${title}</legend><div class="settings-grid">${keys.map(field).join('')}</div></fieldset>`).join('')}<div class="actions sticky"><button class="primary" type="submit">Apply settings</button>${button('refresh-inputs', 'Refresh microphones', 'type="button"')}</div></form><section><h2>Startup at login</h2><p>Only changes the candidate’s own login entry.</p><div class="actions">${button('autostart-on', 'Enable')}${button('autostart-off', 'Disable')}</div></section><section><h2>Data and diagnostics</h2><p class="path">${esc(snapshot!.data_dir)}</p><p>Timings: <code>logs/timings.jsonl</code>. Rotates at 2 MB; transcripts are excluded from generic logs.</p><p>Clipboard snapshots preserve supported text and rich formats. Unsupported or oversized formats refuse automatic paste. Newer copies are protected by ownership or generation checks. macOS has no atomic compare-and-restore API.</p><div class="actions">${button('export-config', 'Export TOML')}${button('import-config', 'Import TOML')}${button('import-data', 'Copy existing Speechify data')}</div><textarea id="config-transfer" rows="6" aria-label="TOML import or export" placeholder="Export settings here, or paste TOML to import"></textarea></section>`;
  void inputs();
}
async function inputs() { try { const names = await api<string[]>('devices'); const el = document.querySelector('#inputs'); if (el) el.innerHTML = names.map(n => `<option value="${esc(n)}"></option>`).join(''); } catch (e) { error(e); } }
app.addEventListener('submit', e => { e.preventDefault(); const form = e.target as HTMLFormElement; const cfg = { ...snapshot!.config }; for (const el of form.elements) { if (!(el instanceof HTMLInputElement || el instanceof HTMLSelectElement) || !el.name) continue; cfg[el.name] = el instanceof HTMLInputElement && el.type === 'checkbox' ? el.checked : typeof cfg[el.name] === 'number' ? Number(el.value) : el.value; } void api('settings', cfg).then(async () => { snapshot = await api('snapshot'); dirty = false; notify('Settings applied.'); status(); }).catch(error); });
app.addEventListener('input', e => { const el = e.target as HTMLInputElement; if (el.id === 'search') { search = el.value; void history().catch(error); } if (el.id === 'correction' || el.closest('#settings')) dirty = true; });
app.addEventListener('change', e => {
  const el = e.target as HTMLInputElement;
  if (el.id === 'filter') { filter = el.value; void history().catch(error); }
  if (el.id === 'model-language') {
    const focused = document.activeElement === el;
    modelLanguage = el.value;
    void render().then(() => { if (focused) document.querySelector<HTMLSelectElement>('#model-language')?.focus(); }).catch(error);
  }
  if (el.id === 'downloaded') { downloaded = el.checked; void render().catch(error); }
});
app.addEventListener('keydown', e => { if ((e.target as HTMLElement).id === 'correction' && e.key === 'Enter' && (e.shiftKey || e.ctrlKey)) { e.preventDefault(); void action(e.ctrlKey ? 'replace' : 'save', {} as HTMLElement).catch(error); } });
app.addEventListener('click', e => { const b = (e.target as HTMLElement).closest<HTMLElement>('button'); if (!b) return; if (b.dataset.page) { if (dirty && !confirm('Discard unsaved edits?')) return; dirty = false; page = b.dataset.page; void render().catch(error); } else if (b.dataset.sample) { if (dirty && !confirm('Discard unsaved correction?')) return; selected = Number(b.dataset.sample); void history().catch(error); } else if (b.dataset.action) { void action(b.dataset.action, b).catch(error); } });
async function action(a: string, b: HTMLElement) { const r = rows.find(r => r.id === selected); const id = selected;
  if (['load', 'cancel', 'train', 'rollback'].includes(a)) { await api(a); }
  else if (a === 'manual-record') { await api(manualRecording ? 'manual_stop' : 'manual_start'); manualRecording = !manualRecording; b.textContent = manualRecording ? 'Stop and transcribe' : 'Record in app'; }
  else if (a === 'install') await api('install', { id: b.dataset.id });
  else if (a === 'use-model') { await api('settings', { ...snapshot!.config, model: b.dataset.id }); }
  else if (a === 'reuse') { const path = prompt('Full path to the existing GGUF. It will be checksum-validated and copied; the source is never modified.'); if (path) await api('reuse_model', { id: b.dataset.id, path }); }
  else if (a === 'copy-recent') await api('copy', { text: snapshot!.state.recent });
  else if (a === 'copy') await api('copy', { text: (document.querySelector('#correction') as HTMLTextAreaElement).value });
  else if (a === 'save' || a === 'replace') { await api('correct', { id, text: (document.querySelector('#correction') as HTMLTextAreaElement).value, replace: a === 'replace' }); dirty = false; await history(); }
  else if (a === 'revert') { await api('revert', { id }); await history(); }
  else if (a === 'exclude') { await api('exclude', { id, excluded: !r?.excluded }); await history(); }
  else if (a === 'delete' && confirm('Delete this recording and its candidate-owned audio?')) { await api('delete', { id }); await history(); }
  else if (a === 'play') { const data = await api<string>('audio', { id }); const bytes = Uint8Array.from(atob(data), c => c.charCodeAt(0)); const audio = document.querySelector<HTMLAudioElement>('#playback')!; if (audio.src.startsWith('blob:')) URL.revokeObjectURL(audio.src); audio.src = URL.createObjectURL(new Blob([bytes], { type: 'audio/wav' })); audio.hidden = false; await audio.play(); }
  else if (a.startsWith('forget-')) { await api('forget', { kind: a.slice(7), key: b.dataset.key }); await render(); }
  else if (a === 'export') notify(`Exported labelled audio to ${await api('export')}`);
  else if (a === 'refresh-inputs') await inputs();
  else if (a === 'export-config') (document.querySelector('#config-transfer') as HTMLTextAreaElement).value = await api('export_config');
  else if (a === 'import-config') { await api('import_config', { text: (document.querySelector('#config-transfer') as HTMLTextAreaElement).value }); snapshot = await api('snapshot'); await render(); }
  else if (a === 'import-data') { const path = prompt('Existing Speechify data directory. Copies a consistent SQLite backup and referenced audio into empty candidate history.'); if (path) notify(`Imported ${await api('import', { path })} audio files. Source unchanged.`); }
  else if (a.startsWith('autostart-')) { await api('autostart', { enabled: a === 'autostart-on' }); notify(a === 'autostart-on' ? 'Candidate login entry enabled.' : 'Candidate login entry removed.'); }
  snapshot = await api('snapshot'); status(); if (page === 'Models' && ['install', 'use-model', 'reuse'].includes(a)) await render();
}
async function refresh() {
  if (refreshBusy) return; refreshBusy = true;
  try {
    const old = snapshot?.state.download?.stage;
    snapshot = await api('snapshot');
    status();
    if (!initialized) {
      await render();
      await listen<string>('navigate', e => { if (dirty) return; page = e.payload; void render().catch(error); });
      initialized = true;
    }
    if (page === 'Models' && old !== snapshot!.state.download?.stage && snapshot!.state.download?.stage === 'Installed') await render();
  } catch (e) { error(initialized ? e : `Cannot connect to the Rust service: ${e}`); }
  finally { refreshBusy = false; }
}
async function start() { if (overlay) { document.body.classList.add('overlay-body'); } else shell(); await refresh(); setInterval(() => void refresh(), overlay ? 80 : 700); }
void start();
