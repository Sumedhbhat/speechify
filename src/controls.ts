// Form controls bound to one setting. Each saves as soon as it changes, shows
// the shell's error under itself when a save is rejected, and reverts.

import type { Settings } from "./api";
import type { SettingsStore } from "./settings";
import { h, nextId, type Child } from "./ui";

interface Labelled {
  label: string;
  hint?: string;
}

export interface Binding<T> extends Labelled {
  get: (settings: Settings) => T;
  set: (settings: Settings, value: T) => void;
}

/** Wires a control's value to the store; returns the commit function. */
function bind<T>(
  store: SettingsStore,
  settings: Settings,
  binding: Binding<T>,
  write: (value: T) => void,
  error: HTMLElement,
): (value: T) => Promise<void> {
  let saved = binding.get(settings);
  write(saved);
  return async (value) => {
    const message = await store.update((next) => binding.set(next, value));
    error.textContent = message;
    error.hidden = !message;
    // Show what the shell accepted, which can differ from what was typed
    // (a clamped number), or the last accepted value after a rejection.
    if (!message) saved = binding.get(await store.load());
    write(saved);
  };
}

function field(
  id: string,
  binding: Labelled,
  control: Child,
  error: HTMLElement,
  className = "field",
): HTMLElement {
  return h("div", { className }, [
    h("div", { className: "field-text" }, [
      h("label", { htmlFor: id, textContent: binding.label }),
      binding.hint && h("p", { className: "hint", id: `${id}-hint`, textContent: binding.hint }),
    ]),
    h("div", { className: "field-control" }, [control]),
    error,
  ]);
}

function errorSlot(): HTMLElement {
  return h("p", { className: "field-error", role: "alert", hidden: true });
}

function describedBy(id: string, binding: Labelled): string | undefined {
  return binding.hint ? `${id}-hint` : undefined;
}

export function toggle(
  store: SettingsStore,
  settings: Settings,
  binding: Binding<boolean>,
): HTMLElement {
  const id = nextId("toggle");
  const error = errorSlot();
  const input = h("input", {
    type: "checkbox",
    id,
    className: "switch",
    role: "switch",
    "aria-describedby": describedBy(id, binding),
  });
  const commit = bind(store, settings, binding, (value) => (input.checked = value), error);
  input.addEventListener("change", () => void commit(input.checked));
  return field(id, binding, input, error, "field toggle-field");
}

export function select(
  store: SettingsStore,
  settings: Settings,
  binding: Binding<string> & { options: [value: string, label: string][] },
): HTMLElement {
  const id = nextId("select");
  const error = errorSlot();
  const options = [...binding.options];
  const current = binding.get(settings);
  if (!options.some(([value]) => value === current)) options.push([current, current]);
  const input = h(
    "select",
    { id, "aria-describedby": describedBy(id, binding) },
    options.map(([value, label]) => h("option", { value, textContent: label })),
  );
  const commit = bind(store, settings, binding, (value) => (input.value = value), error);
  input.addEventListener("change", () => void commit(input.value));
  return field(id, binding, input, error);
}

export function text(
  store: SettingsStore,
  settings: Settings,
  binding: Binding<string> & { placeholder?: string; type?: string; list?: string },
): HTMLElement {
  const id = nextId("text");
  const error = errorSlot();
  const input = h("input", {
    id,
    type: binding.type ?? "text",
    placeholder: binding.placeholder ?? "",
    spellcheck: false,
    autocomplete: "off",
    "aria-describedby": describedBy(id, binding),
  });
  if (binding.list) input.setAttribute("list", binding.list);
  const write = (value: string) => {
    if (input.value === value) return;
    input.value = value;
    // A rejected save puts the old value back; tell anyone following the typing.
    input.dispatchEvent(new Event("input"));
  };
  const commit = bind(store, settings, binding, write, error);
  input.addEventListener("change", () => void commit(input.value.trim()));
  return field(id, binding, input, error);
}

export function number(
  store: SettingsStore,
  settings: Settings,
  binding: Binding<number> & { min: number; max: number; step?: number; unit?: string },
): HTMLElement {
  const id = nextId("number");
  const error = errorSlot();
  const input = h("input", {
    id,
    type: "number",
    min: String(binding.min),
    max: String(binding.max),
    step: String(binding.step ?? 1),
    "aria-describedby": describedBy(id, binding),
  });
  const write = (value: number) => (input.value = String(value));
  const commit = bind(store, settings, binding, write, error);
  input.addEventListener("change", () => {
    // Empty or invalid input falls back to the minimum; the range is clamped.
    const value = Math.round(Number(input.value) || binding.min);
    void commit(Math.min(binding.max, Math.max(binding.min, value)));
  });
  const control = binding.unit
    ? h("span", { className: "with-unit" }, [input, h("span", { textContent: binding.unit })])
    : input;
  return field(id, binding, control, error);
}

