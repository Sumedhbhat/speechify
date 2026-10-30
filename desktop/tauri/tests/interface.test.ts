import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import defaults from '../src-tauri/resources/defaults.json';

const bridge = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: bridge.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: bridge.listen }));

type Obj = Record<string, any>;
let snapshot: Obj;
let recordings: Obj[];
let learning: Obj;
let navigate: (event: { payload: string }) => void;
let operations: [string, Obj][];

const model = (id: string, languages: string, extra: Obj = {}) => ({
  id, name: id, languages, installed: false, runtime: 'Native GGUF', notes: '', ...extra,
});
const sample = (id: number, extra: Obj = {}) => ({
  id, ts: 1_700_000_000 + id, pasted: `Transcript ${id}`, corrected: null,
  audio: `clip-${id}.wav`, excluded: 0, ...extra,
});
const element = <T extends Element = HTMLElement>(selector: string): T => {
  const el = document.querySelector<T>(selector);
  expect(el, `Missing control: ${selector}`).not.toBeNull();
  return el!;
};
const settle = async () => { for (let i = 0; i < 25; i++) await Promise.resolve(); };
const click = async (selector: string) => {
  element<HTMLElement>(selector).click();
  await settle();
};
const page = (name: string) => click(`[data-page="${name}"]`);
const action = (name: string) => click(`[data-action="${name}"]`);
async function change(selector: string, value: string | boolean, event = 'change') {
  const el = element<HTMLInputElement>(selector);
  if (typeof value === 'boolean') el.checked = value; else el.value = value;
  el.dispatchEvent(new Event(event, { bubbles: true }));
  await settle();
}
async function start(query = '') {
  window.history.replaceState({}, '', `/${query}`);
  await import('../src/main');
  await settle();
}

beforeEach(() => {
  vi.resetModules();
  vi.useFakeTimers();
  document.body.className = '';
  document.body.innerHTML = '<div id="app"></div>';
  snapshot = {
    config: { ...defaults }, data_dir: '/candidate', manual_recording: false,
    state: { stage: 'Ready', device: 'cpu', level: 0, recent: '', error: null, download: null },
    models: [
      model('parakeet-tdt-0.6b-v3', '25 European languages', { installed: true, gguf: { size: 700_000_000 } }),
      model('english', 'English', { gguf: { size: 80_000_000 }, punctuates: false }),
      model('optional', 'en', { gated: true, download_gb: 2 }),
      model('french', 'French', { installed: true, vram_gb: 3 }),
      model('comma', 'English, French'), model('trained', 'as trained', { notes: null }), model('unknown', ''),
    ],
    diagnostics: { guidance: 'Allow microphone access in system settings.' },
  };
  recordings = [sample(2), sample(1, { audio: null, corrected: 'A correction', excluded: 1 })];
  learning = { rules: [], vocab: [], versions: [], teacher: 'Idle', training: 'Not started' };
  operations = [];
  bridge.listen.mockReset().mockImplementation(async (_event, fn) => { navigate = fn; return () => {}; });
  bridge.invoke.mockReset().mockImplementation(async (_command, { op, args }) => {
    operations.push([op, structuredClone(args)]);
    if (op === 'snapshot') return structuredClone(snapshot);
    if (op === 'history') return structuredClone(recordings.filter(r =>
      (r.corrected ?? r.pasted).includes(args.search) &&
      (args.filter !== 'excluded' || r.excluded) &&
      (args.filter !== 'edited' || r.corrected !== null) &&
      (args.filter !== 'unedited' || r.corrected === null)));
    if (op === 'learning') return structuredClone(learning);
    if (op === 'devices') return ['USB "Microphone"', 'Built-in'];
    if (op === 'manual_start' || op === 'manual_stop') snapshot.manual_recording = op === 'manual_start';
    if (op === 'settings') snapshot.config = structuredClone(args);
    if (op === 'correct') recordings.find(r => r.id === args.id)!.corrected = args.text;
    if (op === 'revert') recordings.find(r => r.id === args.id)!.corrected = null;
    if (op === 'exclude') recordings.find(r => r.id === args.id)!.excluded = Number(args.excluded);
    if (op === 'delete') recordings = recordings.filter(r => r.id !== args.id);
    if (op === 'export') return '/candidate/exports/dataset';
    if (op === 'export_config') return 'language = "fr"';
    if (op === 'import_config') snapshot.config.language = 'fr';
    if (op === 'import') return 3;
    if (op === 'audio') return btoa('RIFFwave');
    return true;
  });
  vi.stubGlobal('matchMedia', vi.fn(() => ({ matches: false })));
  vi.stubGlobal('confirm', vi.fn(() => true));
  vi.stubGlobal('prompt', vi.fn(() => '/existing/model.gguf'));
});
afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); vi.unstubAllGlobals(); });

