// Snippets: say a short phrase, get a longer text typed.

import type { Snippet } from "../api";
import type { Deps, Page } from "../deps";
import { plural } from "../format";
import { emptyState, errorText, h, iconButton, pageHeader, section, toast } from "../ui";

export function createSnippetsPage({ api }: Deps): Page {
  let editing: number | null = null;
  const trigger = h("input", {
    id: "snippet-trigger",
    type: "text",
    placeholder: "my address",
    autocomplete: "off",
  });
  const expansion = h("textarea", {
    id: "snippet-expansion",
    rows: 4,
    placeholder: "42 Lake View Road\nBengaluru 560001",
  });
  const error = h("p", { className: "field-error", role: "alert", hidden: true });
  const heading = h("h2");
  const submitButton = h("button", { type: "submit", className: "btn primary" });
  const cancel = h("button", {
    type: "button",
    className: "btn",
    textContent: "Cancel",
    onclick: () => edit(null),
  });
  const form = h(
    "form",
    { className: "snippet-form", onsubmit: (event: Event) => void submit(event) },
    [
      heading,
      h("div", { className: "stacked" }, [
        h("label", { htmlFor: trigger.id, textContent: "When I say" }),
        trigger,
      ]),
      h("div", { className: "stacked" }, [
        h("label", { htmlFor: expansion.id, textContent: "Type this" }),
        expansion,
      ]),
      error,
      h("div", { className: "row" }, [submitButton, cancel]),
    ],
  );
  const list = h("div");

  function edit(snippet: Snippet | null): void {
    editing = snippet?.id ?? null;
    trigger.value = snippet?.trigger ?? "";
    expansion.value = snippet?.expansion ?? "";
    heading.textContent = snippet ? "Edit snippet" : "New snippet";
    submitButton.textContent = snippet ? "Save changes" : "Add snippet";
    cancel.hidden = !snippet;
    error.hidden = true;
    if (snippet) trigger.focus();
  }

  async function submit(event: Event): Promise<void> {
    event.preventDefault();
    try {
      await api.saveSnippet(editing, trigger.value.trim(), expansion.value);
      edit(null);
      await refresh();
    } catch (failure) {
      error.textContent = errorText(failure);
      error.hidden = false;
    }
  }

  async function remove(snippet: Snippet): Promise<void> {
    try {
      await api.deleteSnippet(snippet.id);
      if (editing === snippet.id) edit(null);
      await refresh();
    } catch (failure) {
      toast(errorText(failure), "error");
    }
  }

  function row(snippet: Snippet): HTMLElement {
    return h("li", { className: "item" }, [
      h("div", { className: "item-main" }, [
        h("p", {}, [h("strong", { textContent: `“${snippet.trigger}”` })]),
        h("p", { className: "expansion", textContent: snippet.expansion }),
        h("p", {
          className: "hint",
          textContent: snippet.hits ? `Used ${plural(snippet.hits, "time")}` : "Not used yet",
        }),
      ]),
      h("div", { className: "row" }, [
        iconButton("edit", `Edit ${snippet.trigger}`, () => edit(snippet)),
        iconButton("trash", `Delete ${snippet.trigger}`, () => void remove(snippet)),
      ]),
    ]);
  }

  async function refresh(): Promise<void> {
    const snippets = await api.snippets();
    list.replaceChildren(
      snippets.length
        ? h("ul", { className: "items" }, snippets.map(row))
        : emptyState(
            "No snippets yet",
            "Say “my address” and Speechify types the whole thing. Add your first one here.",
          ),
    );
  }

  edit(null);
  const el = h("div", { className: "page snippets" }, [
    pageHeader("Snippets", "Short phrases that expand into text you type often."),
    h("div", { className: "columns wide-left" }, [section("Your snippets", "", [list]), form]),
  ]);
  return { el, show: refresh };
}
