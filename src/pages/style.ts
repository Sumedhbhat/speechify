// Style: tone per kind of app, spelling and numbers, clean-up, app rules,
// and a playground that formats typed text without a microphone.

import type { Category, Rule, Settings, Style } from "../api";
import { select } from "../controls";
import type { Deps, Page } from "../deps";
import { CATEGORY_LABELS } from "../format";
import { debounce, errorText, h, iconButton, nextId, pageHeader, section, toast } from "../ui";

type StyleKey = keyof Settings["styles"];

const KINDS: { key: StyleKey; title: string; where: string }[] = [
  {
    key: "personal",
    title: "Personal messages",
    where: "WhatsApp, Telegram, Signal, Discord, Messages",
  },
  { key: "work", title: "Work messages", where: "Slack, Teams, Zoom, Jira, Linear" },
  { key: "email", title: "Email", where: "Gmail, Outlook, Thunderbird, Apple Mail" },
  {
    key: "other",
    title: "Everything else",
    where:
      "Documents, AI chats and every other app. Code editors and terminals keep your exact words.",
  },
];

const STYLES: { value: Style; label: string; example: string }[] = [
  { value: "formal", label: "Formal", example: "Hi Sam, I will be there by six. Thank you." },
  { value: "casual", label: "Casual", example: "Hey Sam, I'll be there by six. Thanks" },
  { value: "very_casual", label: "Very casual", example: "hey Sam, I'll be there by six. thanks" },
  { value: "excited", label: "Excited", example: "Hey Sam, I'll be there by six. Thanks so much!" },
];

const LEVELS: [string, string][] = [
  ["none", "None: type exactly what was heard, plus your dictionary and snippets"],
  ["light", "Light: remove filler words, apply spoken corrections, format lists and numbers"],
  ["medium", "Medium: as Light, and the language model may also tighten wordy phrasing"],
];

const SAMPLE =
  "um my plan for today is one, buy milk. two, call mom. three, pay two lakh fifty thousand rupees";

function styleCard(deps: Deps, settings: Settings, kind: (typeof KINDS)[number]): HTMLElement {
  const name = nextId("style");
  let saved = settings.styles[kind.key];
  const example = h("p", { className: "example", "aria-live": "polite" });
  const show = (value: Style) =>
    (example.textContent = STYLES.find((style) => style.value === value)!.example);
  const radios = STYLES.map((style) => {
    const input = h("input", {
      type: "radio",
      name,
      value: style.value,
      checked: style.value === saved,
      onchange: async () => {
        show(style.value);
        const error = await deps.settings.update((next) => (next.styles[kind.key] = style.value));
        if (error) {
          radios.find((radio) => radio.value === saved)!.checked = true;
          show(saved);
        } else {
          saved = style.value;
        }
      },
    });
    return input;
  });
  show(saved);
  return h("fieldset", { className: "style-card" }, [
    h("legend", { textContent: kind.title }),
    h("p", { className: "hint", textContent: kind.where }),
    h(
      "div",
      { className: "choices" },
      radios.map((input, i) =>
        h("label", { className: "choice" }, [input, h("span", { textContent: STYLES[i]!.label })]),
      ),
    ),
    example,
  ]);
}