describe('dictation', () => {
  it('shows readiness, microphone guidance and the latest transcript without interpreting it as HTML', async () => {
    snapshot.state.recent = '<img src=x onerror=alert(1)> & "hello"';
    snapshot.state.error = 'Microphone disconnected';
    snapshot.state.level = 0.5;
    await start();
    expect(element('#readiness').textContent).toBe('Ready');
    expect(element('#active-model').textContent).toBe('parakeet-tdt-0.6b-v3');
    expect(element('#recent').textContent).toBe(snapshot.state.recent);
    expect(document.querySelector('#recent img')).toBeNull();
    expect(element('#service-error').classList.contains('visible')).toBe(true);
    expect(document.body.textContent).toContain(snapshot.diagnostics.guidance);
    await action('copy-recent');
    expect(operations).toContainEqual(['copy', { text: snapshot.state.recent }]);
  });

  it('starts and stops an app recording and reflects service updates', async () => {
    await start();
    expect(element('#recent').textContent).toContain('Your next dictation');
    await action('manual-record');
    expect(element('[data-action="manual-record"]').textContent).toBe('Stop and transcribe');
    await action('manual-record');
    expect(element('[data-action="manual-record"]').textContent).toBe('Record in app');
    expect(operations.filter(([op]) => op.startsWith('manual_'))).toEqual([
      ['manual_start', {}], ['manual_stop', {}],
    ]);
    snapshot.manual_recording = true;
    snapshot.state.stage = 'Recording';
    await vi.advanceTimersByTimeAsync(700);
    expect(element('#dictation-stage').textContent).toBe('Recording');
    expect(element('[data-action="manual-record"]').textContent).toBe('Stop and transcribe');
    await action('load'); await action('cancel');
    expect(operations).toContainEqual(['load', {}]);
    expect(operations).toContainEqual(['cancel', {}]);
  });

  it('shows configured microphone, pre-roll and unknown model names', async () => {
    snapshot.config.keep_mic_open = true;
    snapshot.config.input_device = 'USB mic';
    snapshot.config.model = 'ft:my-voice';
    await start();
    expect(document.body.textContent).toContain('USB mic · Always open, 300 ms pre-roll');
    expect(element('#active-model').textContent).toBe('ft:my-voice');
  });

  it('prefers the microphone actually opened over the configured device', async () => {
    snapshot.state.actual_input = 'Fallback microphone';
    snapshot.config.input_device = 'Unplugged microphone';
    await start();
    expect(document.body.textContent).toContain('Fallback microphone');
    expect(document.body.textContent).not.toContain('Unplugged microphone');
  });

  it('opens on an active app recording and lets it be stopped', async () => {
    snapshot.manual_recording = true;
    await start();
    expect(element('[data-action="manual-record"]').textContent).toBe('Stop and transcribe');
    await page('Models'); await page('Dictation');
    expect(element('[data-action="manual-record"]').textContent).toBe('Stop and transcribe');
    await action('manual-record');
    expect(operations).toContainEqual(['manual_stop', {}]);
  });

  it('does not overlap slow refresh requests', async () => {
    await start();
    let resolve!: (value: Obj) => void;
    bridge.invoke.mockImplementationOnce(() => new Promise(done => { resolve = done; }));
    await vi.advanceTimersByTimeAsync(700);
    const requests = bridge.invoke.mock.calls.length;
    await vi.advanceTimersByTimeAsync(1400);
    expect(bridge.invoke).toHaveBeenCalledTimes(requests);
    resolve(structuredClone(snapshot)); await settle();
    await vi.advanceTimersByTimeAsync(700);
    expect(bridge.invoke).toHaveBeenCalledTimes(requests + 1);
  });

  it('recovers the overlay after a connection error without creating settings UI', async () => {
    bridge.invoke.mockRejectedValueOnce('Service stopped');
    await start('?overlay');
    expect(document.querySelector('nav')).toBeNull();
    await vi.advanceTimersByTimeAsync(80);
    expect(element('.overlay strong').textContent).toBe('Ready');
  });

  it('reports service connection and periodic refresh failures', async () => {
    bridge.invoke.mockRejectedValueOnce('service stopped');
    await start();
    expect(element('#message').textContent).toContain('Cannot connect to the Rust service: service stopped');
    await vi.advanceTimersByTimeAsync(700);
    expect(element('#readiness').textContent).toBe('Ready');
    expect(element('#message').textContent).toBe('');
    expect(element('#message').classList.contains('visible')).toBe(false);
    await action('manual-record');
    expect(operations).toContainEqual(['manual_start', {}]);
    await action('manual-record');
    navigate({ payload: 'History' }); await settle();
    expect(element('#heading').textContent).toBe('History');
    expect(element('#history-list').textContent).toContain('Transcript 2');
    bridge.invoke.mockRejectedValueOnce('connection lost');
    await vi.advanceTimersByTimeAsync(700);
    expect(element('#message').textContent).toBe('connection lost');
  });

  it('shows a current page error after reconnecting instead of keeping the connection error', async () => {
    bridge.invoke.mockRejectedValueOnce('service stopped');
    await start(); await page('Settings');
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((command, args) => args.op === 'devices'
      ? Promise.reject('Microphone enumeration failed') : original(command, args));
    await vi.advanceTimersByTimeAsync(700);
    expect(element('#settings')).toBeDefined();
    expect(element('#message').textContent).toBe('Microphone enumeration failed');
    expect(element('#message').classList.contains('visible')).toBe(true);
  });

  it.each(['Recording', 'Ready'])('renders the overlay in state %s without settings controls', async stage => {
    snapshot.state.stage = stage;
    snapshot.state.level = 0.7;
    await start('?overlay');
    expect(document.body.classList.contains('overlay-body')).toBe(true);
    expect(element('.overlay strong').textContent).toBe(stage);
    expect(element<HTMLElement>('.meter i').style.width).toBe('100%');
    expect(element('.overlay .dot').classList.contains('recording')).toBe(stage === 'Recording');
    expect(document.querySelector('nav')).toBeNull();
    snapshot.state.level = 0;
    await vi.advanceTimersByTimeAsync(80);
    expect(element<HTMLElement>('.meter i').style.width).toBe('0%');
  });
});

