import { escapeHtml, pages } from './ui';
import type { Page, Snapshot } from './types';

export function renderShell(app: HTMLDivElement, page: Page) {
  app.innerHTML = /* HTML */ `<aside>
      <div class="brand">
        <svg viewBox="0 0 30 30" aria-hidden="true">
          <path d="M4 12v6m6-12v18m6-21v24m6-18v12m6-9v6" /></svg
        ><span>Speechify<small>Tauri candidate</small></span>
      </div>
      <nav aria-label="Main">
        ${pages.map((p) => /* HTML */ `<button data-page="${p}" aria-current="${p === page ? 'page' : 'false'}">${p}</button>`).join('')}
      </nav>
      <div class="sidebar-status">
        <span class="dot"></span><span id="readiness">Connecting to service</span
        ><small id="active-model"></small>
      </div>
    </aside>
    <main>
      <header>
        <h1 id="heading">${page}</h1>
        <span class="quiet" id="device"></span>
      </header>
      <div id="message" role="status" aria-live="polite"></div>
      <div id="view"></div>
    </main>`;
}
function theme(snapshot: Snapshot) {
  const t = snapshot.config.theme;
  document.documentElement.dataset.theme =
    t === 'system' ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light') : t;
}
export function updateStatus(app: HTMLDivElement, current: Snapshot, overlay: boolean) {
  theme(current);
  const state = current.state;
  const manualRecording = current.manual_recording === true;
  const record = document.querySelector('[data-action="manual-record"]');
  if (record) record.textContent = manualRecording ? 'Stop and transcribe' : 'Record in app';
  if (overlay) {
    app.innerHTML = /* HTML */ `<div class="overlay">
      <span class="dot ${state.stage === 'Recording' ? 'recording' : ''}"></span
      ><strong>${escapeHtml(state.stage)}</strong>
      <div class="meter"><i style="width:${Math.min(100, state.level * 400)}%"></i></div>
    </div>`;
    return;
  }
  document.querySelector('#readiness')!.textContent = state.stage;
  document.querySelector('#active-model')!.textContent =
    current.models.find((m) => m.id === current.config.model)?.name ?? current.config.model;
  document.querySelector('#device')!.textContent = state.device;
  const stage = document.querySelector('#dictation-stage');
  if (stage) stage.textContent = state.stage;
  const recent = document.querySelector('#recent');
  if (recent) recent.textContent = state.recent || 'Your next dictation will appear here.';
  const wave = document.querySelectorAll<HTMLElement>('.wave span');
  wave.forEach((bar, i) => {
    bar.style.height = `${4 + Math.min(90, state.level * 500) * (0.45 + 0.55 * Math.sin(i * 0.7) ** 2)}px`;
  });
  const progress = document.querySelector('#download-progress');
  if (progress && state.download) {
    const p = state.download;
    progress.innerHTML = /* HTML */ `<strong>${escapeHtml(p.id)}</strong> — ${escapeHtml(p.stage)}
      ${p.bytes ? `${(p.bytes / 1e6).toFixed(1)} / ${(p.total! / 1e6).toFixed(1)} MB` : ''}${p.error ? /* HTML */ `<p class="error">${escapeHtml(p.error)}</p>` : ''}`;
  }
  const err = document.querySelector('#service-error');
  if (err) {
    err.textContent = state.error ?? '';
    err.classList.toggle('visible', !!state.error);
  }
}
