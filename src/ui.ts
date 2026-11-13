// Tiny DOM helpers shared by every page: element builder, icons, toasts.

export type Child = Node | string | false | null | undefined;
export type Props = Record<string, unknown>;

/**
 * Builds an element. `on*` function props become event listeners, props with a
 * dash (aria-*, data-*) and `role` become attributes, everything else is set
 * as a DOM property (className, textContent, value, disabled, ...).
 */
export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Props = {},
  children: Child[] = [],
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value === undefined) continue;
    if (key.startsWith("on") && typeof value === "function") {
      el.addEventListener(key.slice(2), value as EventListener);
    } else if (key.includes("-") || key === "role") {
      el.setAttribute(key, String(value));
    } else {
      (el as unknown as Record<string, unknown>)[key] = value;
    }
  }
  el.append(...children.filter((child): child is Node | string => !!child));
  return el;
}

const ICONS = {
  home: '<path d="M4 11l8-6 8 6v8a1 1 0 0 1-1 1h-4v-6H9v6H5a1 1 0 0 1-1-1z"/>',
  insights: '<path d="M5 20V10M12 20V4M19 20v-7"/>',
  dictionary:
    '<path d="M5 4h11a3 3 0 0 1 3 3v13H8a3 3 0 0 1-3-3z"/><path d="M5 17a3 3 0 0 1 3-3h11"/>',
  snippets: '<path d="M8 6l-5 6 5 6M16 6l5 6-5 6"/>',
  style: '<path d="M4 20l4-1L19 8l-3-3L5 16z"/><path d="M14 7l3 3"/>',
  notes: '<path d="M6 3h9l4 4v14H6z"/><path d="M9 12h7M9 16h5"/>',
  models: '<rect x="4" y="4" width="16" height="16" rx="2"/><path d="M9 9h6v6H9z"/>',
  learning: '<path d="M12 3a6 6 0 0 0-3.5 10.9V17h7v-3.1A6 6 0 0 0 12 3zM10 21h4"/>',
  settings:
    '<circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M4.2 4.2l2.1 2.1M17.7 17.7l2.1 2.1M2 12h3M19 12h3M4.2 19.8l2.1-2.1M17.7 6.3l2.1-2.1"/>',
  mic: '<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 11a7 7 0 0 0 14 0M12 18v3"/>',
  close: '<path d="M6 6l12 12M18 6L6 18"/>',
  check: '<path d="M5 12.5l4.5 4.5L19 7"/>',
  stop: '<rect x="7" y="7" width="10" height="10" rx="2"/>',
  copy: '<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V5a1 1 0 0 1 1-1h9"/>',
  play: '<path d="M8 5v14l11-7z"/>',
  flag: '<path d="M5 21V4h11l-2 4 2 4H5"/>',
  edit: '<path d="M4 20h4L19 9l-4-4L4 16z"/>',
  trash: '<path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/>',
  chevron: '<path d="M9 6l6 6-6 6"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  download: '<path d="M12 4v11M7 10l5 5 5-5M5 20h14"/>',
} as const;

export type IconName = keyof typeof ICONS;

/** An inline SVG icon (decorative; pair it with a text label or aria-label). */
export function icon(name: IconName): HTMLSpanElement {
  const span = h("span", { className: "icon", "aria-hidden": "true" });
  span.innerHTML = `<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">${ICONS[name]}</svg>`;
  return span;
}

/** A button with an icon and a visible or screen-reader-only label. */
export function iconButton(
  name: IconName,
  label: string,
  onclick: () => void,
  props: Props = {},
): HTMLButtonElement {
  return h(
    "button",
    { type: "button", className: "icon-btn", "aria-label": label, title: label, onclick, ...props },
    [icon(name)],
  );
}

/** Turns a rejected command into the sentence to show. */
export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Something went wrong.";
}

function region(id: string, className: string): HTMLElement {
  const existing = document.getElementById(id);
  if (existing) return existing;
  const el = h("div", { id, className, role: "status", "aria-live": "polite" });
  document.body.append(el);
  return el;
}

/** Shows a short message in the corner; errors stay a little longer. */
export function toast(message: string, kind: "info" | "error" = "info"): void {
  const item = h("div", { className: `toast ${kind}`, textContent: message });
  if (kind === "error") item.setAttribute("role", "alert");
  region("toasts", "toasts").append(item);
  setTimeout(() => item.remove(), kind === "error" ? 6000 : 3500);
}

let savedTimer: ReturnType<typeof setTimeout> | undefined;

/** Unobtrusive "Saved" confirmation for controls that save on change. */
export function flashSaved(): void {
  const el = region("saved", "saved-flash");
  el.textContent = "Saved";
  el.classList.add("show");
  clearTimeout(savedTimer);
  savedTimer = setTimeout(() => el.classList.remove("show"), 1500);
}

export interface Debounced<A extends unknown[]> {
  (...args: A): void;
  /** Runs a pending call right away. */
  flush(): void;
}

export function debounce<A extends unknown[]>(fn: (...args: A) => void, ms: number): Debounced<A> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let pending: A | null = null;
  const run = () => {
    clearTimeout(timer);
    const args = pending;
    pending = null;
    if (args) fn(...args);
  };
  const debounced = (...args: A) => {
    pending = args;
    clearTimeout(timer);
    timer = setTimeout(run, ms);
  };
  debounced.flush = run;
  return debounced;
}

/**
 * Makes a destructive button ask once: the first press arms it and changes its
 * label, a second press within a few seconds runs the action.
 */
export function confirmButton(
  label: string,
  confirmLabel: string,
  action: () => void,
): HTMLButtonElement {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const reset = () => {
    clearTimeout(timer);
    button.textContent = label;
    button.classList.remove("armed");
  };
  const button = h("button", {
    type: "button",
    className: "btn danger-quiet",
    textContent: label,
    onclick: () => {
      if (button.classList.contains("armed")) {
        reset();
        action();
        return;
      }
      button.textContent = confirmLabel;
      button.classList.add("armed");
      timer = setTimeout(reset, 4000);
    },
  });
  return button;
}

/** A page section with a serif heading and optional one-line description. */
export function section(title: string, description: string, children: Child[]): HTMLElement {
  return h("section", { className: "section" }, [
    h("h2", { textContent: title }),
    description && h("p", { className: "lede", textContent: description }),
    ...children,
  ]);
}

/** The heading block at the top of every page. */
export function pageHeader(title: string, description: string): HTMLElement {
  return h("header", { className: "page-header" }, [
    h("h1", { textContent: title }),
    h("p", { className: "lede", textContent: description }),
  ]);
}

export function emptyState(title: string, body: string, children: Child[] = []): HTMLElement {
  return h("div", { className: "empty" }, [
    h("p", { className: "empty-title", textContent: title }),
    h("p", { textContent: body }),
    ...children,
  ]);
}

let uid = 0;
/** A unique id for pairing labels with inputs. */
export function nextId(prefix: string): string {
  uid += 1;
  return `${prefix}-${uid}`;
}