describe('model selection', () => {
  it('combines language and downloaded filters, preserving keyboard focus', async () => {
    await start(); await page('Models');
    expect(document.querySelectorAll('.model-row')).toHaveLength(7);
    element<HTMLElement>('#model-language').focus();
    await change('#model-language', 'english');
    expect([...document.querySelectorAll('.model-row h2')].map(e => e.textContent)).toEqual(['english', 'optional']);
    expect(document.activeElement?.id).toBe('model-language');
    await change('#downloaded', true);
    expect(document.querySelectorAll('.model-row')).toHaveLength(0);
    expect(element('.empty').textContent).toContain('No models match');
    await change('#model-language', 'multilingual');
    expect([...document.querySelectorAll('.model-row h2')].map(e => e.textContent)).toEqual(['parakeet-tdt-0.6b-v3']);
    await change('#downloaded', false);
    expect(document.querySelectorAll('.model-row')).toHaveLength(2);
    await change('#model-language', 'other');
    expect(element('.model-row h2').textContent).toBe('french');
    await change('#model-language', '');
    expect(document.querySelectorAll('.model-row')).toHaveLength(7);
  });

  it('installs, selects and reuses models through the service', async () => {
    await start(); await page('Models');
    expect(document.body.textContent).toContain('Punctuation is unreliable');
    expect(document.body.textContent).toContain('Upstream terms required');
    await click('[data-action="install"][data-id="english"]');
    expect(operations).toContainEqual(['install', { id: 'english' }]);
    await click('[data-action="use-model"][data-id="french"]');
    expect(operations).toContainEqual(['settings', { ...defaults, model: 'french' }]);
    expect(element('#active-model').textContent).toBe('french');
    await click('[data-action="reuse"][data-id="english"]');
    expect(operations).toContainEqual(['reuse_model', { id: 'english', path: '/existing/model.gguf' }]);
    vi.mocked(prompt).mockReturnValueOnce(null);
    const reused = operations.filter(([op]) => op === 'reuse_model').length;
    await click('[data-action="reuse"][data-id="english"]');
    expect(operations.filter(([op]) => op === 'reuse_model')).toHaveLength(reused);
  });

  it('shows download progress and errors, and refreshes installed actions when a download finishes', async () => {
    snapshot.state.download = { id: 'english', stage: 'Downloading', bytes: 1_000_000, total: 2_000_000 };
    await start(); await page('Models');
    expect(element('#download-progress').textContent).toContain('1.0 / 2.0 MB');
    snapshot.state.download = { id: 'english', stage: 'Failed', error: '<offline>' };
    await vi.advanceTimersByTimeAsync(700);
    expect(element('#download-progress .error').textContent).toBe('<offline>');
    snapshot.models[1].installed = true;
    snapshot.state.download = { id: 'english', stage: 'Installed' };
    await vi.advanceTimersByTimeAsync(700);
    expect(document.querySelector('[data-action="use-model"][data-id="english"]')).not.toBeNull();
    await vi.advanceTimersByTimeAsync(700);
    expect(document.querySelectorAll('.model-row')).toHaveLength(7);
  });
});