function rulesEditor({ api }: Deps, rules: Rule[]): HTMLElement {
  const rows = h("ul", { className: "rules" });
  const error = h("p", { className: "field-error", role: "alert", hidden: true });
  const categories = Object.entries(CATEGORY_LABELS) as [Category, string][];

  const addRow = (rule: Rule) => {
    const pattern = h("input", {
      type: "text",
      value: rule.pattern,
      placeholder: "App or window name",
      "aria-label": "App name contains",
    });
    const category = h(
      "select",
      { "aria-label": "Treat as" },
      categories.map(([value, label]) =>
        h("option", { value, textContent: label, selected: value === rule.category }),
      ),
    );
    const li: HTMLElement = h("li", { className: "rule" }, [
      pattern,
      h("span", { className: "hint", textContent: "is" }),
      category,
      iconButton("trash", "Remove rule", () => li.remove()),
    ]);
    rows.append(li);
    return pattern;
  };

  const save = async () => {
    const next = [...rows.children].map((li) => ({
      pattern: li.querySelector("input")!.value.trim(),
      category: li.querySelector("select")!.value as Category,
    }));
    try {
      await api.saveRules(next.filter((rule) => rule.pattern));
      error.hidden = true;
      toast("Rules saved");
    } catch (failure) {
      error.textContent = errorText(failure);
      error.hidden = false;
    }
  };

  rules.forEach(addRow);
  return section(
    "App rules",
    "Speechify guesses the kind of app from its name or window title. Add a rule when it guesses wrong; your rules are checked first, in order.",
    [
      rows,
      error,
      h("div", { className: "row" }, [
        h("button", {
          type: "button",
          className: "btn",
          textContent: "Add rule",
          onclick: () => addRow({ pattern: "", category: "work" }).focus(),
        }),
        h("button", {
          type: "button",
          className: "btn primary",
          textContent: "Save rules",
          onclick: () => void save(),
        }),
      ]),
    ],
  );
}

function playground({ api }: Deps): { el: HTMLElement; run: () => void } {
  const input = h("textarea", { id: "try-text", rows: 4, value: SAMPLE });
  const app = h("input", {
    id: "try-app",
    type: "text",
    placeholder: "Slack",
    autocomplete: "off",
  });
  const output = h("output", { className: "try-output", htmlFor: input.id });
  let request = 0;
  const run = debounce(async () => {
    const token = ++request;
    try {
      const result = await api.tryFormat(input.value, app.value.trim());
      if (token !== request) return;
      output.classList.remove("error");
      output.textContent = result;
    } catch (failure) {
      output.classList.add("error");
      output.textContent = errorText(failure);
    }
  }, 300);
  input.addEventListener("input", () => run());
  app.addEventListener("input", () => run());
  const el = section(
    "Try it",
    "Type as if you were speaking and see what Speechify would type. No microphone needed.",
    [
      h("div", { className: "try" }, [
        h("div", { className: "stacked" }, [
          h("label", { htmlFor: input.id, textContent: "You say" }),
          input,
        ]),
        h("div", { className: "stacked" }, [
          h("label", { htmlFor: app.id, textContent: "Into this app" }),
          app,
        ]),
        h("div", { className: "stacked" }, [
          h("span", { className: "label", textContent: "Speechify types" }),
          output,
        ]),
      ]),
    ],
  );
  const now = () => {
    run();
    run.flush();
  };
  return { el, run: now };
}

export function createStylePage(deps: Deps): Page {
  const body = h("div");
  const el = h("div", { className: "page style" }, [
    pageHeader("Style", "How your words are written, depending on where they go."),
    body,
  ]);
  return {
    el,
    async show() {
      const [settings, rules] = await Promise.all([deps.settings.load(), deps.api.rules()]);
      const store = deps.settings;
      const tryIt = playground(deps);
      body.replaceChildren(
        h(
          "div",
          { className: "style-grid" },
          KINDS.map((kind) => styleCard(deps, settings, kind)),
        ),
        section("Writing", "", [
          select(store, settings, {
            label: "Spelling",
            options: [
              ["indian", "Indian and British (colour, organise)"],
              ["american", "American (color, organize)"],
            ],
            get: (s) => s.spelling,
            set: (s, v) => (s.spelling = v as Settings["spelling"]),
          }),
          select(store, settings, {
            label: "Large numbers",
            options: [
              ["indian", "Indian (2,50,000)"],
              ["international", "International (250,000)"],
            ],
            get: (s) => s.numbers,
            set: (s, v) => (s.numbers = v as Settings["numbers"]),
          }),
          select(store, settings, {
            label: "Clean-up",
            hint: "None skips tone, spelling and number formatting too. Light and Medium use the built-in rules, plus the language model on longer dictations when it is on and reachable.",
            options: LEVELS,
            get: (s) => s.rewrite.level,
            set: (s, v) => (s.rewrite.level = v as Settings["rewrite"]["level"]),
          }),
        ]),
        rulesEditor(deps, rules),
        tryIt.el,
      );
      tryIt.run();
    },
  };
}
