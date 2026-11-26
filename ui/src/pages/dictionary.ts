// Dictionary: words to recognise, corrections, and suggested corrections.

import type { Suggestion, Word } from "../api";
import type { Deps, Page } from "../deps";
import { plural } from "../format";
import { emptyState, errorText, h, iconButton, nextId, pageHeader, section, toast } from "../ui";

const SOURCES: Record<string, string> = {
  manual: "Added by you",
  edit: "Learned from your edits",
  teacher: "Learned overnight",
};

function labelled(label: string, input: HTMLInputElement): HTMLElement {
  input.id = nextId("word");
  return h("div", { className: "stacked" }, [
    h("label", { htmlFor: input.id, textContent: label }),
    input,
  ]);
}

export function createDictionaryPage({ api }: Deps): Page {
  let correcting = false;
  const heard = h("input", {
    type: "text",
    placeholder: "jason",
    autocomplete: "off",
    spellcheck: false,
  });
  const term = h("input", {
    type: "text",
    placeholder: "JSON",
    autocomplete: "off",
    spellcheck: false,
  });
  const heardField = labelled("Speechify hears", heard);
  const termField = labelled("Word or name", term);
  const error = h("p", { className: "field-error", role: "alert", hidden: true });
  const modeWord = h("button", {
    type: "button",
    textContent: "Add a word",
    onclick: () => setMode(false),
  });
  const modeFix = h("button", {
    type: "button",
    textContent: "Correct a misspelling",
    onclick: () => setMode(true),
  });
  const form = h(
    "form",
    { className: "add-form", onsubmit: (event: Event) => void submit(event) },
    [
      h("div", { className: "segmented", role: "group", "aria-label": "What to add" }, [
        modeWord,
        modeFix,
      ]),
      h("div", { className: "row fields" }, [
        heardField,
        termField,
        h("button", { type: "submit", className: "btn primary", textContent: "Add" }),
      ]),
      error,
    ],
  );
  const suggested = h("div");
  const list = h("div");

  function setMode(fix: boolean): void {
    correcting = fix;
    modeWord.setAttribute("aria-pressed", String(!fix));
    modeFix.setAttribute("aria-pressed", String(fix));
    heardField.hidden = !fix;
    term.previousElementSibling!.textContent = fix ? "Write it as" : "Word or name";
  }
  setMode(false);

  async function submit(event: Event): Promise<void> {
    event.preventDefault();
    const value = term.value.trim();
    if (!value) return;
    try {
      await api.addWord(correcting ? heard.value.trim() : "", value);
      heard.value = "";
      term.value = "";
      error.hidden = true;
      await refresh();
    } catch (failure) {
      error.textContent = errorText(failure);
      error.hidden = false;
    }
  }

  async function act(run: () => Promise<void>): Promise<void> {
    try {
      await run();
      await refresh();
    } catch (failure) {
      toast(errorText(failure), "error");
    }
  }

  function suggestionRow(item: Suggestion): HTMLElement {
    return h("li", { className: "item" }, [
      h("div", { className: "item-main" }, [
        h("p", {}, [
          h("span", { className: "heard", textContent: item.heard }),
          " → ",
          h("strong", { textContent: item.term }),
        ]),
        h("p", {
          className: "hint",
          textContent: `You corrected this ${plural(item.count, "time")}`,
        }),
      ]),
      h("div", { className: "row" }, [
        h("button", {
          type: "button",
          className: "btn primary",
          textContent: "Accept",
          onclick: () => void act(() => api.resolveSuggestion(item.id, true)),
        }),
        h("button", {
          type: "button",
          className: "btn",
          textContent: "Dismiss",
          onclick: () => void act(() => api.resolveSuggestion(item.id, false)),
        }),
      ]),
    ]);
  }

  function wordRow(word: Word): HTMLElement {
    return h("li", { className: "item" }, [
      h("div", { className: "item-main" }, [
        word.heard
          ? h("p", {}, [
              h("span", { className: "heard", textContent: word.heard }),
              " → ",
              h("strong", { textContent: word.term }),
            ])
          : h("p", {}, [h("strong", { textContent: word.term })]),
        h("p", { className: "hint" }, [
          h("span", {
            className: `badge ${word.source}`,
            textContent: SOURCES[word.source] ?? word.source,
          }),
          word.hits > 0 && ` · used ${plural(word.hits, "time")}`,
        ]),
      ]),
      iconButton("trash", `Delete ${word.term}`, () => void act(() => api.removeWord(word.id))),
    ]);
  }

  async function refresh(): Promise<void> {
    const [words, suggestions] = await Promise.all([api.dictionary(), api.suggestions()]);
    suggested.replaceChildren(
      suggestions.length
        ? section(
            "Suggested",
            "Corrections you keep making. Accept one to apply it automatically.",
            [h("ul", { className: "items" }, suggestions.map(suggestionRow))],
          )
        : "",
    );
    list.replaceChildren(
      section(
        "Your words",
        "Deleting a learned word also stops Speechify from learning it again.",
        words.length
          ? [h("ul", { className: "items" }, words.map(wordRow))]
          : [
              emptyState(
                "No words yet",
                "Add names, places and jargon that Speechify should spell your way.",
              ),
            ],
      ),
    );
  }

  const el = h("div", { className: "page dictionary" }, [
    pageHeader("Dictionary", "Names, terms and spellings Speechify should get right."),
    form,
    suggested,
    list,
  ]);
  return { el, show: refresh };
}