describe('recording history', () => {
  it('searches and filters transcripts, including an empty result', async () => {
    await start(); await page('History');
    expect(element<HTMLTextAreaElement>('#correction').value).toBe('Transcript 2');
    await change('#search', 'correction', 'input');
    expect(document.querySelectorAll('.sample')).toHaveLength(1);
    expect(element<HTMLTextAreaElement>('#correction').value).toBe('A correction');
    expect(document.body.textContent).toContain('Audio retention is off');
    expect(element('[data-action="exclude"]').textContent).toBe('Include in training');
    await change('#search', '', 'input');
    await change('#filter', 'unedited');
    expect(element<HTMLTextAreaElement>('#correction').value).toBe('Transcript 2');
    await change('#filter', 'excluded');
    expect(element<HTMLTextAreaElement>('#correction').value).toBe('A correction');
    await change('#search', 'no match', 'input');
    expect(document.querySelectorAll('.sample')).toHaveLength(0);
    expect(element('#history-detail').textContent).toContain('Select a recording');
    expect(operations).toContainEqual(['history', { search: 'no match', filter: 'excluded' }]);
  });

  it('saves with Shift+Enter and requests replacement with Ctrl+Enter', async () => {
    await start(); await page('History');
    await change('#correction', 'Corrected via keyboard', 'input');
    element('#correction').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await settle();
    expect(operations.some(([op]) => op === 'correct')).toBe(false);
    element('#correction').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', shiftKey: true, bubbles: true }));
    await settle();
    expect(operations).toContainEqual(['correct', { id: 2, text: 'Corrected via keyboard', replace: false }]);
    await change('#correction', 'Replacement', 'input');
    element('#correction').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true, bubbles: true }));
    await settle();
    expect(operations).toContainEqual(['correct', { id: 2, text: 'Replacement', replace: true }]);
    await action('copy');
    expect(operations).toContainEqual(['copy', { text: 'Replacement' }]);
    await action('revert');
    expect(element<HTMLTextAreaElement>('#correction').value).toBe('Transcript 2');
    await action('exclude');
    expect(element('[data-action="exclude"]').textContent).toBe('Include in training');
    await action('exclude');
    expect(element('[data-action="exclude"]').textContent).toBe('Exclude from training');
  });

  it('keeps unsaved corrections when navigation or selection is declined', async () => {
    await start(); await page('History');
    await change('#correction', 'Unsaved', 'input');
    vi.mocked(confirm).mockReturnValueOnce(false);
    await click('[data-sample="1"]');
    expect(element<HTMLTextAreaElement>('#correction').value).toBe('Unsaved');
    vi.mocked(confirm).mockReturnValueOnce(false);
    await page('Models');
    expect(element('#heading').textContent).toBe('History');
    navigate({ payload: 'Settings' }); await settle();
    expect(element('#heading').textContent).toBe('History');
    await click('[data-sample="1"]');
    expect(element<HTMLTextAreaElement>('#correction').value).toBe('A correction');
    await page('Models');
    expect(element('#heading').textContent).toBe('Models');
    navigate({ payload: 'History' }); await settle();
    expect(element('#heading').textContent).toBe('History');
  });

  it('requires confirmation to delete and selects the next remaining recording', async () => {
    await start(); await page('History');
    vi.mocked(confirm).mockReturnValueOnce(false);
    await action('delete');
    expect(operations.some(([op]) => op === 'delete')).toBe(false);
    await action('delete');
    expect(operations).toContainEqual(['delete', { id: 2 }]);
    expect(element<HTMLTextAreaElement>('#correction').value).toBe('A correction');
    await action('delete');
    expect(element('#history-list').textContent).toContain('No recordings match');
    await action('export');
    expect(element('#message').textContent).toBe('Exported labelled audio to /candidate/exports/dataset');
  });

  it('shows teacher acceptance, confidence and rejection reasons', async () => {
    recordings = [sample(1, { teacher_text: '<teacher>', teacher_confidence: null, teacher_note: null })];
    await start(); await page('History');
    expect(document.body.textContent).toContain('0.00 confidence');
    expect(document.body.textContent).toContain('Accepted subject to configured threshold');
    expect(document.body.textContent).toContain('<teacher>');
    recordings[0].teacher_confidence = 0.4;
    recordings[0].teacher_note = 'Too few words';
    await change('#filter', 'unedited');
    expect(document.body.textContent).toContain('0.40 confidence');
    expect(document.body.textContent).toContain('Too few words');
  });

  it('plays returned WAV bytes and releases the previous audio URL on replay', async () => {
    const play = vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue(undefined);
    const create = vi.fn().mockReturnValueOnce('blob:first').mockReturnValueOnce('blob:second');
    const revoke = vi.fn();
    vi.stubGlobal('URL', class extends URL {
      static createObjectURL = create;
      static revokeObjectURL = revoke;
    });
    await start(); await page('History'); await action('play');
    expect(operations).toContainEqual(['audio', { id: 2 }]);
    expect(create.mock.calls[0][0]).toBeInstanceOf(Blob);
    expect(create.mock.calls[0][0].type).toBe('audio/wav');
    expect(element<HTMLAudioElement>('#playback').hidden).toBe(false);
    expect(play).toHaveBeenCalledTimes(1);
    await action('play');
    expect(revoke).toHaveBeenCalledWith('blob:first');
    expect(element<HTMLAudioElement>('#playback').src).toBe('blob:second');
  });

  it('shows a failed save and keeps the unsaved correction', async () => {
    await start(); await page('History');
    await change('#correction', 'Keep this edit', 'input');
    bridge.invoke.mockRejectedValueOnce('Disk full');
    await action('save');
    expect(element('#message').textContent).toBe('Disk full');
    expect(element<HTMLTextAreaElement>('#correction').value).toBe('Keep this edit');
    vi.mocked(confirm).mockReturnValueOnce(false);
    await page('Settings');
    expect(element('#heading').textContent).toBe('History');
  });
});

