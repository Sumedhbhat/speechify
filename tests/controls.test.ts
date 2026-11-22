import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  chordName,
  number,
  RECORD_LIMIT_MS,
  select,
  shortcut,
  text,
  toggle,
} from "../src/controls";
import type { Settings } from "../src/api";
import { change, makeDeps, settle } from "./fakes";

let ctx: ReturnType<typeof makeDeps>;
let current: Settings;

beforeEach(async () => {
  ctx = makeDeps();
  current = await ctx.deps.settings.load();
});

function mount(el: HTMLElement) {
  document.body.append(el);
  return el;
}

describe("toggle", () => {
  it("saves on change and reverts when rejected", async () => {
    const el = mount(
      toggle(ctx.deps.settings, current, {
        label: "Play sounds",
        hint: "Little clicks.",
        get: (s) => s.sounds,
        set: (s, v) => (s.sounds = v),
      }),
    );
    const input = el.querySelector("input")!;
    expect(input.checked).toBe(true);
    expect(input.getAttribute("role")).toBe("switch");
    expect(input.getAttribute("aria-describedby")).toBe(el.querySelector(".hint")!.id);
    expect(el.querySelector("label")!.htmlFor).toBe(input.id);

    change(input, false);
    await settle();
    expect(ctx.api.saveSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({ sounds: false }),
    );
    const error = el.querySelector<HTMLElement>(".field-error")!;
    expect(error.hidden).toBe(true);

    ctx.api.saveSettings.mockRejectedValueOnce("Cannot change that now.");
    change(input, true);
    await settle();
    expect(input.checked).toBe(false);
    expect(error.hidden).toBe(false);
    expect(error.textContent).toBe("Cannot change that now.");
  });

  it("has no description without a hint", () => {
    const el = toggle(ctx.deps.settings, current, {
      label: "X",
      get: (s) => s.sounds,
      set: () => undefined,
    });
    expect(el.querySelector("input")!.hasAttribute("aria-describedby")).toBe(false);
    expect(el.querySelector(".hint")).toBeNull();
  });
});

describe("select", () => {
  it("lists options, keeps an unknown current value and saves", async () => {
    current.microphone = "Old mic";
    const el = mount(
      select(ctx.deps.settings, current, {
        label: "Microphone",
        options: [
          ["", "System default"],
          ["USB", "USB"],
        ],
        get: (s) => s.microphone,
        set: (s, v) => (s.microphone = v),
      }),
    );
    const input = el.querySelector("select")!;
    expect([...input.options].map((o) => o.value)).toEqual(["", "USB", "Old mic"]);
    expect(input.value).toBe("Old mic");
    change(input, "USB");
    await settle();
    expect(ctx.api.saveSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({ microphone: "USB" }),
    );
  });

  it("does not duplicate a known value", () => {
    const el = select(ctx.deps.settings, current, {
      label: "Theme",
      options: [["system", "System"]],
      get: (s) => s.theme,
      set: (s, v) => (s.theme = v),
    });
    expect(el.querySelectorAll("option").length).toBe(1);
  });
});

describe("text", () => {
  it("trims, saves on change and can attach a datalist", async () => {
    const el = mount(
      text(ctx.deps.settings, current, {
        label: "Model",
        list: "models",
        type: "url",
        placeholder: "name",
        get: (s) => s.rewrite.model,
        set: (s, v) => (s.rewrite.model = v),
      }),
    );
    const input = el.querySelector("input")!;
    expect(input.getAttribute("list")).toBe("models");
    expect(input.type).toBe("url");
    expect(input.placeholder).toBe("name");
    change(input, "  llama3.2:3b ");
    await settle();
    expect(ctx.api.saveSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({ rewrite: expect.objectContaining({ model: "llama3.2:3b" }) }),
    );
  });

  it("defaults to a plain text box", () => {
    const input = text(ctx.deps.settings, current, {
      label: "Key",
      get: (s) => s.speech.remote_key,
      set: () => undefined,
    }).querySelector("input")!;
    expect(input.type).toBe("text");
    expect(input.placeholder).toBe("");
    expect(input.hasAttribute("list")).toBe(false);
  });
});

