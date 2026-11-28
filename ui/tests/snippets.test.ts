import { beforeEach, describe, expect, it } from "vitest";
import { createSnippetsPage } from "../src/pages/snippets";
import { byLabel, byText, makeDeps, settle, toasts } from "./fakes";

let ctx: ReturnType<typeof makeDeps>;

async function setup() {
  const page = createSnippetsPage(ctx.deps);
  document.body.append(page.el);
  await page.show();
}

const submit = () =>
  document.querySelector("form")!.dispatchEvent(new Event("submit", { cancelable: true }));

beforeEach(() => {
  ctx = makeDeps();
  ctx.api.snippets.mockResolvedValue([
    { id: 1, trigger: "my address", expansion: "42 Lake View Road\nBengaluru", hits: 2 },
    { id: 2, trigger: "sign off", expansion: "Thanks,\nA", hits: 0 },
  ]);
});

describe("snippets page", () => {
  it("lists snippets with how often they were used", async () => {
    await setup();
    const items = [...document.querySelectorAll(".item")].map((el) => el.textContent);
    expect(items[0]).toContain("“my address”");
    expect(items[0]).toContain("Used 2 times");
    expect(items[1]).toContain("Not used yet");
    expect(document.querySelector(".snippet-form h2")!.textContent).toBe("New snippet");
    expect(byText(document, "button", "Cancel").hidden).toBe(true);
  });

  it("creates a snippet", async () => {
    await setup();
    byLabel<HTMLInputElement>(document, "When I say").value = " my phone ";
    byLabel<HTMLTextAreaElement>(document, "Type this").value = "98450\n00000";
    submit();
    await settle();
    expect(ctx.api.saveSnippet).toHaveBeenCalledWith(null, "my phone", "98450\n00000");
    expect(byLabel<HTMLInputElement>(document, "When I say").value).toBe("");
  });

  it("edits a snippet, shows validation errors and cancels", async () => {
    await setup();
    byLabel<HTMLButtonElement>(document, "Edit my address").click();
    expect(document.querySelector(".snippet-form h2")!.textContent).toBe("Edit snippet");
    expect(document.activeElement).toBe(byLabel(document, "When I say"));
    ctx.api.saveSnippet.mockRejectedValueOnce("A snippet with that phrase already exists.");
    submit();
    await settle();
    expect(ctx.api.saveSnippet).toHaveBeenCalledWith(
      1,
      "my address",
      "42 Lake View Road\nBengaluru",
    );
    const error = document.querySelector<HTMLElement>(".field-error")!;
    expect(error.textContent).toBe("A snippet with that phrase already exists.");
    expect(error.hidden).toBe(false);
    byText<HTMLButtonElement>(document, "button", "Cancel").click();
    expect(error.hidden).toBe(true);
    expect(byText(document, "button", "Add snippet")).toBeTruthy();
  });

  it("deletes snippets, leaving the editor if it was open", async () => {
    await setup();
    byLabel<HTMLButtonElement>(document, "Edit my address").click();
    byLabel<HTMLButtonElement>(document, "Delete my address").click();
    await settle();
    expect(ctx.api.deleteSnippet).toHaveBeenCalledWith(1);
    expect(document.querySelector(".snippet-form h2")!.textContent).toBe("New snippet");

    byLabel<HTMLButtonElement>(document, "Edit sign off").click();
    byLabel<HTMLButtonElement>(document, "Delete my address").click();
    await settle();
    expect(document.querySelector(".snippet-form h2")!.textContent).toBe("Edit snippet");

    ctx.api.deleteSnippet.mockRejectedValueOnce("Could not delete it.");
    byLabel<HTMLButtonElement>(document, "Delete sign off").click();
    await settle();
    expect(toasts()).toContain("Could not delete it.");
  });

  it("has an empty state", async () => {
    ctx.api.snippets.mockResolvedValue([]);
    await setup();
    expect(document.querySelector(".empty-title")!.textContent).toBe("No snippets yet");
  });
});