describe('learning', () => {
  it('explains the empty state and runs training, rollback and environment setup', async () => {
    await start(); await page('Learning');
    expect(document.body.textContent).toContain('No rules yet');
    expect(document.body.textContent).toContain('Your corrected words will appear here');
    expect(document.body.textContent).toContain('No trained versions');
    await action('train'); await action('rollback'); await action('install'); await action('cancel');
    for (const op of ['train', 'rollback', 'cancel']) expect(operations).toContainEqual([op, {}]);
    expect(operations).toContainEqual(['install', { id: 'training' }]);
  });

  it('shows learned rules, vocabulary and both gate reports, and can forget individual items', async () => {
    learning = {
      teacher: { completed: 3 }, training: { stage: 'Evaluating' },
      rules: [{ wrong: 'get hub', right: 'GitHub', count: 2 }],
      vocab: [{ term: 'GitHub' }],
      versions: [{ name: 'Your model', gate1: { passed: true }, gate2: { passed: false } }],
    };
    await start(); await page('Learning');
    expect(document.body.textContent).toContain('get hub → GitHub');
    expect(document.body.textContent).toContain('2 corrections');
    expect(document.body.textContent).toContain('Evaluating');
    expect(element('pre').textContent).toContain('"passed": false');
    await action('forget-rule'); await action('forget-vocab');
    expect(operations).toContainEqual(['forget', { kind: 'rule', key: 'get hub' }]);
    expect(operations).toContainEqual(['forget', { kind: 'vocab', key: 'GitHub' }]);
  });
});

