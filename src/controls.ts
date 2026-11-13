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
  const commit = bind(store, settings, binding, (value) => (input.value = value), error);
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
