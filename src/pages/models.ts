// Models: speech models to download and pick, the device, the language model.

import type { Progress, RewriteStatus, Settings, SpeechModel, Status } from "../api";
import { number, select, text, toggle } from "../controls";
import type { Deps, Page } from "../deps";
import { formatBytes } from "../format";
import { confirmButton, errorText, h, icon, pageHeader, section, toast } from "../ui";

/** How often the open Models page asks the language-model server again. */
export const HEARTBEAT_MS = 10_000;

const DEVICES: [string, string][] = [
  ["auto", "Automatic"],
  ["cpu", "Processor (CPU)"],
  ["vulkan", "Graphics card (Vulkan)"],
  ["metal", "Apple GPU (Metal)"],
  ["cuda", "NVIDIA GPU (CUDA)"],
];

function meter(progress: Progress): { el: HTMLElement; update: (progress: Progress) => void } {
  const bar = h("progress", { max: 1 });
  const label = h("span", { className: "hint" });
  const update = (next: Progress) => {
    if (next.total > 0) {
      bar.max = next.total;
      bar.value = next.done;
      label.textContent = `${formatBytes(next.done)} of ${formatBytes(next.total)}`;
    } else {
      bar.removeAttribute("value");
      label.textContent = "Starting";
    }
  };
  update(progress);
  return { el: h("div", { className: "meter" }, [bar, label]), update };
}

/**
 * Whether Ollama already has `name`. Ollama lists a bare name as "name:latest"
 * and ignores case. Only a colon after the last "/" is a tag; one before it is
 * a registry port, as in "host:5000/team/model".
 */
export function ollamaHas(models: string[], name: string): boolean {
  if (!name.trim()) return false;
  const wanted = ollamaName(name);
  return models.some((listed) => ollamaName(listed) === wanted);
}

function ollamaName(name: string): string {
  const lower = name.trim().toLowerCase();
  const tagged = lower.slice(lower.lastIndexOf("/") + 1).includes(":");
  return tagged ? lower : `${lower}:latest`;
}

