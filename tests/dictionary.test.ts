import { beforeEach, describe, expect, it } from "vitest";
import { createDictionaryPage } from "../src/pages/dictionary";
import { byLabel, byText, makeDeps, settle, toasts } from "./fakes";

let ctx: ReturnType<typeof makeDeps>;

async function setup() {
  const page = createDictionaryPage(ctx.deps);
  document.body.append(page.el);
  await page.show();
  return page;
}

const submit = () =>
  document.querySelector("form")!.dispatchEvent(new Event("submit", { cancelable: true }));

beforeEach(() => {
  ctx = makeDeps();
  ctx.api.dictionary.mockResolvedValue([
    { id: 1, heard: "", term: "Kubernetes", source: "manual", hits: 0 },
    { id: 2, heard: "jason", term: "JSON", source: "edit", hits: 3 },
    { id: 3, heard: "", term: "Ollama", source: "teacher", hits: 1 },
    { id: 4, heard: "", term: "Odd", source: "import", hits: 0 },
  ]);
});

describe("dictionary page", () => {
  it("lists words with where they came from", async () => {
    await setup();
    const items = [...document.querySelectorAll(".item")].map((el) => el.textContent);
    expect(items[0]).toContain("Added by you");
    expect(items[0]).not.toContain("used");
    expect(items[1]).toContain("jason → JSON");
    expect(items[1]).toContain("Learned from your edits · used 3 times");
    expect(items[2]).toContain("Learned overnight · used 1 time");
    expect(items[3]).toContain("import");
    expect(document.body.textContent).toContain("stops Speechify from learning it again");
    expect(document.body.textContent).not.toContain("Suggested");
  });

  it("adds a plain word and a correction", async () => {
    await setup();
    const word = byLabel<HTMLInputElement>(document, "Word or name");
    submit();
    expect(ctx.api.addWord).not.toHaveBeenCalled();
    word.value = " Tauri ";
    submit();
    await settle();
    expect(ctx.api.addWord).toHaveBeenCalledWith("", "Tauri");
    expect(word.value).toBe("");

    byText<HTMLButtonElement>(document, "button", "Correct a misspelling").click();
    expect(byText(document, "button", "Correct a misspelling").getAttribute("aria-pressed")).toBe(
      "true",
    );
    const heard = byLabel<HTMLInputElement>(document, "Speechify hears");
    expect(heard.parentElement!.hidden).toBe(false);
    heard.value = "tory";
    byLabel<HTMLInputElement>(document, "Write it as").value = "Tauri";
    submit();
    await settle();
    expect(ctx.api.addWord).toHaveBeenLastCalledWith("tory", "Tauri");

    byText<HTMLButtonElement>(document, "button", "Add a word").click();
    expect(heard.parentElement!.hidden).toBe(true);
  });

  it("shows the reason a word was rejected", async () => {
    await setup();
    ctx.api.addWord.mockRejectedValueOnce("That word is already in your dictionary.");
    byLabel<HTMLInputElement>(document, "Word or name").value = "Kubernetes";
    submit();
    await settle();
    const error = document.querySelector<HTMLElement>(".field-error")!;
    expect(error.hidden).toBe(false);
    expect(error.textContent).toBe("That word is already in your dictionary.");
  });

  it("deletes words and handles suggestions", async () => {
    ctx.api.suggestions.mockResolvedValue([
      { id: 8, heard: "sequel", term: "SQL", count: 2 },
      { id: 9, heard: "jif", term: "GIF", count: 1 },
    ]);
    await setup();
    expect(document.body.textContent).toContain("You corrected this 2 times");
    byText<HTMLButtonElement>(document, "button", "Accept").click();
    await settle();
    expect(ctx.api.resolveSuggestion).toHaveBeenCalledWith(8, true);
    [...document.querySelectorAll<HTMLButtonElement>("button")]
      .filter((b) => b.textContent === "Dismiss")[1]!
      .click();
    await settle();
    expect(ctx.api.resolveSuggestion).toHaveBeenLastCalledWith(9, false);

    byLabel<HTMLButtonElement>(document, "Delete JSON").click();
    await settle();
    expect(ctx.api.removeWord).toHaveBeenCalledWith(2);
    ctx.api.removeWord.mockRejectedValueOnce("Could not delete it.");
    byLabel<HTMLButtonElement>(document, "Delete JSON").click();
    await settle();
    expect(toasts()).toContain("Could not delete it.");
  });

  it("has an empty state", async () => {
    ctx.api.dictionary.mockResolvedValue([]);
    await setup();
    expect(document.querySelector(".empty-title")!.textContent).toBe("No words yet");
  });
});
