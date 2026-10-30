import { button, escapeHtml } from '../ui';
import type { Snapshot, Learning } from '../types';

export function renderLearning(snapshot: Snapshot, data: Learning): string {
  return /* HTML */ `<p>
      Corrections become vocabulary hints immediately. Repeated word-level fixes become automatic
      rules after ${snapshot.config.autocorrect_after} corrections.
    </p>
    <section>
      <h2>Background learning</h2>
      <div id="download-progress" role="status"></div>
      <dl>
        <dt>Teacher</dt>
        <dd>
          ${escapeHtml(typeof data.teacher === 'string' ? data.teacher : JSON.stringify(data.teacher))}
        </dd>
        <dt>Training</dt>
        <dd>
          ${escapeHtml(typeof data.training === 'string' ? data.training : JSON.stringify(data.training))}
        </dd>
      </dl>
      <p class="quiet">
        Runs while idle and on mains. Human corrections take precedence over confident teacher
        labels. Uncorrected dictation guesses are never training labels.
      </p>
      <div class="actions">
        ${button('install', 'Install training environment', 'data-id="training"')}
        ${button('train', 'Run fine-tuning')} ${button('cancel', 'Pause current job')}
        ${button('rollback', 'Roll back version')}
      </div>
      <p class="quiet">
        Training uses the optional Python/CUDA environment. Adoption requires improved held-out
        personal WER and the LibriSpeech regression gate.
      </p>
    </section>
    <section>
      <h2>Word rules</h2>
      ${
        data.rules.length
          ? data.rules
              .map(
                (r) =>
                  /* HTML */ `<div class="learning-row">
                    <span
                      >${escapeHtml(r.wrong)} → <strong>${escapeHtml(r.right)}</strong
                      ><small>${r.count} corrections</small></span
                    >${button('forget-rule', 'Forget', `data-key="${escapeHtml(r.wrong)}"`)}
                  </div>`,
              )
              .join('')
          : '<p class="empty">No rules yet. Correct a recording in History to start learning.</p>'
      }
    </section>
    <section>
      <h2>Vocabulary</h2>
      ${data.vocab.length ? data.vocab.map((v) => /* HTML */ `<div class="learning-row"><span>${escapeHtml(v.term)}</span>${button('forget-vocab', 'Forget', `data-key="${escapeHtml(v.term)}"`)}</div>`).join('') : '<p class="empty">Your corrected words will appear here.</p>'}
    </section>
    <section>
      <h2>Trained versions</h2>
      ${
        data.versions.length
          ? data.versions
              .map(
                (v) =>
                  /* HTML */ `<details>
                    <summary>${escapeHtml(v.name)}</summary>
                    <pre>
${escapeHtml(JSON.stringify({ personal: v.gate1, regression: v.gate2 }, null, 2))}</pre>
                  </details>`,
              )
              .join('')
          : '<p class="empty">No trained versions. The original model remains active.</p>'
      }
    </section>`;
}
