declare const speechify: {
  call: (m: string, a?: any) => Promise<any>;
  onLevel: (f: (n: number) => void) => void;
  onState: (f: (s: any) => void) => void;
  onNavigate: (f: (p: string) => void) => void;
};
let data: any = null,
  page = "Dictation",
  selected: number | undefined,
  search = "",
  filter = "all",
  downloaded = false,
  editing = false;
const content = document.getElementById("content")!;
const esc = (s: any) =>
  String(s ?? "").replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ]!,
  );
async function action(m: string, a?: any) {
  try {
    const r = await speechify.call(m, a);
    if (r?.config) {
      data = r;
      render();
    }
    return r;
  } catch (e) {
    const el = document.getElementById("error")!;
    el.textContent = String(e);
    el.hidden = false;
  }
}
function button(text: string, method: string, arg?: any) {
  return `<button data-action="${method}" data-arg="${esc(JSON.stringify(arg ?? null))}">${text}</button>`;
}
function render() {
  if (!data) return;
  document.body.classList.toggle(
    "light",
    data.config.theme === "light" ||
      (data.config.theme === "system" &&
        matchMedia("(prefers-color-scheme:light)").matches),
  );
  document.getElementById("status")!.textContent = data.status;
  document.getElementById("runtime")!.textContent =
    data.device + " · " + data.config.model;
  document.getElementById("heading")!.textContent = page;
  const err = document.getElementById("error")!;
  err.textContent = data.error;
  err.hidden = !data.error;
  document
    .querySelectorAll("nav button")
    .forEach((b) =>
      b.setAttribute(
        "aria-current",
        b.getAttribute("data-page") === page ? "page" : "false",
      ),
    );
  if (editing) return;
  if (page === "Dictation") {
    content.innerHTML = `<section class="panel"><h2>Hold to talk</h2><p>Hold <kbd>${esc(data.config.ptt_key)}</kbd>, speak, then release. Short taps and other key combinations are ignored.</p><svg class="wave" viewBox="0 0 640 120" role="img" aria-label="Microphone input level">${Array.from({ length: 48 }, (_, i) => `<line x1="${i * 13 + 8}" x2="${i * 13 + 8}" y1="${60 - Math.max(2, data.level * 120 * Math.sin(i * 0.6) ** 2)}" y2="${60 + Math.max(2, data.level * 120 * Math.sin(i * 0.6) ** 2)}"/>`).join("")}</svg><p>${esc(data.devices.find((d: any) => d.id === data.config.input_device)?.label || "System default microphone")} · ${data.config.keep_mic_open ? "Always open, with pre-roll" : "Microphone closes when idle"}</p></section><div class="toolbar">${button("Start recording", "startRecording")}${button("Stop recording", "stopRecording")}</div><h2>Recent result</h2><div class="result">${esc(data.recent || "Your next dictation will appear here.")}</div>${data.recent ? button("Copy text", "copy", data.recent) : ""}<h2>Permissions</h2><p>${esc(data.permissions.guidance)}</p>${data.platform === "darwin" ? button("Request permissions", "permission") : ""}`;
  }
  if (page === "Models") {
    const rows = data.models.filter((m: any) => !downloaded || m.installed);
    content.innerHTML = `<label><input id="downloaded" type="checkbox" ${downloaded ? "checked" : ""}> Downloaded only</label><p>One dictation model stays loaded between recordings. Switching unloads the previous model. Shared files are read-only and verified against the pinned checksum.</p>${rows.map((m: any) => `<section class="row"><div><h2>${esc(m.name)}</h2><div class="meta">${esc(m.languages)} · ${m.gguf ? (m.gguf.size / 1e9).toFixed(2) : m.download_gb} GB download · ${m.vram_gb} GB memory · Accuracy ${m.accuracy}/5 · Speed ${m.speed}/5</div><p>${esc(m.notes)}</p><small>${esc(m.support)}${m.gated ? " · Upstream access terms apply" : ""}</small>${m.progress ? `<p>${esc(m.progress.stage)} ${m.progress.bytes ?? 0} / ${m.progress.total ?? 0} bytes</p><progress max="${m.progress.total || 1}" value="${m.progress.bytes || 0}"></progress><small>${esc(m.progress.error)}</small>` : ""}</div><div class="toolbar">${m.progress && ["downloading", "verifying"].includes(m.progress.stage) ? button("Cancel download", "cancelDownload", m.id) : m.installed ? button(data.config.model === m.id ? "Reload" : "Use model", "use", m.id) : m.gguf ? button("Install", "install", m.id) : button("Install optional model", "install", m.id) + button("Use offline", "use", m.id)}${m.gguf ? button("Use existing file…", "reference", m.id) : ""}</div></section>`).join("")}`;
    document.getElementById("downloaded")!.onchange = (e) => {
      downloaded = (e.target as HTMLInputElement).checked;
      render();
    };
  }
  if (page === "History") {
    content.innerHTML = `<div class="toolbar"><input id="search" aria-label="Search history" placeholder="Search recordings" value="${esc(search)}"><select id="filter" aria-label="History filter">${["all", "edited", "unedited", "excluded"].map((v) => `<option ${filter === v ? "selected" : ""}>${v}</option>`).join("")}</select>${button("Export labeled WAVs + CSV", "export")}</div><div class="split"><div class="history-list" id="list"></div><section id="detail"><p>Select a recording to listen or correct its text.</p></section></div>`;
    const refresh = async () => {
      const rows = await action("history", { search, filter });
      if (!Array.isArray(rows)) return;
      const list = document.getElementById("list");
      if (!list) return;
      list.innerHTML = rows.length
        ? rows
            .map(
              (r: any) =>
                `<button data-sample="${r.id}" class="${selected === r.id ? "selected" : ""}"><small>${new Date(r.ts * 1000).toLocaleString()} · ${r.excluded ? "excluded" : r.corrected ? "edited" : "as pasted"}</small>${esc((r.corrected ?? r.pasted).slice(0, 140))}</button>`,
            )
            .join("")
        : "<p>No recordings match. Completed dictations appear here.</p>";
      list.querySelectorAll("[data-sample]").forEach((b) =>
        b.addEventListener("click", () => {
          selected = Number(b.getAttribute("data-sample"));
          detail(rows.find((r: any) => r.id === selected));
        }),
      );
      if (selected) detail(rows.find((r: any) => r.id === selected));
    };
    document.getElementById("search")!.oninput = (e) => {
      search = (e.target as HTMLInputElement).value;
      void refresh();
    };
    document.getElementById("filter")!.onchange = (e) => {
      filter = (e.target as HTMLSelectElement).value;
      void refresh();
    };
    void refresh();
  }
  if (page === "Learning") {
    content.innerHTML = `<h2>Teacher labels</h2><p>${esc(data.teacher)}</p><p>Training uses your corrections first, then confident teacher labels. Uncorrected pasted text is never a training label.</p><h2>Fine-tuning</h2><p>${esc(data.training)}</p><div class="toolbar">${button("Run teacher now", "teacher")}${button("Run training", "train")}${button("Cancel learning", "cancelLearning")}${button("Roll back model", "rollback")}</div><h2>Versions and evaluation gates</h2><pre>${esc(data.versions.length ? JSON.stringify(data.versions, null, 2) : "No trained versions yet. Adoption requires better held-out personal WER and a passing LibriSpeech regression gate.")}</pre><h2>Learned rules</h2>${data.rules.length ? data.rules.map((r: any) => `<div class="row"><div>${esc(r.wrong)} → ${esc(r.right)}<small>${r.count} corrections · ${r.count >= data.config.autocorrect_after ? "active" : "learning"}</small></div>${button("Forget", "forget", r.wrong)}</div>`).join("") : "<p>Correct a recording to start learning word-level fixes.</p>"}<h2>Vocabulary hints</h2>${data.vocab.map((r: any) => `<div class="row"><span>${esc(r.term)} <small>${r.count} corrections</small></span>${button("Forget", "forget", r.term)}</div>`).join("") || "<p>No vocabulary learned yet.</p>"}`;
  }
  if (page === "Settings") {
    const groups: Record<string, string[]> = {
      General: ["theme", "startup"],
      Microphone: ["input_device", "keep_mic_open"],
      Shortcuts: [
        "ptt_key",
        "correct_hotkey",
        "history_hotkey",
        "models_hotkey",
        "min_hold_ms",
      ],
      Inference: [
        "device",
        "compute_type",
        "language",
        "beam_size",
        "onnx_threads",
      ],
      Learning: [
        "autocorrect_after",
        "learn_auto",
        "teacher",
        "idle_minutes",
        "teacher_on_cpu",
        "teacher_min_confidence",
        "retrain_after_labels",
        "retrain_after_days",
        "max_regression_wer",
        "keep_versions",
      ],
      "Optional Python workers": ["inference_python", "training_python"],
      "Privacy / Storage": ["keep_all_audio", "llm_url", "llm_model"],
    };
    content.innerHTML = `<p>Changes apply immediately. Microphone and shortcuts remain owned by the background service when this window closes.</p>${Object.entries(
      groups,
    )
      .map(
        ([g, keys]) =>
          `<section class="section"><h2>${g}</h2>${keys.map((k) => field(k)).join("")}</section>`,
      )
      .join(
        "",
      )}<div class="toolbar">${button("Export TOML", "exportConfig")}${button("Import TOML", "importConfig")}${button("Copy original data…", "importData")}${button("Open diagnostic logs", "logs")}</div><p>Candidate storage: ${esc(data.root)}</p><p>${esc(data.permissions.guidance)}</p><p>Clipboard restoration supports formats exposed by Electron. Private application formats may not round-trip. Focus changes prevent paste. macOS replacement requires the latest verified insertion and an unchanged accessible caret and text value; other targets support save-only.</p>`;
    content.querySelectorAll("[data-setting]").forEach((el) =>
      el.addEventListener("change", () => {
        const e = el as HTMLInputElement;
        const k = e.dataset.setting!;
        const v =
          typeof data.config[k] === "boolean"
            ? e.checked
            : typeof data.config[k] === "number"
              ? Number(e.value)
              : e.value;
        void action("settings", { [k]: v });
      }),
    );
  }
  content
    .querySelectorAll("[data-action]")
    .forEach((b) =>
      b.addEventListener(
        "click",
        () =>
          void action(
            b.getAttribute("data-action")!,
            JSON.parse(b.getAttribute("data-arg")!),
          ),
      ),
    );
}
function field(k: string) {
  const value = data.config[k];
  const label = k.replaceAll("_", " ");
  let control = "";
  const options: any = {
    theme: ["system", "dark", "light"],
    device: ["auto", "cuda", "vulkan", "metal", "cpu"],
    compute_type: ["float16", "int8", "float32"],
    teacher: ["off", "auto", ...data.models.map((m: any) => m.id)],
  };
  if (k === "input_device")
    control = `<select id="${k}" data-setting="${k}"><option value="">System default</option>${data.devices.map((d: any) => `<option value="${esc(d.id)}" ${value === d.id ? "selected" : ""}>${esc(d.label)}</option>`).join("")}</select>`;
  else if (options[k])
    control = `<select id="${k}" data-setting="${k}">${options[k].map((v: string) => `<option ${v === value ? "selected" : ""}>${esc(v)}</option>`).join("")}</select>`;
  else
    control = `<input id="${k}" data-setting="${k}" type="${typeof value === "boolean" ? "checkbox" : typeof value === "number" ? "number" : "text"}" ${typeof value === "boolean" ? (value ? "checked" : "") : `value="${esc(value)}"`} ${typeof value === "number" ? 'step="any"' : ""}>`;
  return `<div class="field"><label for="${k}">${label[0].toUpperCase() + label.slice(1)}</label>${control}</div>`;
}
function detail(row: any) {
  const el = document.getElementById("detail");
  if (!el || !row) return;
  el.innerHTML = `<h2>Correction</h2>${row.audio ? '<audio controls id="playback"></audio>' : "<p>Audio not retained.</p>"}<label for="correction">What you said</label><textarea id="correction">${esc(row.corrected ?? row.pasted)}</textarea><small>Originally heard: ${esc(row.raw)}</small><div class="toolbar"><button id="save" class="primary">Save only · Shift+Enter</button>${data.replaceableId === row.id ? '<button id="replace">Save and replace · Ctrl+Enter</button>' : ""}<button id="copy">Copy text</button><button id="revert">Revert</button><button id="exclude">${row.excluded ? "Include in training" : "Exclude from training"}</button><button id="delete">Delete recording</button></div><small>Save-and-replace closes History and checks the original target, caret and text before writing. If any check fails, the correction is saved only. Audio for the latest recording is recoverable for ten minutes when keep-all-audio is off.</small>`;
  const editor = document.getElementById("correction") as HTMLTextAreaElement;
  editor.onfocus = () => (editing = true);
  editor.onblur = () => (editing = false);
  const save = () => {
    editing = false;
    void action("correct", { id: row.id, text: editor.value });
  };
  const replace = () => {
    editing = false;
    void action("replace", { id: row.id, text: editor.value });
  };
  document.getElementById("replace")?.addEventListener("click", replace);
  editor.onkeydown = (e) => {
    if (e.key === "Enter" && e.ctrlKey && !e.shiftKey) {
      e.preventDefault();
      replace();
      return;
    }
    if (e.key === "Enter" && e.shiftKey) {
      e.preventDefault();
      save();
    }
  };
  document.getElementById("save")!.onclick = save;
  document.getElementById("copy")!.onclick = () =>
    void action("copy", editor.value);
  document.getElementById("revert")!.onclick = () =>
    void action("revert", row.id);
  document.getElementById("exclude")!.onclick = () =>
    void action("exclude", { id: row.id, value: !row.excluded });
  document.getElementById("delete")!.onclick = () => {
    if (confirm("Delete this candidate recording and its audio?")) {
      selected = undefined;
      void action("delete", row.id);
    }
  };
  if (row.audio)
    void action("audio", row.id).then((base64) => {
      const audio = document.getElementById("playback") as HTMLAudioElement;
      if (audio && typeof base64 === "string")
        audio.src = "data:audio/wav;base64," + base64;
    });
}
document.querySelectorAll("[data-page]").forEach((b) =>
  b.addEventListener("click", () => {
    editing = false;
    page = b.getAttribute("data-page")!;
    render();
  }),
);
document.getElementById("cancel")!.onclick = () => void action("cancel");
speechify.onNavigate((p) => {
  page = p;
  editing = false;
  render();
});
let lastRender = 0,
  lastStatic = "";
speechify.onState((s) => {
  const signature = JSON.stringify({ ...s, level: 0 });
  data = s;
  if (
    page === "Dictation"
      ? Date.now() - lastRender > 250
      : signature !== lastStatic
  ) {
    lastRender = Date.now();
    lastStatic = signature;
    render();
  }
});
speechify.onLevel((n) => {
  if (data) data.level = n;
  if (page !== "Dictation") return;
  document.querySelectorAll(".wave line").forEach((line, i) => {
    const h = Math.max(2, n * 120 * Math.sin(i * 0.6) ** 2);
    line.setAttribute("y1", String(60 - h));
    line.setAttribute("y2", String(60 + h));
  });
});
void action("state");
