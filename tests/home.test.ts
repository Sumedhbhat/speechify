import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Dictation } from "../src/api";
import { createHomePage } from "../src/pages/home";
import {
  byLabel,
  byText,
  dictation,
  insights,
  makeDeps,
  settle,
  status,
  toasts,
  type,
} from "./fakes";

let ctx: ReturnType<typeof makeDeps>;
const DAY = 86_400;
const now = Math.floor(Date.now() / 1000);

function items(): Dictation[] {
  return [
    dictation({ id: 1, audio: "yes", command: true, text: "First." }),
    dictation({
      id: 2,
      app: "",
      created_at: now - 60,
      teacher_text: "Second, by the teacher.",
      rewritten: false,
      text: "Second.",
    }),
    dictation({ id: 3, created_at: now - DAY, text: "Third.", flagged: true }),
    dictation({ id: 4, created_at: now - 10 * DAY, text: "Fourth." }),
  ];
}

async function setup(list: Dictation[] = items()) {
  ctx.api.history.mockResolvedValue(list);
  const page = createHomePage(ctx.deps);
  document.body.append(page.el);
  await page.show();
  await settle();
  return page;
}

const entries = () => [...document.querySelectorAll<HTMLElement>(".entry")];
const action = (entry: HTMLElement, name: string) =>
  entry.querySelector<HTMLButtonElement>(`[data-action="${name}"]`)!;

beforeEach(() => {
  ctx = makeDeps();
});

describe("home header", () => {
  it("greets, shows headline numbers and the checklist", async () => {
    await setup([]);
    expect(document.querySelector("h1")!.textContent).toMatch(/Good|Working/);
    expect(document.querySelector(".lede")!.textContent).toBe("You have dictated 250 words today.");
    const stats = [...document.querySelectorAll(".stat")].map((s) => s.textContent);
    expect(stats).toEqual([
      `Words this week${(1800).toLocaleString()}`,
      "Speaking speed142wpm",
      "Streak4days",
      "Time saved210min",
    ]);
    expect(document.querySelector(".checklist h2")!.textContent).toBe("Ready when you are");
    expect(document.querySelectorAll(".step.done").length).toBe(3);
    expect(document.querySelector("kbd")!.textContent).toBe("Ctrl+Meta+Space");
  });

  it("guides a new user and explains Wayland", async () => {
    ctx.api.getInsights.mockResolvedValue(insights({ today_words: 0, streak: 1 }));
    ctx.api.getStatus.mockResolvedValue(
      status({ speech_ready: false, rewrite_reachable: false, wayland: true }),
    );
    await setup([]);
    expect(document.querySelector(".lede")!.textContent).toBe(
      "Hold your shortcut anywhere and start talking.",
    );
    expect(document.querySelector(".stats")!.textContent).toContain("1day");
    expect(document.querySelector(".checklist h2")!.textContent).toBe("Getting started");
    expect(document.querySelectorAll(".step.done").length).toBe(0);
    expect(document.querySelector(".checklist")!.textContent).toContain("speechify --toggle");
    byText<HTMLButtonElement>(document, "button", "Choose a model").click();
    byText<HTMLButtonElement>(document, "button", "Set up").click();
    expect(ctx.navigate).toHaveBeenCalledWith("models");
    expect(ctx.navigate).toHaveBeenCalledTimes(2);
  });
});

