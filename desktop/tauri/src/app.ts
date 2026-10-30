import { listen } from '@tauri-apps/api/event';
import { api } from './api';
import { notify, escapeHtml } from './ui';
import { renderShell, updateStatus } from './shell';
import { renderDictation } from './pages/dictation';
import { renderModels } from './pages/models';
import { renderHistory, renderHistoryList, renderHistoryDetail } from './pages/history';
import { renderLearning } from './pages/learning';
import { renderSettings, readSettings } from './pages/settings';
import type { Page, Snapshot, Recording, Learning } from './types';

export function startApplication(app: HTMLDivElement, overlay: boolean): void {
  const error = notify;
  function status() {
    manualRecording = snapshot!.manual_recording === true;
    updateStatus(app, snapshot!, overlay);
  }
  let manualRecording = false;
  let page: Page = 'Dictation',
    snapshot: Snapshot | null = null,
    rows: Recording[] = [],
    selected: number | null = null,
    downloaded = false;
  let search = '',
    filter = '',
    dirty = false,
    refreshBusy = false;
  let initialized = false,
    renderVersion = 0,
    historyRequest = 0;
  let modelLanguage = '';
  async function render() {
    if (!snapshot || overlay) return;
    const view = document.querySelector('#view')!;
    const version = ++renderVersion;
    document.querySelector('#heading')!.textContent = page;
    document
      .querySelectorAll<HTMLElement>('[data-page]')
      .forEach((b) => b.setAttribute('aria-current', b.dataset.page === page ? 'page' : 'false'));
    if (page === 'Dictation') {
      view.innerHTML = renderDictation(snapshot, manualRecording);
    } else if (page === 'Models') {
      view.innerHTML = renderModels(snapshot, downloaded, modelLanguage);
    } else if (page === 'History') {
      view.innerHTML = renderHistory(search, filter);
      await history();
    } else if (page === 'Learning') {
      let data: Learning;
      try {
        data = await api<Learning>('learning');
      } catch (e) {
        if (version === renderVersion) throw e;
        return;
      }
      if (version !== renderVersion) return;
      view.innerHTML = renderLearning(snapshot, data);
    } else {
      view.innerHTML = renderSettings(snapshot);
      void inputs();
    }
    status();
  }
  async function history() {
    const version = renderVersion,
      request = ++historyRequest;
    const current = () =>
      version === renderVersion && request === historyRequest && page === 'History';
    let data: Recording[];
    try {
      data = await api<Recording[]>('history', { search, filter });
    } catch (e) {
      if (current()) throw e;
      return;
    }
    if (!current()) return;
    rows = data;
    if (!rows.some((r) => r.id === selected)) selected = rows[0]?.id ?? null;
    document.querySelector('#history-list')!.innerHTML = renderHistoryList(rows, selected);
    detail();
  }
  function detail() {
    const recording = rows.find((row) => row.id === selected);
    if (recording) dirty = false;
    document.querySelector('#history-detail')!.innerHTML = renderHistoryDetail(recording);
  }

  async function inputs() {
    try {
      const names = await api<string[]>('devices');
      const el = document.querySelector('#inputs');
      if (el)
        el.innerHTML = names
          .map((n) => /* HTML */ `<option value="${escapeHtml(n)}"></option>`)
          .join('');
    } catch (e) {
      error(e);
    }
  }
  app.addEventListener('submit', (e) => {
    e.preventDefault();
    const form = e.target as HTMLFormElement;
    const cfg = readSettings(form, snapshot!.config);
    void api('settings', cfg)
      .then(async () => {
        snapshot = await api<Snapshot>('snapshot');
        dirty = false;
        notify('Settings applied.');
        status();
      })
      .catch(error);
  });
  app.addEventListener('input', (e) => {
    const el = e.target as HTMLInputElement;
    if (el.id === 'search') {
      search = el.value;
      void history().catch(error);
    }
    if (el.id === 'correction' || el.closest('#settings')) dirty = true;
  });
  app.addEventListener('change', (e) => {
    const el = e.target as HTMLInputElement;
    if (el.id === 'filter') {
      filter = el.value;
      void history().catch(error);
    }
    if (el.id === 'model-language') {
      const focused = document.activeElement === el;
      modelLanguage = el.value;
      void render()
        .then(() => {
          if (focused) document.querySelector<HTMLSelectElement>('#model-language')?.focus();
        })
        .catch(error);
    }
    if (el.id === 'downloaded') {
      downloaded = el.checked;
      void render().catch(error);
    }
  });
  app.addEventListener('keydown', (e) => {
    if (
      (e.target as HTMLElement).id === 'correction' &&
      e.key === 'Enter' &&
      (e.shiftKey || e.ctrlKey)
    ) {
      e.preventDefault();
      void action(e.ctrlKey ? 'replace' : 'save', {} as HTMLElement).catch(error);
    }
  });
  app.addEventListener('click', (e) => {
    const b = (e.target as HTMLElement).closest<HTMLElement>('button');
    if (!b) return;
    if (b.dataset.page) {
      if (dirty && !confirm('Discard unsaved edits?')) return;
      dirty = false;
      page = b.dataset.page as Page;
      void render().catch(error);
    } else if (b.dataset.sample) {
      if (dirty && !confirm('Discard unsaved correction?')) return;
      selected = Number(b.dataset.sample);
      void history().catch(error);
    } else if (b.dataset.action) {
      void action(b.dataset.action, b).catch(error);
    }
  });
  async function action(actionName: string, control: HTMLElement) {
    const recording = rows.find((recording) => recording.id === selected);
    const id = selected;
    if (['load', 'cancel', 'train', 'rollback'].includes(actionName)) {
      await api(actionName);
    } else if (actionName === 'manual-record') {
      await api(manualRecording ? 'manual_stop' : 'manual_start');
      manualRecording = !manualRecording;
      control.textContent = manualRecording ? 'Stop and transcribe' : 'Record in app';
    } else if (actionName === 'install') await api('install', { id: control.dataset.id });
    else if (actionName === 'use-model') {
      await api('settings', { ...snapshot!.config, model: control.dataset.id });
    } else if (actionName === 'reuse') {
      const path = prompt(
        'Full path to the existing GGUF. It will be checksum-validated and copied; the source is never modified.',
      );
      if (path) await api('reuse_model', { id: control.dataset.id, path });
    } else if (actionName === 'copy-recent') await api('copy', { text: snapshot!.state.recent });
    else if (actionName === 'copy')
      await api('copy', {
        text: (document.querySelector('#correction') as HTMLTextAreaElement).value,
      });
    else if (actionName === 'save' || actionName === 'replace') {
      await api('correct', {
        id,
        text: (document.querySelector('#correction') as HTMLTextAreaElement).value,
        replace: actionName === 'replace',
      });
      dirty = false;
      await history();
    } else if (actionName === 'revert') {
      await api('revert', { id });
      await history();
    } else if (actionName === 'exclude') {
      await api('exclude', { id, excluded: !recording?.excluded });
      await history();
    } else if (
      actionName === 'delete' &&
      confirm('Delete this recording and its candidate-owned audio?')
    ) {
      await api('delete', { id });
      await history();
    } else if (actionName === 'play') {
      const data = await api<string>('audio', { id });
      const bytes = Uint8Array.from(atob(data), (c) => c.charCodeAt(0));
      const audio = document.querySelector<HTMLAudioElement>('#playback')!;
      if (audio.src.startsWith('blob:')) URL.revokeObjectURL(audio.src);
      audio.src = URL.createObjectURL(new Blob([bytes], { type: 'audio/wav' }));
      audio.hidden = false;
      await audio.play();
    } else if (actionName.startsWith('forget-')) {
      await api('forget', { kind: actionName.slice(7), key: control.dataset.key });
      await render();
    } else if (actionName === 'export') notify(`Exported labelled audio to ${await api('export')}`);
    else if (actionName === 'refresh-inputs') await inputs();
    else if (actionName === 'export-config')
      (document.querySelector('#config-transfer') as HTMLTextAreaElement).value =
        await api<string>('export_config');
    else if (actionName === 'import-config') {
      await api('import_config', {
        text: (document.querySelector('#config-transfer') as HTMLTextAreaElement).value,
      });
      snapshot = await api<Snapshot>('snapshot');
      await render();
    } else if (actionName === 'import-data') {
      const path = prompt(
        'Existing Speechify data directory. Copies a consistent SQLite backup and referenced audio into empty candidate history.',
      );
      if (path) notify(`Imported ${await api('import', { path })} audio files. Source unchanged.`);
    } else if (actionName.startsWith('autostart-')) {
      await api('autostart', { enabled: actionName === 'autostart-on' });
      notify(
        actionName === 'autostart-on'
          ? 'Candidate login entry enabled.'
          : 'Candidate login entry removed.',
      );
    }
    snapshot = await api<Snapshot>('snapshot');
    status();
    if (page === 'Models' && ['install', 'use-model', 'reuse'].includes(actionName)) await render();
  }
  async function refresh() {
    if (refreshBusy) return;
    refreshBusy = true;
    try {
      const old = snapshot?.state.download?.stage;
      snapshot = await api<Snapshot>('snapshot');
      status();
      if (!initialized) {
        const message = document.querySelector('#message');
        if (message) {
          message.textContent = '';
          message.classList.remove('visible');
        }
        await render();
        await listen<Page>('navigate', (e) => {
          if (dirty) return;
          page = e.payload;
          void render().catch(error);
        });
        initialized = true;
      }
      if (
        page === 'Models' &&
        old !== snapshot!.state.download?.stage &&
        snapshot!.state.download?.stage === 'Installed'
      )
        await render();
    } catch (e) {
      error(initialized ? e : `Cannot connect to the Rust service: ${e}`);
    } finally {
      refreshBusy = false;
    }
  }
  async function start() {
    if (overlay) {
      document.body.classList.add('overlay-body');
    } else renderShell(app, page);
    await refresh();
    setInterval(() => void refresh(), overlay ? 80 : 700);
  }
  void start();
}