export function createModelsPage(deps: Deps): Page {
  const { api, events, settings: store } = deps;
  const downloads = new Map<string, Progress>();
  const meters = new Map<string, (progress: Progress) => void>();
  let models: SpeechModel[] = [];
  let pull: Progress | null = null;
  // The last server answer, the server it came from and the Model field, to
  // tell if the model typed there is already on the server.
  let reach: RewriteStatus = { reachable: false, models: [] };
  let server = "";
  let modelInput: HTMLInputElement | undefined;

  const cards = h("div", { className: "model-grid" });
  const device = h("div");
  const language = h("div");
  const pullMeter = h("div", { className: "pull" });
  const reachNote = h("p");
  const modelList = h("datalist", { id: "rewrite-models" });
  const refreshButton = h(
    "button",
    {
      type: "button",
      className: "btn",
      title: "Check the server now (it is also checked every 10 seconds)",
      onclick: () => void refreshReach(),
    },
    [icon("refresh"), "Refresh"],
  );
  // The button sits at the end of the row, so a status change never moves it.
  const reachRow = h("div", { className: "reach" }, [reachNote, refreshButton]);
  let shown = "";
  let latest = 0;
  let heartbeat: ReturnType<typeof setTimeout> | undefined;
  const pullButton = h(
    "button",
    {
      type: "button",
      className: "btn",
      onclick: () =>
        void guard(async () => {
          // A model name typed just before clicking may still be saving.
          const current = await store.settled();
          pull = { id: current.rewrite.model, done: 0, total: 0, finished: false, error: "" };
          renderPull();
          await api.pullRewriteModel(current.rewrite.model);
        }),
    },
    [icon("download"), "Download with Ollama"],
  );
  const pulled = h("p", { className: "status-note ok", hidden: true });

  const pullControls = h("div", {}, [
    h("div", { className: "row" }, [pullButton, pulled]),
    pullMeter,
  ]);
  const setup = h("details", { className: "advanced" }, [
    h("summary", { textContent: "Set up llama-server" }),
    h("p", { className: "hint" }, [
      "Install llama.cpp from ",
      h("code", { textContent: "github.com/ggml-org/llama.cpp/releases" }),
      ". Download Llama-3.2-3B-Instruct-Q4_K_M.gguf from bartowski/Llama-3.2-3B-Instruct-GGUF on Hugging Face. Put it beside the server executable and run the command from that folder.",
    ]),
    h("p", {}, [
      h("code", {
        textContent:
          "./llama-server -m Llama-3.2-3B-Instruct-Q4_K_M.gguf --alias llama3.2:3b -c 4096 --host 127.0.0.1 --port 18080 --log-disable",
      }),
    ]),
    h("p", {
      className: "hint",
      textContent:
        "On Windows, replace ./llama-server with .\\llama-server.exe in PowerShell. Keep the server open. Set Server to http://127.0.0.1:18080/v1 and Model to llama3.2:3b, click Refresh, then turn on language-model cleanup.",
    }),
  ]);

  function updatePullControls(url: string): void {
    pullControls.hidden = url.trim().replace(/\/+$/, "").endsWith("/v1");
  }

  async function guard(run: () => Promise<unknown>): Promise<void> {
    try {
      await run();
    } catch (error) {
      toast(errorText(error), "error");
    }
  }

  async function reloadModels(): Promise<void> {
    models = await api.models();
    renderCards();
  }

  /** Saves a model choice; the cards are redrawn once it is accepted. */
  async function choose(change: (settings: Settings) => void): Promise<void> {
    const error = await store.update(change);
    if (!error) await guard(reloadModels);
  }

  function card(model: SpeechModel): HTMLElement {
    const progress = downloads.get(model.id);
    const tags = h("p", { className: "tags" }, [
      h("span", { className: "chip", textContent: formatBytes(model.bytes) }),
      model.wants_gpu && h("span", { className: "chip", textContent: "Needs a graphics card" }),
      model.active && h("span", { className: "chip accent", textContent: "Used for dictation" }),
      model.teacher && h("span", { className: "chip", textContent: "Teacher" }),
    ]);
    let actions: HTMLElement;
    if (progress) {
      const m = meter(progress);
      meters.set(model.id, m.update);
      actions = h("div", { className: "model-actions" }, [
        m.el,
        h("button", {
          type: "button",
          className: "btn",
          textContent: "Cancel",
          onclick: () => void guard(() => api.cancelDownload(model.id)),
        }),
      ]);
    } else if (!model.installed) {
      actions = h("div", { className: "model-actions" }, [
        h(
          "button",
          { type: "button", className: "btn primary", onclick: () => void download(model) },
          [icon("download"), "Download"],
        ),
      ]);
    } else {
      actions = h("div", { className: "model-actions" }, [
        h("button", {
          type: "button",
          className: "btn primary",
          textContent: model.active ? "In use" : "Use for dictation",
          disabled: model.active,
          onclick: () => void choose((s) => (s.speech.model = model.id)),
        }),
        h("button", {
          type: "button",
          className: "btn",
          textContent: model.teacher ? "Teacher" : "Use as teacher",
          disabled: model.teacher,
          onclick: () => void choose((s) => (s.teacher.speech_model = model.id)),
        }),
        confirmButton(
          "Delete",
          "Delete it?",
          () => void guard(() => api.deleteModel(model.id).then(reloadModels)),
        ),
      ]);
    }
    return h("article", { className: `model${model.active ? " active" : ""}` }, [
      h("h3", { textContent: model.name }),
      tags,
      h("p", { className: "model-notes", textContent: model.notes }),
      actions,
    ]);
  }

  function renderCards(): void {
    meters.clear();
    cards.replaceChildren(...models.map(card));
  }

  async function download(model: SpeechModel): Promise<void> {
    downloads.set(model.id, {
      id: model.id,
      done: 0,
      total: model.bytes,
      finished: false,
      error: "",
    });
    renderCards();
    try {
      await api.downloadModel(model.id);
    } catch (error) {
      downloads.delete(model.id);
      renderCards();
      toast(errorText(error), "error");
    }
  }

  function renderDevice(settings: Settings, status: Status): void {
    device.replaceChildren(
      select(store, settings, {
        label: "Run speech on",
        hint: "Automatic picks your graphics card when it can and falls back to the processor.",
        options: DEVICES,
        get: (s) => s.speech.device,
        set: (s, v) => (s.speech.device = v),
      }),
      status.speech_error
        ? h("p", { className: "status-note error", textContent: status.speech_error })
        : h("p", {
            className: "status-note",
            textContent: status.speech_device
              ? `Running on ${status.speech_device}`
              : "The model loads when you first dictate.",
          }),
    );
  }

  /** Offers the download only while the server does not have the chosen model yet. */
  function renderPullButton(): void {
    const wanted = modelInput?.value.trim() ?? "";
    const have = reach.reachable && ollamaHas(reach.models, wanted);
    pullButton.hidden = have;
    pulled.hidden = !have;
    pulled.textContent = have ? `${wanted} is downloaded` : "";
  }

  function renderPull(): void {
    if (!pull) {
      pullMeter.replaceChildren();
      return;
    }
    pullMeter.replaceChildren(meter(pull).el);
  }

  /** Shows whether the server answered; the editable fields are left alone. */
  function renderReach(answer: RewriteStatus): void {
    // The heartbeat usually brings the same answer; leave the page untouched then.
    reach = answer;
    renderPullButton();
    const key = JSON.stringify(answer);
    if (key === shown) return;
    shown = key;
    const ok = answer.reachable;
    reachNote.className = `status-note${ok ? " ok" : " error"}`;
    reachNote.replaceChildren(
      h("span", { className: "status-dot", "data-state": ok ? "ready" : "error" }),
      ok
        ? "Connected"
        : "Can't reach the server. Start it and click Refresh. Dictation can use the built-in rules.",
    );
    modelList.replaceChildren(...answer.models.map((name) => h("option", { value: name })));
    pullButton.disabled = !ok;
  }

  /** Asks the server; only the newest answer is shown, so a slow one cannot undo a later one. */
  async function checkReach(): Promise<void> {
    const { rewrite } = await store.settled();
    const ticket = ++latest;
    const reach = await api.rewriteStatus();
    const accepted = await store.settled();
    if (ticket === latest && rewrite.url === accepted.rewrite.url) renderReach(reach);
  }

  /** Asks the server again, e.g. after starting Ollama with the page open. */
  async function refreshReach(): Promise<void> {
    const focused = document.activeElement === refreshButton;
    refreshButton.disabled = true;
    await guard(async () => {
      await checkReach();
    });
    refreshButton.disabled = false;
    // Disabling drops keyboard focus; give it back unless the user moved on.
    if (focused && document.activeElement === document.body) refreshButton.focus();
  }

  /** Checks the server every HEARTBEAT_MS while this page is open and the window visible. */
  function startHeartbeat(): void {
    clearTimeout(heartbeat);
    heartbeat = setTimeout(() => void beat(), HEARTBEAT_MS);
  }

  async function beat(): Promise<void> {
    // Leaving the page stops the heartbeat; showing it again restarts it.
    if (!el.isConnected) return;
    // A failed heartbeat stays quiet; the next one, or Refresh, tries again.
    if (!document.hidden) await checkReach().catch(() => undefined);
    // The next beat is timed from the end of this one, so a slow server is never asked twice at once.
    startHeartbeat();
  }

  function renderLanguage(settings: Settings, status: Status, current: boolean): void {
    const modelField = text(store, settings, {
      label: "Model",
      placeholder: "llama3.2:3b",
      list: modelList.id,
      get: (s) => s.rewrite.model,
      set: (s, v) => (s.rewrite.model = v),
    });
    // Follow the name while it is typed; the setting saves when the field is left.
    modelInput = modelField.querySelector("input")!;
    modelInput.addEventListener("input", renderPullButton);
    server = settings.rewrite.url;
    if (current)
      renderReach({ reachable: status.rewrite_reachable, models: status.rewrite_models });
    updatePullControls(settings.rewrite.url);
    language.replaceChildren(
      toggle(store, settings, {
        label: "Tidy text with a language model",
        hint: "A local model tidies longer dictations. Without it, or if it is slow or unreachable, the built-in rules are used.",
        get: (s) => s.rewrite.enabled,
        set: (s, v) => (s.rewrite.enabled = v),
      }),
      text(store, settings, {
        label: "Server",
        hint: "Use the llama-server address below, another OpenAI-compatible address ending in /v1, or the address of an Ollama server.",
        placeholder: "http://127.0.0.1:18080/v1",
        type: "url",
        get: (s) => s.rewrite.url,
        set: (s, v) => (s.rewrite.url = v),
      }),
      reachRow,
      modelField,
      modelList,
      pullControls,
      setup,
      number(store, settings, {
        label: "Give up after",
        hint: "If the model is slower than this, the text from the built-in rules is typed instead.",
        min: 500,
        max: 60000,
        step: 500,
        unit: "ms",
        get: (s) => s.rewrite.timeout_ms,
        set: (s, v) => (s.rewrite.timeout_ms = v),
      }),
      number(store, settings, {
        label: "Skip short dictations",
        hint: "Dictations with fewer words than this skip the model and use only the built-in rules.",
        min: 0,
        max: 100,
        unit: "words",
        get: (s) => s.rewrite.min_words,
        set: (s, v) => (s.rewrite.min_words = v),
      }),
      h("details", { className: "advanced" }, [
        h("summary", { textContent: "Remote speech server" }),
        h("p", {
          className: "hint",
          textContent:
            "Send audio to an OpenAI-compatible transcription server instead of recognising it here. Leave empty to stay local.",
        }),
        text(store, settings, {
          label: "Address",
          placeholder: "http://192.168.1.20:8080/v1",
          type: "url",
          get: (s) => s.speech.remote_url,
          set: (s, v) => (s.speech.remote_url = v),
        }),
        text(store, settings, {
          label: "Key",
          type: "password",
          get: (s) => s.speech.remote_key,
          set: (s, v) => (s.speech.remote_key = v),
        }),
        text(store, settings, {
          label: "Model name",
          get: (s) => s.speech.remote_model,
          set: (s, v) => (s.speech.remote_model = v),
        }),
      ]),
    );
    renderPullButton();
    renderPull();
  }

  void events.modelProgress((progress) => {
    if (progress.finished) {
      downloads.delete(progress.id);
      if (progress.error) toast(progress.error, "error");
      void guard(reloadModels);
      return;
    }
    const known = downloads.has(progress.id);
    downloads.set(progress.id, progress);
    if (known) meters.get(progress.id)?.(progress);
    else renderCards();
  });

  void events.rewriteProgress((progress) => {
    pull = progress.finished ? null : progress;
    renderPull();
    if (progress.error) toast(progress.error, "error");
    else if (progress.finished) toast(`${progress.id} is ready`);
    // Hide the button straight away rather than at the next heartbeat.
    if (progress.finished) void checkReach().catch(() => undefined);
  });

  store.subscribe((next) => {
    updatePullControls(next.rewrite.url);
    if (next.rewrite.url !== server) {
      server = next.rewrite.url;
      // The old server's list says nothing about the new one: offer the
      // download until the new server answers.
      reach = { reachable: false, models: [] };
      if (el.isConnected) void checkReach().catch(() => undefined);
    }
    renderPullButton();
  });

  const el = h("div", { className: "page models" }, [
    pageHeader("Models", "Everything runs on this computer. Pick what fits your hardware."),
    section(
      "Speech models",
      "Bigger models are more accurate but slower. Without a graphics card, pick a small one.",
      [cards, device],
    ),
    section("Language model", "", [language]),
  ]);

  return {
    el,
    async show() {
      // Opening the page asks the server too, so a check still in flight is now older.
      const ticket = ++latest;
      const openingSettings = store.load();
      const [list, status, opening] = await Promise.all([
        api.models(),
        api.getStatus(),
        openingSettings,
      ]);
      const settings = await store.settled();
      const currentServer = opening.rewrite.url === settings.rewrite.url;
      models = list;
      renderCards();
      renderDevice(settings, status);
      renderLanguage(settings, status, ticket === latest && currentServer);
      if (ticket === latest && !currentServer) await guard(checkReach);
      startHeartbeat();
    },
  };
}
