import { beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { createBar } from "../src/bar-view";
import { fakeApi, fakeEvents, settle } from "./fakes";

let api: ReturnType<typeof fakeApi>;
let emit: ReturnType<typeof fakeEvents>["emit"];
let frames: ((time: number) => void)[];
let cancelFrame: Mock<(id: number) => void>;
let reduced: boolean;
let root: HTMLElement;

function setup() {
  api = fakeApi();
  const fake = fakeEvents();
  emit = fake.emit;
  frames = [];
  cancelFrame = vi.fn<(id: number) => void>();
  root = document.createElement("div");
  document.body.append(root);
  return createBar(root, {
    api,
    events: fake.events,
    frame: (callback) => frames.push(callback),
    cancelFrame,
    reducedMotion: () => reduced,
  });
}

const pill = () => root.querySelector<HTMLElement>(".pill")!;
const button = (label: string) => root.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!;

/** Runs the newest scheduled animation frame. */
function nextFrame(time: number) {
  frames[frames.length - 1]!(time);
}

beforeEach(() => {
  vi.useFakeTimers();
  reduced = false;
});

describe("bar", () => {
  it("starts as a small idle pill that dictates on click", () => {
    setup();
    expect(pill().className).toBe("pill idle");
    expect(root.querySelector(".pill-hint")!.textContent).toBe("Click to dictate");
    button("Click to dictate").click();
    expect(api.barAction).toHaveBeenCalledWith("click");
  });

  it("swallows a failed action", async () => {
    setup();
    api.barAction.mockRejectedValueOnce("Shell is busy");
    button("Click to dictate").click();
    await settle();
    expect(pill().className).toBe("pill idle");
  });

  it("listens with a live waveform and a cancel button", () => {
    setup();
    emit("bar", { state: "listening", command: false, locked: false });
    expect(pill().className).toBe("pill listening");
    expect(root.querySelectorAll(".wave-bar").length).toBe(9);
    expect(button("Finish dictation")).toBeNull();
    expect(root.querySelector(".pill-label")).toBeNull();
    expect(root.querySelector(".sr-only")!.textContent).toBe("Listening");

    const bars = [...root.querySelectorAll<HTMLElement>(".wave-bar")];
    emit("level", 0.8);
    nextFrame(100);
    const loud = Number(bars[4]!.style.transform.match(/[\d.]+/)![0]);
    expect(loud).toBeGreaterThan(0.5);
    for (let t = 0; t < 40; t++) nextFrame(200 + t * 16);
    const quiet = Number(bars[4]!.style.transform.match(/[\d.]+/)![0]);
    expect(quiet).toBeLessThan(loud);

    emit("level", 5);
    emit("level", -1);
    nextFrame(900);

    button("Cancel dictation").click();
    expect(api.barAction).toHaveBeenCalledWith("cancel");
  });

  it("keeps the waveform still when motion is reduced", () => {
    reduced = true;
    setup();
    emit("bar", { state: "listening", command: false, locked: false });
    emit("level", 1);
    nextFrame(0);
    const first = root.querySelector<HTMLElement>(".wave-bar")!.style.transform;
    emit("level", 1);
    nextFrame(0);
    nextFrame(500);
    expect(root.querySelector<HTMLElement>(".wave-bar")!.style.transform).not.toBe("");
    expect(first).toMatch(/scaleY/);
  });

  it("shows command mode and a finish button when hands-free", () => {
    setup();
    emit("bar", { state: "listening", command: true, locked: true });
    expect(pill().classList.contains("command")).toBe(true);
    expect(root.querySelector(".pill-label")!.textContent).toBe("Command");
    expect(root.querySelector(".sr-only")!.textContent).toBe("Listening for a command");
    button("Finish dictation").click();
    expect(api.barAction).toHaveBeenCalledWith("click");
  });

  it("stops animating when the state changes", () => {
    setup();
    emit("bar", { state: "listening", command: false, locked: false });
    emit("bar", { state: "processing" });
    expect(cancelFrame).toHaveBeenCalled();
    expect(root.querySelector(".shimmer")).not.toBeNull();
    expect(root.querySelector("button")).toBeNull();
    expect(root.querySelector(".sr-only")!.textContent).toBe("Working on it");
  });

  it("flashes done with a word count, then returns to idle", () => {
    setup();
    emit("bar", { state: "done", words: 12 });
    expect(pill().className).toBe("pill done");
    expect(root.querySelector(".pill-label")!.textContent).toBe("12 words");
    vi.advanceTimersByTime(1200);
    expect(pill().className).toBe("pill idle");
    emit("bar", { state: "done", words: 1 });
    expect(root.querySelector(".pill-label")!.textContent).toBe("1 word");
    emit("bar", { state: "done", words: 0 });
    expect(root.querySelector(".pill-label")!.textContent).toBe("Done");
  });

  it("shows a card that stays until dismissed and copies the text", async () => {
    setup();
    emit("bar", {
      state: "card",
      text: "Hello there",
      message: "Select a text box first, then dictate",
    });
    expect(root.querySelector(".card-text")!.textContent).toBe("Hello there");
    expect(root.querySelector(".card-message")!.textContent).toBe(
      "Select a text box first, then dictate",
    );
    expect(pill().className).toBe("pill idle");
    vi.advanceTimersByTime(60_000);
    expect(root.querySelector(".card")).not.toBeNull();

    root.querySelector<HTMLButtonElement>(".card-copy")!.click();
    await settle();
    expect(api.copyText).toHaveBeenCalledWith("Hello there");
    expect(root.querySelector(".card-copy")!.textContent).toBe("Copied");
    vi.advanceTimersByTime(1000);
    expect(api.barAction).toHaveBeenCalledWith("dismiss");
    expect(root.querySelector(".card")).toBeNull();
  });

  it("keeps a new dictation on screen after two quick copies", async () => {
    setup();
    emit("bar", { state: "card", text: "Hello there", message: "" });
    root.querySelector<HTMLButtonElement>(".card-copy")!.click();
    root.querySelector<HTMLButtonElement>(".card-copy")!.click();
    await settle();
    emit("bar", { state: "listening", command: false, locked: false });
    vi.advanceTimersByTime(60_000);
    expect(api.barAction).not.toHaveBeenCalledWith("dismiss");
    expect(pill().className).toContain("listening");
  });

  it("closes the card with the X and shows copy errors", async () => {
    setup();
    emit("bar", { state: "card", text: "Hi", message: "" });
    expect(root.querySelector<HTMLElement>(".card-message")!.hidden).toBe(true);
    api.copyText.mockRejectedValueOnce("Clipboard is busy.");
    root.querySelector<HTMLButtonElement>(".card-copy")!.click();
    await settle();
    const note = root.querySelector<HTMLElement>(".card-message.error")!;
    expect(note.textContent).toBe("Clipboard is busy.");
    expect(note.hidden).toBe(false);
    button("Dismiss").click();
    expect(api.barAction).toHaveBeenCalledWith("dismiss");
  });

  it("ignores a copy that finishes after the card was replaced", async () => {
    setup();
    let finish = () => undefined as unknown;
    api.copyText.mockImplementationOnce(() => new Promise<void>((resolve) => (finish = resolve)));
    emit("bar", { state: "card", text: "Hi", message: "m" });
    root.querySelector<HTMLButtonElement>(".card-copy")!.click();
    emit("bar", { state: "processing" });
    finish();
    await settle();
    vi.advanceTimersByTime(2000);
    expect(api.barAction).not.toHaveBeenCalled();
    expect(pill().className).toBe("pill processing");
  });

  it("shows notices with undo and hides them after a while", () => {
    setup();
    emit("bar", { state: "notice", message: "Pasted", error: false, undo: true });
    const notice = root.querySelector(".notice")!;
    expect(notice.getAttribute("role")).toBe("status");
    root.querySelector<HTMLButtonElement>(".notice-undo")!.click();
    expect(api.barAction).toHaveBeenCalledWith("undo");
    vi.advanceTimersByTime(4000);
    expect(root.querySelector(".notice")).toBeNull();
  });

  it("keeps errors up longer and without undo", () => {
    setup();
    emit("bar", { state: "notice", message: "Microphone not found", error: true, undo: false });
    expect(root.querySelector(".notice.error")!.getAttribute("role")).toBe("alert");
    expect(root.querySelector(".notice-undo")).toBeNull();
    vi.advanceTimersByTime(4000);
    expect(root.querySelector(".notice")).not.toBeNull();
    vi.advanceTimersByTime(2000);
    expect(root.querySelector(".notice")).toBeNull();
  });

  it("cancels pending timers when a new state arrives", () => {
    const bar = setup();
    bar.show({ state: "notice", message: "x", error: false, undo: false });
    bar.show({ state: "listening", command: false, locked: false });
    vi.advanceTimersByTime(10_000);
    expect(pill().className).toBe("pill listening");
  });
});
