import { beforeEach, describe, expect, it, vi } from "vitest";
import { createStylePage } from "../src/pages/style";
import { byLabel, byText, change, makeDeps, settle, toasts, type } from "./fakes";

let ctx: ReturnType<typeof makeDeps>;

async function setup() {
  const page = createStylePage(ctx.deps);
  document.body.append(page.el);
  await page.show();
  await settle();
}

const card = (title: string) => byText(document, "legend", title).parentElement!;

beforeEach(() => {
  ctx = makeDeps();
  ctx.api.rules.mockResolvedValue([{ pattern: "Linear", category: "work" }]);
});

describe("style page", () => {
  it("chooses a tone per kind of app with a live example", async () => {
    await setup();
    expect(document.querySelectorAll(".style-card")).toHaveLength(4);
    const work = card("Work messages");
    expect(work.querySelector(".example")!.textContent).toContain("Thank you.");
    const excited = work.querySelector<HTMLInputElement>("input[value=excited]")!;
    excited.checked = true;
    excited.dispatchEvent(new Event("change"));
    await settle();
    expect(work.querySelector(".example")!.textContent).toContain("Thanks so much!");
    expect(ctx.api.saveSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({ styles: expect.objectContaining({ work: "excited" }) }),
    );

    ctx.api.saveSettings.mockRejectedValueOnce("Styles are locked.");
    const casual = work.querySelector<HTMLInputElement>("input[value=very_casual]")!;
    casual.checked = true;
    casual.dispatchEvent(new Event("change"));
    await settle();
    expect(excited.checked).toBe(true);
    expect(work.querySelector(".example")!.textContent).toContain("Thanks so much!");
  });

  it("switches spelling, numbers and clean-up", async () => {
    await setup();
    change(byLabel<HTMLSelectElement>(document, "Spelling"), "american");
    change(byLabel<HTMLSelectElement>(document, "Large numbers"), "international");
    change(byLabel<HTMLSelectElement>(document, "Clean-up"), "medium");
    await settle();
    expect(ctx.api.saveSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({
        spelling: "american",
        numbers: "international",
        rewrite: expect.objectContaining({ level: "medium" }),
      }),
    );
  });

  it("edits app rules", async () => {
    await setup();
    expect(byLabel<HTMLInputElement>(document, "App name contains").value).toBe("Linear");
    byText<HTMLButtonElement>(document, "button", "Add rule").click();
    const patterns = document.querySelectorAll<HTMLInputElement>(".rule input");
    expect(document.activeElement).toBe(patterns[1]);
    patterns[1]!.value = " Zed ";
    document.querySelectorAll<HTMLSelectElement>(".rule select")[1]!.value = "code";
    byText<HTMLButtonElement>(document, "button", "Add rule").click();
    byText<HTMLButtonElement>(document, "button", "Save rules").click();
    await settle();
    expect(ctx.api.saveRules).toHaveBeenCalledWith([
      { pattern: "Linear", category: "work" },
      { pattern: "Zed", category: "code" },
    ]);
    expect(toasts()).toContain("Rules saved");

    document.querySelector<HTMLButtonElement>(".rule [aria-label='Remove rule']")!.click();
    ctx.api.saveRules.mockRejectedValueOnce("“Zed” is listed twice.");
    byText<HTMLButtonElement>(document, "button", "Save rules").click();
    await settle();
    expect(ctx.api.saveRules).toHaveBeenLastCalledWith([{ pattern: "Zed", category: "code" }]);
    const error = document.querySelector<HTMLElement>(".section .field-error:not([hidden])")!;
    expect(error.textContent).toBe("“Zed” is listed twice.");
  });

  it("formats typed text in the playground", async () => {
    vi.useFakeTimers();
    await setup();
    const output = document.querySelector("output")!;
    expect(ctx.api.tryFormat).toHaveBeenCalledWith(
      expect.stringContaining("two lakh fifty thousand"),
      "",
    );
    expect(output.textContent).toContain("formatted: um my plan");

    type(byLabel<HTMLInputElement>(document, "Into this app"), " Slack ");
    type(byLabel<HTMLTextAreaElement>(document, "You say"), "hello");
    vi.advanceTimersByTime(300);
    await settle();
    expect(ctx.api.tryFormat).toHaveBeenLastCalledWith("hello", "Slack");
    expect(output.textContent).toBe("formatted: hello");

    ctx.api.tryFormat.mockRejectedValueOnce("The language model timed out.");
    type(byLabel<HTMLTextAreaElement>(document, "You say"), "again");
    vi.advanceTimersByTime(300);
    await settle();
    expect(output.classList.contains("error")).toBe(true);
    expect(output.textContent).toBe("The language model timed out.");
  });

  it("drops a playground result that arrives after a newer one", async () => {
    vi.useFakeTimers();
    await setup();
    let slow = (_: string) => undefined as unknown;
    ctx.api.tryFormat.mockImplementationOnce(() => new Promise((resolve) => (slow = resolve)));
    const input = byLabel<HTMLTextAreaElement>(document, "You say");
    type(input, "first");
    vi.advanceTimersByTime(300);
    type(input, "second");
    vi.advanceTimersByTime(300);
    await settle();
    slow("stale");
    await settle();
    expect(document.querySelector("output")!.textContent).toBe("formatted: second");
  });
});
