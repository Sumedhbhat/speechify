// The main window shell: sidebar navigation, live status line, page host.

import type { Deps, Page } from "./deps";
import { createSettingsStore } from "./settings";
import { errorText, flashSaved, h, icon, toast, type IconName } from "./ui";
import { createHomePage } from "./pages/home";
import { createInsightsPage } from "./pages/insights";
import { createDictionaryPage } from "./pages/dictionary";
import { createSnippetsPage } from "./pages/snippets";
import { createStylePage } from "./pages/style";
import { createNotesPage } from "./pages/notes";
import { createModelsPage } from "./pages/models";
import { createLearningPage } from "./pages/learning";
import { createSettingsPage } from "./pages/settings";

type PageId =
  | "home"
  | "insights"
  | "dictionary"
  | "snippets"
  | "style"
  | "notes"
  | "models"
  | "learning"
  | "settings";

const PAGES: { id: PageId; label: string; create: (deps: Deps) => Page }[] = [
  { id: "home", label: "Home", create: createHomePage },
  { id: "insights", label: "Insights", create: createInsightsPage },
  { id: "dictionary", label: "Dictionary", create: createDictionaryPage },
  { id: "snippets", label: "Snippets", create: createSnippetsPage },
  { id: "style", label: "Style", create: createStylePage },
  { id: "notes", label: "Notes", create: createNotesPage },
  { id: "models", label: "Models", create: createModelsPage },
  { id: "learning", label: "Learning", create: createLearningPage },
  { id: "settings", label: "Settings", create: createSettingsPage },
];

const STATUS_MS = 15_000;

export function applyTheme(theme: string): void {
  if (theme === "light" || theme === "dark") document.documentElement.dataset.theme = theme;
  else delete document.documentElement.dataset.theme;
}

export function createApp(root: HTMLElement, base: Pick<Deps, "api" | "events">) {
  const settings = createSettingsStore(base.api, flashSaved, (message) => toast(message, "error"));
  const deps: Deps = { ...base, settings, navigate };
  const pages = new Map<PageId, Page>();

  const statusDot = h("span", { className: "status-dot" });
  const statusText = h("span", { textContent: "Starting" });
  const content = h("main", { className: "content", tabIndex: -1 });
  const links = PAGES.map(({ id, label }) =>
    h(
      "button",
      { type: "button", className: "nav-link", "data-page": id, onclick: () => navigate(id) },
      [icon(id as IconName), h("span", { textContent: label })],
    ),
  );

  root.replaceChildren(
    h("div", { className: "shell" }, [
      h("aside", { className: "sidebar" }, [
        h("div", { className: "brand" }, [
          h("span", { className: "brand-mark", "aria-hidden": "true" }),
          "Speechify",
        ]),
        h("nav", { "aria-label": "Pages" }, links),
        h("p", { className: "status-line", role: "status" }, [statusDot, statusText]),
      ]),
      content,
    ]),
  );

  function navigate(target: string): void {
    const entry = PAGES.find((page) => page.id === target) ?? PAGES[0]!;
    let page = pages.get(entry.id);
    if (!page) {
      page = entry.create(deps);
      pages.set(entry.id, page);
    }
    for (const link of links) {
      if (link.dataset.page === entry.id) link.setAttribute("aria-current", "page");
      else link.removeAttribute("aria-current");
    }
    content.replaceChildren(page.el);
    content.scrollTop = 0;
    page.show().catch((error: unknown) => toast(errorText(error), "error"));
  }

  async function refreshStatus(): Promise<void> {
    try {
      const [status, models] = await Promise.all([base.api.getStatus(), base.api.models()]);
      const model = models.find((m) => m.active);
      const device = status.speech_device.split(" ")[0];
      let state = "ready";
      let line = `Ready · ${model?.name}${device ? ` on ${device}` : ""}`;
      if (status.speech_error) [state, line] = ["error", status.speech_error];
      else if (!model) [state, line] = ["busy", "Choose a speech model"];
      else if (!status.speech_ready) [state, line] = ["busy", `Loading ${model.name}`];
      statusDot.dataset.state = state;
      statusText.textContent = line;
    } catch (error) {
      statusDot.dataset.state = "error";
      statusText.textContent = errorText(error);
    }
  }

  settings.subscribe((next) => applyTheme(next.theme));
  settings.load().then(
    (loaded) => applyTheme(loaded.theme),
    (error: unknown) => toast(errorText(error), "error"),
  );
  void base.events.navigate(navigate);
  void refreshStatus();
  setInterval(() => void refreshStatus(), STATUS_MS);
  navigate("home");
  return { navigate, refreshStatus };
}