describe("number", () => {
  it("clamps, falls back to the minimum and shows a unit", async () => {
    const el = mount(
      number(ctx.deps.settings, current, {
        label: "Timeout",
        min: 500,
        max: 60000,
        step: 500,
        unit: "ms",
        get: (s) => s.rewrite.timeout_ms,
        set: (s, v) => (s.rewrite.timeout_ms = v),
      }),
    );
    const input = el.querySelector("input")!;
    expect(input.value).toBe("3000");
    expect(el.querySelector(".with-unit")!.textContent).toBe("ms");
    const saved = () => (ctx.api.saveSettings.mock.lastCall![0] as Settings).rewrite.timeout_ms;
    change(input, "999999");
    await settle();
    expect(saved()).toBe(60000);
    change(input, "abc");
    await settle();
    expect(saved()).toBe(500);
    change(input, "1200.4");
    await settle();
    expect(saved()).toBe(1200);
  });

  it("renders without a unit", () => {
    const el = number(ctx.deps.settings, current, {
      label: "Words",
      min: 0,
      max: 10,
      get: (s) => s.rewrite.min_words,
      set: () => undefined,
    });
    expect(el.querySelector(".with-unit")).toBeNull();
    expect(el.querySelector("input")!.step).toBe("1");
  });
});

describe("chordName", () => {
  it("keeps the side of a modifier pressed alone", () => {
    expect(chordName(["ShiftRight"])).toBe("Right Shift");
    expect(chordName(["ControlLeft"])).toBe("Left Ctrl");
    expect(chordName(["AltRight"])).toBe("Right Alt");
    expect(chordName(["MetaLeft"])).toBe("Left Meta");
  });

  it("names combinations with either side, modifiers first", () => {
    expect(chordName(["KeyZ", "ShiftRight", "AltLeft"])).toBe("Alt+Shift+Z");
    expect(chordName(["MetaLeft", "ControlRight", "Space"])).toBe("Ctrl+Meta+Space");
    expect(chordName(["ShiftLeft", "ShiftRight"])).toBe("Shift");
    expect(chordName(["ControlLeft", "Digit5"])).toBe("Ctrl+5");
    expect(chordName(["F13"])).toBe("F13");
    expect(chordName(["Escape"])).toBe("Escape");
  });

  it("takes letters from the keyboard layout when given", () => {
    // AZERTY: the key in Q's place types A.
    expect(chordName(["ControlLeft", "KeyQ"], { KeyQ: "a" })).toBe("Ctrl+A");
    // Shifted digits and composed characters keep the key's place.
    expect(chordName(["ShiftLeft", "Digit1"], { Digit1: "&" })).toBe("Shift+1");
    expect(chordName(["AltLeft", "KeyA"], { KeyA: "å" })).toBe("Alt+A");
  });

  it("leaves keys a shortcut cannot use for the shell to reject", () => {
    expect(chordName(["ControlLeft", "ArrowLeft"])).toBe("Ctrl+ArrowLeft");
    expect(chordName(["KeyAB"])).toBe("KeyAB");
  });
});