describe("history", () => {
  it("groups dictations by day with the details of each", async () => {
    await setup();
    const days = [...document.querySelectorAll(".day h3")].map((el) => el.textContent);
    expect(days.slice(0, 2)).toEqual(["Today", "Yesterday"]);
    expect(days).toHaveLength(3);
    const [first, second, third] = entries();
    expect(first!.querySelector(".entry-app")!.textContent).toBe("Slack");
    expect(first!.querySelector(".chip.accent")!.textContent).toBe("Command");
    expect(second!.querySelector(".entry-app")!.textContent).toBe("Unknown app");
    expect(third!.classList.contains("flagged")).toBe(true);
    expect(action(first!, "play")).not.toBeNull();
    expect(action(second!, "play")).toBeNull();
    expect(document.querySelector<HTMLButtonElement>(".more")!.hidden).toBe(true);
  });

  it("expands to show what was heard and how it was made", async () => {
    await setup();
    const toggle = action(entries()[0]!, "details");
    toggle.focus();
    toggle.click();
    const open = entries()[0]!;
    expect(action(open, "details").getAttribute("aria-expanded")).toBe("true");
    expect(document.activeElement).toBe(action(open, "details"));
    const detail = open.querySelector(".detail")!.textContent;
    expect(detail).toContain("um hello there");
    expect(detail).toContain("Recognition 320 ms");
    expect(detail).toContain("Whisper turbo + qwen2.5:1.5b");
    expect(detail).toContain("Typed into the app");
    expect(detail).not.toContain("Teacher");

    action(entries()[1]!, "details").click();
    const second = entries()[1]!.querySelector(".detail")!.textContent;
    expect(second).toContain("Second, by the teacher.");
    expect(second).toContain("(not rewritten)");

    action(entries()[0]!, "details").click();
    expect(entries()[0]!.querySelector(".detail")).toBeNull();
  });

  it("copies and reports copy failures", async () => {
    await setup();
    action(entries()[0]!, "copy").click();
    await settle();
    expect(ctx.api.copyText).toHaveBeenCalledWith("First.");
    expect(toasts()).toContain("Copied");
    ctx.api.copyText.mockRejectedValueOnce("Clipboard is busy.");
    action(entries()[0]!, "copy").click();
    await settle();
    expect(toasts()).toContain("Clipboard is busy.");
  });

  it("plays recordings one at a time", async () => {
    const play = vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue();
    const pause = vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => undefined);
    ctx.api.dictationAudio.mockResolvedValue("data:audio/wav;base64,AAAA");
    await setup();
    action(entries()[0]!, "play").click();
    await settle();
    expect(ctx.api.dictationAudio).toHaveBeenCalledWith(1);
    expect(play).toHaveBeenCalledOnce();
    expect(pause).not.toHaveBeenCalled();
    action(entries()[0]!, "play").click();
    await settle();
    expect(pause).toHaveBeenCalledOnce();

    ctx.api.dictationAudio.mockResolvedValueOnce("");
    action(entries()[0]!, "play").click();
    await settle();
    expect(toasts()).toContain("This recording is no longer available.");

    play.mockRejectedValueOnce(new Error("Cannot play this file."));
    action(entries()[0]!, "play").click();
    await settle();
    expect(toasts()).toContain("Cannot play this file.");
  });

  it("flags and unflags", async () => {
    await setup();
    action(entries()[0]!, "flag").click();
    await settle();
    expect(ctx.api.flagDictation).toHaveBeenCalledWith(1, true);
    expect(action(entries()[0]!, "flag").getAttribute("aria-pressed")).toBe("true");
    expect(entries()[0]!.querySelector(".chip.warn")).not.toBeNull();
    ctx.api.flagDictation.mockRejectedValueOnce("Could not flag.");
    action(entries()[0]!, "flag").click();
    await settle();
    expect(toasts()).toContain("Could not flag.");
    expect(action(entries()[0]!, "flag").getAttribute("aria-label")).toBe("Unflag");
  });

  it("edits a dictation and reports what was learned", async () => {
    await setup();
    const edit = action(entries()[0]!, "edit");
    edit.focus();
    edit.click();
    await settle();
    const area = document.querySelector<HTMLTextAreaElement>(".entry-edit")!;
    expect(document.activeElement).toBe(area);
    area.value = "  First, fixed. ";
    ctx.api.editDictation.mockResolvedValueOnce([
      { from: "jason", to: "JSON" },
      { from: "a", to: "b" },
    ]);
    const save = byText<HTMLButtonElement>(document, "button", "Save");
    save.focus();
    save.click();
    await settle();
    expect(ctx.api.editDictation).toHaveBeenCalledWith(1, "First, fixed.");
    expect(toasts()).toContain("Learned: jason → JSON, a → b");
    expect(entries()[0]!.querySelector(".entry-text")!.textContent).toBe("First, fixed.");
    expect(document.activeElement).toBe(action(entries()[0]!, "edit"));

    action(entries()[1]!, "edit").click();
    byText<HTMLButtonElement>(document, "button", "Save").click();
    await settle();
    expect(toasts()).toContain("Correction saved");
  });

  it("keeps the editor open when saving fails, and cancels", async () => {
    await setup();
    action(entries()[0]!, "edit").click();
    ctx.api.editDictation.mockRejectedValueOnce("Text cannot be empty.");
    byText<HTMLButtonElement>(document, "button", "Save").click();
    await settle();
    expect(toasts()).toContain("Text cannot be empty.");
    expect(document.querySelector(".entry-edit")).not.toBeNull();
    byText<HTMLButtonElement>(document, "button", "Cancel").click();
    expect(document.querySelector(".entry-edit")).toBeNull();
  });

  it("waits for an edit to finish before refreshing", async () => {
    await setup();
    action(entries()[0]!, "edit").click();
    ctx.api.history.mockResolvedValue([dictation({ id: 9, text: "New one." })]);
    ctx.emit("historyChanged");
    await settle();
    expect(document.querySelector(".entry-edit")).not.toBeNull();
    byText<HTMLButtonElement>(document, "button", "Cancel").click();
    await settle();
    expect(entries().map((e) => e.querySelector(".entry-text")!.textContent)).toEqual(["New one."]);

    ctx.api.history.mockRejectedValueOnce("History is unavailable.");
    ctx.emit("historyChanged");
    await settle();
    expect(toasts()).toContain("History is unavailable.");
  });

  it("deletes one dictation or, after confirming, all of them", async () => {
    await setup();
    action(entries()[0]!, "delete").click();
    await settle();
    expect(ctx.api.deleteDictations).toHaveBeenCalledWith([1]);
    expect(entries()).toHaveLength(3);
    ctx.api.deleteDictations.mockRejectedValueOnce("Could not delete.");
    action(entries()[0]!, "delete").click();
    await settle();
    expect(toasts()).toContain("Could not delete.");

    const clear = byText<HTMLButtonElement>(document, "button", "Clear history");
    clear.click();
    ctx.api.history.mockResolvedValue([]);
    clear.click();
    await settle();
    expect(ctx.api.deleteDictations).toHaveBeenLastCalledWith(null);
    expect(document.querySelector(".empty-title")!.textContent).toBe("Nothing here yet");

    ctx.api.deleteDictations.mockRejectedValueOnce("Could not clear.");
    clear.click();
    clear.click();
    await settle();
    expect(toasts()).toContain("Could not clear.");
  });

  it("searches after a pause and pages through results", async () => {
    vi.useFakeTimers();
    const page = Array.from({ length: 50 }, (_, i) => dictation({ id: 100 + i }));
    await setup(page);
    expect(document.querySelector<HTMLButtonElement>(".more")!.hidden).toBe(false);
    ctx.api.history.mockResolvedValueOnce([dictation({ id: 999, text: "Last." })]);
    document.querySelector<HTMLButtonElement>(".more")!.click();
    await settle();
    expect(ctx.api.history).toHaveBeenLastCalledWith("", 50, 50);
    expect(entries()).toHaveLength(51);
    expect(document.querySelector<HTMLButtonElement>(".more")!.hidden).toBe(true);

    ctx.api.history.mockResolvedValue([]);
    type(byLabel<HTMLInputElement>(document, "Search history"), "  milk ");
    vi.advanceTimersByTime(249);
    expect(ctx.api.history).toHaveBeenCalledTimes(2);
    vi.advanceTimersByTime(1);
    await settle();
    expect(ctx.api.history).toHaveBeenLastCalledWith("milk", 50, 0);
    expect(document.querySelector(".empty")!.textContent).toContain("“milk”");
  });

  it("ignores results of an older search", async () => {
    await setup([]);
    let first = (_: Dictation[]) => undefined as unknown;
    ctx.api.history.mockImplementationOnce(() => new Promise((resolve) => (first = resolve)));
    ctx.api.history.mockResolvedValueOnce([dictation({ id: 7, text: "Newest." })]);
    ctx.emit("historyChanged");
    ctx.emit("historyChanged");
    await settle();
    first([dictation({ id: 6, text: "Stale." })]);
    await settle();
    expect(entries().map((e) => e.querySelector(".entry-text")!.textContent)).toEqual(["Newest."]);
  });
});
