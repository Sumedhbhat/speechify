// Home: greeting, headline numbers, getting-started checklist, history.

import type { Insights, Settings, Status } from "../api";
import type { Deps, Page } from "../deps";
import { formatNumber, greeting, plural } from "../format";
import { h, icon, type Child } from "../ui";
import { createHistory } from "./history";

function stat(label: string, value: string, unit = ""): HTMLElement {
  return h("div", { className: "stat" }, [
    h("dt", { textContent: label }),
    h("dd", {}, [value, unit && h("span", { className: "unit", textContent: unit })]),
  ]);
}

function stats(insights: Insights): HTMLElement {
  return h("dl", { className: "stats" }, [
    stat("Words this week", formatNumber(insights.week_words)),
    stat("Speaking speed", formatNumber(insights.wpm), "wpm"),
    stat("Streak", formatNumber(insights.streak), insights.streak === 1 ? "day" : "days"),
    stat("Time saved", formatNumber(insights.minutes_saved), "min"),
  ]);
}

function step(done: boolean, title: string, body: Child[], action?: HTMLElement): HTMLElement {
  return h("li", { className: `step${done ? " done" : ""}` }, [
    h("span", { className: "step-mark", "aria-hidden": "true" }, [done ? icon("check") : ""]),
    h("div", { className: "step-body" }, [
      h("p", { className: "step-title" }, [
        title,
        h("span", { className: "sr-only", textContent: done ? " (done)" : "" }),
      ]),
      h("p", { className: "hint" }, body),
    ]),
    action,
  ]);
}

function checklist(
  status: Status,
  settings: Settings,
  navigate: (page: string) => void,
): HTMLElement {
  const go = (page: string, label: string) =>
    h("button", {
      type: "button",
      className: "btn",
      textContent: label,
      onclick: () => navigate(page),
    });
  const key = h("kbd", { textContent: settings.hotkeys.push_to_talk });
  const shortcut = status.shortcuts
    ? step(true, "Hold your shortcut and speak", [
        "Hold ",
        key,
        " in any app, speak, and let go. The text is typed where your cursor is.",
      ])
    : status.wayland
      ? step(false, "Start dictating from the bar", [
          "Global shortcuts are not available on Wayland. Click the small bar at the bottom of the screen, or bind ",
          h("code", { textContent: "speechify --toggle" }),
          " to a shortcut in your system settings.",
        ])
      : step(false, "Let Speechify see your shortcut", [
          "Allow Speechify under System Settings › Privacy & Security › Accessibility, then restart it. Until then, click the small bar at the bottom of the screen to dictate.",
        ]);
  const items = [
    status.speech_ready
      ? step(true, "Speech model ready", ["Everything you say is recognised on this computer."])
      : step(
          false,
          "Get a speech model",
          ["Download one to start dictating. It stays on this computer."],
          go("models", "Choose a model"),
        ),
    status.rewrite_reachable
      ? step(true, "Language model connected", [
          "Longer dictations are tidied by the model before they are typed.",
        ])
      : step(
          false,
          "Connect a language model (optional)",
          [
            "A local model, through Ollama or any OpenAI-compatible server, tidies longer dictations more naturally than rules alone.",
          ],
          go("models", "Set up"),
        ),
    shortcut,
  ];
  const ready = status.speech_ready && status.rewrite_reachable;
  return h("section", { className: "section checklist" }, [
    h("h2", { textContent: ready ? "Ready when you are" : "Getting started" }),
    h("ol", { className: "steps" }, items),
  ]);
}

export function createHomePage(deps: Deps): Page {
  const { api, settings } = deps;
  const top = h("div");
  const history = createHistory(deps);
  const el = h("div", { className: "page home" }, [top, history.el]);

  return {
    el,
    async show() {
      const [insights, status, current] = await Promise.all([
        api.getInsights(),
        api.getStatus(),
        settings.load(),
      ]);
      top.replaceChildren(
        h("header", { className: "page-header" }, [
          h("h1", { textContent: greeting(new Date().getHours()) }),
          h("p", {
            className: "lede",
            textContent:
              insights.today_words > 0
                ? `You have dictated ${plural(insights.today_words, "word")} today.`
                : "Hold your shortcut anywhere and start talking.",
          }),
        ]),
        stats(insights),
        checklist(status, current, deps.navigate),
      );
      await history.refresh();
    },
  };
}