describe('settings', () => {
  it('loads microphones and applies settings with their original value types', async () => {
    await start(); await page('Settings');
    expect(element<HTMLOptionElement>('#inputs option').value).toBe('USB "Microphone"');
    await click('legend');
    await change('[name="keep_mic_open"]', true, 'input');
    await change('[name="beam_size"]', '3', 'input');
    await change('[name="language"]', 'fr', 'input');
    await change('[name="theme"]', 'dark', 'input');
    await click('button[type="submit"]');
    expect(operations).toContainEqual(['settings', {
      ...defaults, keep_mic_open: true, beam_size: 3, language: 'fr', theme: 'dark',
    }]);
    expect(element('#message').textContent).toBe('Settings applied.');
    expect(document.documentElement.dataset.theme).toBe('dark');
    navigate({ payload: 'Models' }); await settle();
    expect(element('#heading').textContent).toBe('Models');
  });

  it('exports and imports TOML, and copies existing data only when a path is supplied', async () => {
    await start(); await page('Settings');
    await action('export-config');
    expect(element<HTMLTextAreaElement>('#config-transfer').value).toBe('language = "fr"');
    await action('import-config');
    expect(operations).toContainEqual(['import_config', { text: 'language = "fr"' }]);
    expect(element<HTMLInputElement>('[name="language"]').value).toBe('fr');
    vi.mocked(prompt).mockReturnValueOnce(null);
    await action('import-data');
    expect(operations.some(([op]) => op === 'import')).toBe(false);
    vi.mocked(prompt).mockReturnValueOnce('/old-data');
    await action('import-data');
    expect(operations).toContainEqual(['import', { path: '/old-data' }]);
    expect(element('#message').textContent).toContain('Imported 3 audio files. Source unchanged.');
    await action('autostart-on');
    expect(operations).toContainEqual(['autostart', { enabled: true }]);
    expect(element('#message').textContent).toBe('Candidate login entry enabled.');
    await action('autostart-off');
    expect(operations).toContainEqual(['autostart', { enabled: false }]);
    expect(element('#message').textContent).toBe('Candidate login entry removed.');
  });

  it('reports microphone enumeration and save errors without losing the form', async () => {
    await start(); await page('Settings');
    bridge.invoke.mockRejectedValueOnce('Audio device unavailable');
    await action('refresh-inputs');
    expect(element('#message').textContent).toBe('Audio device unavailable');
    await change('[name="language"]', 'bad-value', 'input');
    bridge.invoke.mockRejectedValueOnce('Invalid language');
    element('#settings').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();
    expect(element('#message').textContent).toBe('Invalid language');
    expect(element<HTMLInputElement>('[name="language"]').value).toBe('bad-value');
  });

  it('ignores microphone results arriving after the settings page has closed', async () => {
    await start();
    let resolve!: (value: string[]) => void;
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((command, args) => args.op === 'devices'
      ? new Promise(done => { resolve = done; }) : original(command, args));
    await page('Settings');
    await page('Models');
    resolve(['Late microphone']); await settle();
    expect(element('#heading').textContent).toBe('Models');
    expect(document.body.textContent).not.toContain('Late microphone');
  });

  it.each(['Learning', 'History'])('keeps Settings usable when a delayed %s response arrives', async name => {
    await start();
    let resolve!: (value: unknown) => void;
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((command, args) => args.op === name.toLowerCase()
      ? new Promise(done => { resolve = done; }) : original(command, args));
    await page(name);
    await page('Settings');
    await change('[name="language"]', 'fr', 'input');
    resolve(name === 'Learning' ? structuredClone(learning) : structuredClone(recordings));
    await settle();
    expect(element('#heading').textContent).toBe('Settings');
    expect(element<HTMLInputElement>('[name="language"]').value).toBe('fr');
    expect(document.querySelector('[data-action="train"]')).toBeNull();
    expect(document.querySelector('#correction')).toBeNull();
    expect(element('#message').textContent).toBe('');
  });

  it.each(['Learning', 'History'])('reports a failed current %s request', async name => {
    await start();
    bridge.invoke.mockRejectedValueOnce(`${name} unavailable`);
    await page(name);
    expect(element('#heading').textContent).toBe(name);
    expect(element('#message').textContent).toBe(`${name} unavailable`);
    expect(element('#message').classList.contains('visible')).toBe(true);
  });

  it.each(['Learning', 'History'])('ignores a failed %s request after navigating to Settings', async name => {
    await start();
    let reject!: (reason: string) => void;
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((command, args) => args.op === name.toLowerCase()
      ? new Promise((_done, fail) => { reject = fail; }) : original(command, args));
    await page(name);
    await page('Settings');
    await change('[name="language"]', 'fr', 'input');
    reject(`${name} unavailable`); await settle();
    expect(element('#heading').textContent).toBe('Settings');
    expect(element<HTMLInputElement>('[name="language"]').value).toBe('fr');
    expect(element('#message').textContent).toBe('');
    expect(element('#message').classList.contains('visible')).toBe(false);
  });

  it.each(['success', 'failure'])('keeps the newest history search when the older request completes with %s', async outcome => {
    await start(); await page('History');
    const replies: { resolve: (value: Obj[]) => void, reject: (reason: string) => void }[] = [];
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((command, args) => args.op === 'history'
      ? new Promise((resolve, reject) => { replies.push({ resolve, reject }); }) : original(command, args));
    await change('#search', 'old', 'input');
    await change('#search', 'new', 'input');
    replies[1].resolve([sample(3, { pasted: 'new result' })]); await settle();
    if (outcome === 'success') replies[0].resolve([sample(4, { pasted: 'old result' })]);
    else replies[0].reject('Old search unavailable');
    await settle();
    expect(element('#history-list').textContent).toContain('new result');
    expect(element('#history-list').textContent).not.toContain('old result');
    expect(element<HTMLTextAreaElement>('#correction').value).toBe('new result');
    expect(element('#message').textContent).toBe('');
  });

  it('keeps Settings usable when a history deletion finishes after navigation', async () => {
    await start(); await page('History');
    let resolve!: (value: boolean) => void;
    const original = bridge.invoke.getMockImplementation()!;
    bridge.invoke.mockImplementation((command, args) => args.op === 'delete'
      ? new Promise(done => { resolve = done; }) : original(command, args));
    await action('delete');
    await page('Settings');
    await change('[name="language"]', 'fr', 'input');
    resolve(true); await settle();
    expect(element('#heading').textContent).toBe('Settings');
    expect(element<HTMLInputElement>('[name="language"]').value).toBe('fr');
    expect(element('#message').textContent).toBe('');
  });

  it('uses the system color preference when the theme is system', async () => {
    vi.mocked(matchMedia).mockReturnValue({ matches: true } as MediaQueryList);
    await start();
    expect(document.documentElement.dataset.theme).toBe('dark');
    snapshot.config.theme = 'light';
    await vi.advanceTimersByTimeAsync(700);
    expect(document.documentElement.dataset.theme).toBe('light');
  });
});
