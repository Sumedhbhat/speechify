import { beforeEach, describe, expect, it, vi } from "vitest";
import { createNotesPage } from "../src/pages/notes";
import type { Page } from "../src/deps";
import { byLabel, byText, makeDeps, settle, toasts, type } from "./fakes";

let ctx: ReturnType<typeof makeDeps>;
let page: Page;

async function setup() {
  page = createNotesPage(ctx.deps);
  document.body.append(page.el);
  await page.show();
}

const links = () => [...document.querySelectorAll<HTMLButtonElement>(".note-link")];
const title = () => byLabel<HTMLInputElement>(document, "Title");
const body = () => byLabel<HTMLTextAreaElement>(document, "Note");

beforeEach(() => {
  vi.useFakeTimers();
  ctx = makeDeps();
  ctx.api.notes.mockResolvedValue([
    { id: 1, title: "Groceries", body: "milk, eggs", updated_at: 1_700_000_000 },
    { id: 2, title: " ", body: "", updated_at: 1_700_000_100 },
  ]);
});

describe("notes page", () => {
  it("lists notes and opens the first", async () => {
    await setup();
    expect(links().map((l) => l.querySelector(".note-link-title")!.textContent)).toEqual([
      "Groceries",
      "Untitled",
    ]);
    expect(links()[1]!.querySelector(".hint")!.textContent).toBe("Empty");
    expect(links()[0]!.getAttribute("aria-current")).toBe("true");
    expect(title().value).toBe("Groceries");
    expect(document.querySelector(".note-state")!.textContent).toMatch(/^Edited /);
    links()[1]!.click();
    expect(title().value).toBe(" ");
    expect(links()[1]!.getAttribute("aria-current")).toBe("true");
  });

  it("saves a moment after typing and when switching notes", async () => {
    await setup();
    type(title(), "Shopping");
    expect(document.querySelector(".note-state")!.textContent).toBe("Editing");
    vi.advanceTimersByTime(600);
    await settle();
    expect(ctx.api.saveNote).toHaveBeenCalledWith(1, "Shopping", "milk, eggs");
    expect(document.querySelector(".note-state")!.textContent).toBe("Saved");
    expect(links()[0]!.textContent).toContain("Shopping");

    type(body(), "bread");
    links()[1]!.click();
    await settle();
    expect(ctx.api.saveNote).toHaveBeenLastCalledWith(1, "Shopping", "bread");
  });

  it("says when a save fails", async () => {
    await setup();
    ctx.api.saveNote.mockRejectedValueOnce("Disk is full.");
    type(body(), "x");
    vi.advanceTimersByTime(600);
    await settle();
    expect(document.querySelector(".note-state")!.textContent).toBe(
      "Not saved yet: kept here and tried again",
    );
    expect(toasts()).toContain("Disk is full.");
  });

  it("keeps the open note's status when another note's retry fails", async () => {
    await setup();
    ctx.api.saveNote.mockRejectedValueOnce("Disk is full.");
    type(body(), "milk, eggs, bread");
    vi.advanceTimersByTime(600);
    await settle();
    links()[1]!.click();
    ctx.api.saveNote.mockRejectedValueOnce("Still full.");
    await page.show();
    await settle();
    expect(toasts()).toContain("Still full.");
    expect(document.querySelector(".note-state")!.textContent).not.toContain("Not saved");
  });

  it("keeps an edit that failed to save and saves it on the next visit", async () => {
    await setup();
    ctx.api.saveNote.mockRejectedValueOnce("Disk is full.");
    type(body(), "milk, eggs, bread");
    vi.advanceTimersByTime(600);
    await settle();
    // Coming back reloads the stored notes, which still have the old text.
    await page.show();
    await settle();
    expect(body().value).toBe("milk, eggs, bread");
    expect(ctx.api.saveNote).toHaveBeenLastCalledWith(1, "Groceries", "milk, eggs, bread");
    expect(document.querySelector(".note-state")!.textContent).toBe("Saved");
  });

  it("creates and deletes notes", async () => {
    await setup();
    ctx.api.saveNote.mockResolvedValueOnce(7);
    byLabel<HTMLButtonElement>(document, "New note").click();
    await settle();
    expect(ctx.api.saveNote).toHaveBeenCalledWith(null, "", "");
    expect(links()).toHaveLength(3);
    expect(document.querySelector(".note-state")!.textContent).toBe("New note");
    expect(document.activeElement).toBe(body());

    const remove = byText<HTMLButtonElement>(document, "button", "Delete");
    remove.click();
    remove.click();
    await settle();
    expect(ctx.api.deleteNote).toHaveBeenCalledWith(7);
    expect(links()).toHaveLength(2);
    expect(title().value).toBe("Groceries");

    ctx.api.deleteNote.mockRejectedValueOnce("Could not delete.");
    remove.click();
    remove.click();
    await settle();
    expect(toasts()).toContain("Could not delete.");

    ctx.api.saveNote.mockRejectedValueOnce("Could not create.");
    byLabel<HTMLButtonElement>(document, "New note").click();
    await settle();
    expect(toasts()).toContain("Could not create.");
  });

  it("starts empty and keeps the open note when shown again", async () => {
    ctx.api.notes.mockResolvedValueOnce([]);
    await setup();
    expect(document.querySelector(".empty-title")!.textContent).toBe("No notes yet");
    ctx.api.saveNote.mockResolvedValueOnce(2);
    byText<HTMLButtonElement>(document, ".empty button", "New note").click();
    await settle();
    await page.show();
    expect(links()[1]!.getAttribute("aria-current")).toBe("true");

    const remove = byText<HTMLButtonElement>(document, "button", "Delete");
    ctx.api.notes.mockResolvedValueOnce([{ id: 2, title: "", body: "", updated_at: 5 }]);
    await page.show();
    remove.click();
    remove.click();
    await settle();
    expect(document.querySelector(".empty-title")!.textContent).toBe("No notes yet");
  });
});