describe("shortcut", () => {
  const pause = vi.fn(async (_paused: boolean) => undefined);

  function press(type: "keydown" | "keyup", code: string, repeat = false, key = "") {
    const event = new KeyboardEvent(type, { code, key, repeat, cancelable: true });
    window.dispatchEvent(event);
    return event;
  }

  function recorder(get: (s: Settings) => string = (s) => s.hotkeys.push_to_talk, layout = false) {
    const el = mount(
      shortcut(ctx.deps.settings, current, {
        label: "Push to talk",
        hint: "Hold to dictate.",
        pause,
        layout,
        get,
        set: (s, v) => (s.hotkeys.push_to_talk = v),
      }),
    );
    const [record, clear] = [...el.querySelectorAll("button")];
    return { el, record: record!, clear: clear!, keys: () => el.querySelector(".shortcut-keys")! };
  }

  const saved = () => (ctx.api.saveSettings.mock.lastCall![0] as Settings).hotkeys.push_to_talk;

  beforeEach(() => pause.mockClear());

  it("shows the keys, labelled, and does not pause until recording", () => {
    const { el, record, clear, keys } = recorder();
    expect([...keys().querySelectorAll("kbd")].map((k) => k.textContent)).toEqual([
      "Ctrl",
      "Meta",
      "Space",
    ]);
    expect(el.querySelector("label")!.htmlFor).toBe(record.id);
    expect(record.getAttribute("aria-describedby")).toBe(el.querySelector(".hint")!.id);
    expect(record.textContent).toBe("Record");
    expect(record.getAttribute("aria-pressed")).toBe("false");
    expect(clear.getAttribute("aria-label")).toBe("Clear Push to talk");
    expect(pause).not.toHaveBeenCalled();
  });

  it("records the keys held when the first one is let go", async () => {
    const { record, keys } = recorder();
    record.click();
    expect(pause).toHaveBeenLastCalledWith(true);
    expect(record.textContent).toBe("Press keys…");
    expect(record.getAttribute("aria-pressed")).toBe("true");
    expect(keys().textContent).toBe("Listening…");
    // Enter can start recording and be let go just after: it is not part of it.
    expect(press("keyup", "Enter").defaultPrevented).toBe(true);
    expect(press("keydown", "AltLeft").defaultPrevented).toBe(true);
    press("keydown", "AltLeft", true);
    press("keydown", "");
    press("keydown", "ShiftRight");
    press("keydown", "KeyZ");
    expect(keys().textContent).toBe("AltShiftZ");
    press("keyup", "KeyZ");
    await settle();
    expect(saved()).toBe("Alt+Shift+Z");
    expect(pause).toHaveBeenLastCalledWith(false);
    expect(record.textContent).toBe("Record");
    // Keys after recording are left alone.
    expect(press("keydown", "KeyA").defaultPrevented).toBe(false);
  });

  it("gives up when clicked again or left, keeping the saved shortcut", async () => {
    const { record, keys } = recorder();
    record.click();
    press("keydown", "ShiftLeft");
    record.click();
    expect(pause).toHaveBeenLastCalledWith(false);
    expect(keys().textContent).toBe("CtrlMetaSpace");
    record.click();
    record.dispatchEvent(new FocusEvent("blur"));
    expect(pause).toHaveBeenCalledTimes(4);
    expect(keys().textContent).toBe("CtrlMetaSpace");
    record.dispatchEvent(new FocusEvent("blur"));
    expect(pause).toHaveBeenCalledTimes(4);
    await settle();
    expect(ctx.api.saveSettings).not.toHaveBeenCalled();
  });

  it("clears to off, and survives the shell refusing to pause", async () => {
    const { record, clear, keys } = recorder();
    clear.click();
    await settle();
    expect(saved()).toBe("");
    expect(keys().textContent).toBe("Off");
    pause.mockRejectedValueOnce("gone");
    record.click();
    press("keydown", "ShiftRight");
    press("keyup", "ShiftRight");
    await settle();
    expect(saved()).toBe("Right Shift");
  });

  it("hides an old error when recording again", async () => {
    ctx.api.saveSettings.mockRejectedValueOnce("Unknown key “ArrowUp”");
    const { el, record } = recorder();
    record.click();
    press("keydown", "ArrowUp");
    press("keyup", "ArrowUp");
    await settle();
    const error = el.querySelector<HTMLElement>(".field-error")!;
    expect(error.hidden).toBe(false);
    record.click();
    expect(error.hidden).toBe(true);
  });

  it("follows the keyboard layout where the shell's reader does", async () => {
    const { record } = recorder(undefined, true);
    record.click();
    press("keydown", "ControlLeft", false, "Control");
    press("keydown", "KeyQ", false, "a");
    press("keyup", "KeyQ");
    await settle();
    expect(saved()).toBe("Ctrl+A");
  });

  it("gives up before the shell stops pausing shortcuts", () => {
    vi.useFakeTimers();
    try {
      const { record, keys } = recorder();
      record.click();
      vi.advanceTimersByTime(RECORD_LIMIT_MS - 1);
      expect(record.textContent).toBe("Press keys…");
      vi.advanceTimersByTime(1);
      expect(record.textContent).toBe("Record");
      expect(pause).toHaveBeenLastCalledWith(false);
      expect(keys().textContent).toBe("CtrlMetaSpace");
    } finally {
      vi.useRealTimers();
    }
  });
});
