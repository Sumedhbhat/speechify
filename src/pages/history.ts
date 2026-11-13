// Dictation history on the home page: search, day groups, row actions, edit.

import type { Dictation } from "../api";
import type { Deps } from "../deps";
import {
  categoryLabel,
  dayKey,
  dayLabel,
  formatNumber,
  OUTCOME_LABELS,
  timeOfDay,
  todayKey,
} from "../format";
import { confirmButton, debounce, emptyState, errorText, h, iconButton, toast } from "../ui";

const PAGE = 50;

export function createHistory({ api, events }: Deps) {
  let items: Dictation[] = [];
  let query = "";
  let request = 0;
  let editing: number | null = null;
  // The correction being typed survives redraws (details, paging, deletes).
  let draft = "";
  // Each edit gets a number, so a save that finishes after its edit was
  // cancelled or replaced does not close the edit that is open now.
  let session = 0;
  const flagging = new Set<number>();
  let stale = false;
  let audio: HTMLAudioElement | null = null;
  const expanded = new Set<number>();
  const rows = new Map<number, HTMLElement>();

  const list = h("div", { className: "history-list" });
  const more = h("button", {
    type: "button",
    className: "btn more",
    textContent: "Load more",
    hidden: true,
    onclick: () => void loadOrSay(false),
  });
  const searchInput = h("input", {
    type: "search",
    className: "search",
    placeholder: "Search what you said",
    "aria-label": "Search history",
    oninput: () => search(searchInput.value.trim()),
  });
  const search = debounce((value: string) => {
    query = value;
    void loadOrSay(true);
  }, 250);
  const clear = confirmButton("Clear history", "Click again to delete everything", () =>
    api.deleteDictations(null).then(
      () => loadOrSay(true),
      (error: unknown) => toast(errorText(error), "error"),
    ),
  );

  const el = h("section", { className: "section history", "aria-labelledby": "history-title" }, [
    h("div", { className: "section-head" }, [
      h("h2", { id: "history-title", textContent: "History" }),
      h("div", { className: "section-tools" }, [searchInput, clear]),
    ]),
    list,
    more,
  ]);

  async function load(reset: boolean): Promise<void> {
    const token = ++request;
    const offset = reset ? 0 : items.length;
    const page = await api.history(query, PAGE, offset);
    if (token !== request) return;
    items = reset ? page : [...items, ...page];
    more.hidden = page.length < PAGE;
    // An edit whose dictation is no longer listed (a new search) ends.
    if (editing !== null && !items.some((item) => item.id === editing)) {
      editing = null;
      session += 1;
    }
    render();
  }

  /** Loads, keeping the current results and saying so if that fails. */
  function loadOrSay(reset: boolean): Promise<void> {
    return load(reset).catch((error: unknown) => toast(errorText(error), "error"));
  }

  function render(): void {
    rows.clear();
    if (items.length === 0) {
      list.replaceChildren(
        query
          ? emptyState("Nothing matches", `No dictation contains “${query}”.`)
          : emptyState(
              "Nothing here yet",
              "Everything you dictate shows up here, so you can copy it again, play it back or correct it.",
            ),
      );
      return;
    }
    const today = todayKey();
    const groups: HTMLElement[] = [];
    let current = "";
    let ul = h("ul", { className: "entries" });
    for (const item of items) {
      const key = dayKey(item.created_at, item.offset_min);
      if (key !== current) {
        current = key;
        ul = h("ul", { className: "entries" });
        groups.push(
          h("div", { className: "day" }, [h("h3", { textContent: dayLabel(key, today) }), ul]),
        );
      }
      const li = row(item);
      rows.set(item.id, li);
      ul.append(li);
    }
    list.replaceChildren(...groups);
  }

  /** Redraws one row and keeps focus on the same action. */
  function redraw(item: Dictation): void {
    const old = rows.get(item.id)!;
    const focused = document.activeElement as HTMLElement;
    const action = old.contains(focused) ? focused.dataset.action : undefined;
    const li = row(item);
    old.replaceWith(li);
    rows.set(item.id, li);
    if (action) li.querySelector<HTMLElement>(`[data-action="${action}"]`)!.focus();
  }

  function row(item: Dictation): HTMLElement {
    const open = expanded.has(item.id);
    const actions = h("div", { className: "entry-actions" }, [
      iconButton("copy", "Copy", () => void copy(item), { "data-action": "copy" }),
      item.audio &&
        iconButton("play", "Play recording", () => void play(item), { "data-action": "play" }),
      iconButton("flag", item.flagged ? "Unflag" : "Flag for review", () => void flag(item), {
        "data-action": "flag",
        "aria-pressed": String(item.flagged),
      }),
      iconButton("edit", "Edit", () => startEdit(item), { "data-action": "edit" }),
      iconButton("trash", "Delete", () => void remove(item), { "data-action": "delete" }),
      iconButton("chevron", open ? "Hide details" : "Show details", () => toggle(item), {
        "data-action": "details",
        "aria-expanded": String(open),
        className: "icon-btn chevron",
      }),
    ]);
    return h("li", { className: `entry${item.flagged ? " flagged" : ""}${open ? " open" : ""}` }, [
      h("div", { className: "entry-meta" }, [
        h("time", { textContent: timeOfDay(item.created_at, item.offset_min) }),
        h("span", { className: "entry-app", textContent: item.app || "Unknown app" }),
        h("span", { className: "chip", textContent: categoryLabel(item.category) }),
        item.command && h("span", { className: "chip accent", textContent: "Command" }),
        item.flagged && h("span", { className: "chip warn", textContent: "Flagged" }),
      ]),
      editing === item.id
        ? editor(item)
        : h("p", { className: "entry-text", textContent: item.text }),
      actions,
      open && detail(item),
    ]);
  }

  function detail(item: Dictation): HTMLElement {
    const pair = (label: string, value: string) => [
      h("dt", { textContent: label }),
      h("dd", { textContent: value }),
    ];
    const ms = (value: number) => `${formatNumber(value)} ms`;
    return h("dl", { className: "detail" }, [
      ...pair("Heard", item.raw),
      ...pair("Typed", item.text),
      ...(item.teacher_text === null ? [] : pair("Teacher's version", item.teacher_text)),
      ...pair(
        "Timing",
        `Recognition ${ms(item.asr_ms)} · Clean-up ${ms(item.rewrite_ms)} · Total ${ms(item.total_ms)}`,
      ),
      ...pair(
        "Models",
        item.rewritten
          ? `${item.speech_model} + ${item.rewrite_model}`
          : `${item.speech_model} (not rewritten)`,
      ),
      ...pair("Result", OUTCOME_LABELS[item.outcome]),
    ]);
  }

  function editor(item: Dictation): HTMLElement {
    const area = h("textarea", {
      className: "entry-edit",
      value: draft,
      rows: 3,
      "aria-label": "Corrected text",
      oninput: () => (draft = area.value),
    });
    const save = async () => {
      const text = area.value.trim();
      const mine = session;
      try {
        const learned = await api.editDictation(item.id, text);
        if (learned.length)
          toast(`Learned: ${learned.map((r) => `${r.from} → ${r.to}`).join(", ")}`);
        else toast("Correction saved");
        Object.assign(item, { text, edited: text });
        if (mine === session) stopEdit(item);
        else if (rows.has(item.id) && editing !== item.id) redraw(item);
      } catch (error) {
        toast(errorText(error), "error");
      }
    };
    queueMicrotask(() => area.focus());
    return h("div", { className: "entry-editor" }, [
      area,
      h("div", { className: "row" }, [
        h("button", {
          type: "button",
          className: "btn primary",
          textContent: "Save",
          "data-action": "edit",
          onclick: () => void save(),
        }),
        h("button", {
          type: "button",
          className: "btn",
          textContent: "Cancel",
          "data-action": "edit",
          onclick: () => stopEdit(item),
        }),
      ]),
    ]);
  }

  function startEdit(item: Dictation): void {
    const previous = items.find((other) => other.id === editing);
    editing = item.id;
    draft = item.text;
    session += 1;
    if (previous && previous !== item) redraw(previous);
    redraw(item);
  }

  function stopEdit(item: Dictation): void {
    editing = null;
    session += 1;
    if (stale) {
      stale = false;
      void load(true);
    } else {
      redraw(item);
    }
  }

  function toggle(item: Dictation): void {
    if (!expanded.delete(item.id)) expanded.add(item.id);
    redraw(item);
  }

  async function copy(item: Dictation): Promise<void> {
    try {
      await api.copyText(item.text);
      toast("Copied");
    } catch (error) {
      toast(errorText(error), "error");
    }
  }

  async function play(item: Dictation): Promise<void> {
    try {
      const url = await api.dictationAudio(item.id);
      if (!url) {
        toast("This recording is no longer available.");
        return;
      }
      audio?.pause();
      audio = new Audio(url);
      await audio.play();
    } catch (error) {
      toast(errorText(error), "error");
    }
  }

  async function flag(item: Dictation): Promise<void> {
    // One change at a time per dictation, and the result is what was asked for.
    if (flagging.has(item.id)) return;
    flagging.add(item.id);
    const wanted = !item.flagged;
    try {
      await api.flagDictation(item.id, wanted);
      item.flagged = wanted;
      if (rows.has(item.id)) redraw(item);
    } catch (error) {
      toast(errorText(error), "error");
    } finally {
      flagging.delete(item.id);
    }
  }

  async function remove(item: Dictation): Promise<void> {
    try {
      await api.deleteDictations([item.id]);
      items = items.filter((other) => other !== item);
      render();
    } catch (error) {
      toast(errorText(error), "error");
    }
  }

  function refresh(): Promise<void> {
    if (editing !== null) {
      stale = true;
      return Promise.resolve();
    }
    return load(true);
  }

  void events.historyChanged(
    () => void refresh().catch((error: unknown) => toast(errorText(error), "error")),
  );
  return { el, refresh };
}