// Keyboard codes for modifiers: the plain name, then the name with its side.
const MODIFIERS: Record<string, [string, string]> = {
  ControlLeft: ["Ctrl", "Left Ctrl"],
  ControlRight: ["Ctrl", "Right Ctrl"],
  AltLeft: ["Alt", "Left Alt"],
  AltRight: ["Alt", "Right Alt"],
  ShiftLeft: ["Shift", "Left Shift"],
  ShiftRight: ["Shift", "Right Shift"],
  MetaLeft: ["Meta", "Left Meta"],
  MetaRight: ["Meta", "Right Meta"],
};
const ORDER = ["Ctrl", "Alt", "Shift", "Meta"];

/**
 * Names the keys held together, by `KeyboardEvent.code`, the way shortcut
 * settings write them. A modifier pressed alone keeps its side, so Right
 * Shift stays free of the left one; in a combination either side works
 * (Alt+Shift+Z). Keys a shortcut cannot use keep their code, and the shell
 * rejects them with its own reason.
 */
export function chordName(codes: string[]): string {
  const sides = codes.map((code) => MODIFIERS[code]);
  if (codes.length === 1 && sides[0]) return sides[0][1];
  const modifiers = new Set(sides.map((side) => side?.[0]));
  const others = codes
    .filter((code) => !MODIFIERS[code])
    .map((code) => code.replace(/^Key(?=[A-Z]$)|^Digit(?=\d$)/, ""));
  return [...ORDER.filter((name) => modifiers.has(name)), ...others].join("+");
}

/**
 * A shortcut setting, entered by pressing it: Record, then the keys. It is
 * taken when the first key is let go. Global shortcuts are paused meanwhile,
 * so pressing the current one does not start dictation.
 */
export function shortcut(
  store: SettingsStore,
  settings: Settings,
  binding: Binding<string> & { pause: (paused: boolean) => Promise<void> },
): HTMLElement {
  const id = nextId("shortcut");
  const error = errorSlot();
  const keys = h("span", { className: "shortcut-keys", "aria-live": "polite" });
  const record = h("button", {
    type: "button",
    id,
    className: "btn",
    "aria-describedby": describedBy(id, binding),
  });
  const clear = h("button", {
    type: "button",
    className: "btn danger-quiet",
    textContent: "Clear",
    "aria-label": `Clear ${binding.label}`,
  });
  const show = (value: string) => {
    keys.replaceChildren(
      ...(value ? value.split("+").map((key) => h("kbd", { textContent: key })) : ["Off"]),
    );
  };
  let saved = "";
  const commit = bind(
    store,
    settings,
    binding,
    (value) => {
      saved = value;
      show(value);
    },
    error,
  );

  let held: string[] | null = null;
  const pause = (paused: boolean) => void binding.pause(paused).catch(() => undefined);
  const stop = () => {
    const recording = held !== null;
    held = null;
    window.removeEventListener("keydown", down, true);
    window.removeEventListener("keyup", up, true);
    record.textContent = "Record";
    record.setAttribute("aria-pressed", "false");
    if (recording) pause(false);
  };
  const down = (event: KeyboardEvent) => {
    event.preventDefault();
    event.stopPropagation();
    if (!held || !event.code || held.includes(event.code)) return;
    held.push(event.code);
    show(chordName(held));
  };
  const up = (event: KeyboardEvent) => {
    event.preventDefault();
    event.stopPropagation();
    // Only a key that is part of the shortcut ends it: Enter can start
    // recording and be let go a moment later.
    if (!held?.includes(event.code)) return;
    const value = chordName(held);
    stop();
    void commit(value);
  };
  record.addEventListener("click", () => {
    if (held) {
      stop();
      show(saved);
      return;
    }
    held = [];
    error.hidden = true;
    record.textContent = "Press keys…";
    record.setAttribute("aria-pressed", "true");
    keys.replaceChildren("Listening…");
    window.addEventListener("keydown", down, true);
    window.addEventListener("keyup", up, true);
    pause(true);
  });
  // Clicking elsewhere or leaving the window gives up.
  record.addEventListener("blur", () => {
    if (!held) return;
    stop();
    show(saved);
  });
  clear.addEventListener("click", () => void commit(""));
  stop();
  const control = h("span", { className: "shortcut" }, [keys, record, clear]);
  return field(id, binding, control, error);
}
