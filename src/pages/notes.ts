// Notes: a scratchpad to dictate into. Saves itself as you type.

import type { Note } from "../api";
import type { Deps, Page } from "../deps";
import { dateTime } from "../format";
import { confirmButton, debounce, emptyState, errorText, h, iconButton, toast } from "../ui";

const SAVE_MS = 600;

export function createNotesPage({ api }: Deps): Page {
  let notes: Note[] = [];
  let current: Note | null = null;
  let saving: Promise<unknown> = Promise.resolve();
  // Edits the shell refused to save, kept until a later save succeeds so a
  // refresh never replaces them with the older stored copy.
  const unsaved = new Map<number, Note>();

  const list = h("ul", { className: "note-list", "aria-label": "Notes" });
  const title = h("input", {
    type: "text",
    className: "note-title",
    placeholder: "Untitled",
    "aria-label": "Title",
    oninput: () => changed(),
  });
  const body = h("textarea", {
    className: "note-body",
    placeholder: "Click here and dictate.",
    "aria-label": "Note",
    oninput: () => changed(),
  });
  const state = h("p", { className: "hint note-state", "aria-live": "polite" });
  const remove = confirmButton("Delete", "Delete this note?", () => void removeCurrent());
  const editor = h("div", { className: "note-editor" }, [
    h("div", { className: "row note-bar" }, [state, remove]),
    title,
    body,
  ]);
  const pane = h("div", { className: "note-pane" });

  function persist(note: Note): void {
    saving = saving.then(async () => {
      try {
        await api.saveNote(note.id, note.title, note.body);
        unsaved.delete(note.id);
        if (note === current) state.textContent = "Saved";
        renderList();
      } catch (error) {
        unsaved.set(note.id, note);
        if (note === current) state.textContent = "Not saved yet: kept here and tried again";
        toast(errorText(error), "error");
      }
    });
  }

  const save = debounce((note: Note) => {
    note.title = title.value;
    note.body = body.value;
    persist(note);
  }, SAVE_MS);

  function changed(): void {
    state.textContent = "Editing";
    save(current!);
  }

  function renderList(): void {
    list.replaceChildren(
      ...notes.map((note) =>
        h("li", {}, [
          h(
            "button",
            {
              type: "button",
              className: "note-link",
              "aria-current": note === current ? "true" : undefined,
              onclick: () => select(note),
            },
            [
              h("span", {
                className: "note-link-title",
                textContent: note.title.trim() || "Untitled",
              }),
              h("span", { className: "hint", textContent: note.body.slice(0, 80) || "Empty" }),
            ],
          ),
        ]),
      ),
    );
  }

  function select(note: Note | null): void {
    save.flush();
    current = note;
    renderList();
    if (!note) {
      pane.replaceChildren(
        emptyState(
          "No notes yet",
          "A place to think out loud. Start one and dictate straight into it.",
          [newButton()],
        ),
      );
      return;
    }
    title.value = note.title;
    body.value = note.body;
    state.textContent = note.updated_at ? `Edited ${dateTime(note.updated_at)}` : "New note";
    pane.replaceChildren(editor);
  }

  async function create(): Promise<void> {
    save.flush();
    try {
      await saving;
      const id = await api.saveNote(null, "", "");
      const note: Note = { id, title: "", body: "", updated_at: 0 };
      notes = [note, ...notes];
      select(note);
      body.focus();
    } catch (error) {
      toast(errorText(error), "error");
    }
  }

  async function removeCurrent(): Promise<void> {
    const note = current!;
    save.flush();
    try {
      await saving;
      await api.deleteNote(note.id);
      notes = notes.filter((other) => other !== note);
      select(notes[0] ?? null);
    } catch (error) {
      toast(errorText(error), "error");
    }
  }

  function newButton(): HTMLButtonElement {
    return h("button", {
      type: "button",
      className: "btn primary",
      textContent: "New note",
      onclick: () => void create(),
    });
  }

  const el = h("div", { className: "page notes" }, [
    h("div", { className: "notes-layout" }, [
      h("aside", { className: "note-side" }, [
        h("div", { className: "row note-side-head" }, [
          h("h1", { textContent: "Notes" }),
          iconButton("plus", "New note", () => void create()),
        ]),
        list,
      ]),
      pane,
    ]),
  ]);

  return {
    el,
    async show() {
      save.flush();
      await saving;
      notes = (await api.notes()).map((note) => unsaved.get(note.id) ?? note);
      select(notes.find((note) => note.id === current?.id) ?? notes[0] ?? null);
      // Try the refused edits again now that the user is back.
      [...unsaved.values()].forEach(persist);
    },
  };
}
