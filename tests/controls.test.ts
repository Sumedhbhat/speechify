import { beforeEach, describe, expect, it } from "vitest";
import { number, select, text, toggle } from "../src/controls";
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
