// Insights: totals, activity calendar, where and when you dictate, speed.

import type { Insights } from "../api";
import { calendar, histogram, shareList } from "../charts";
import type { Deps, Page } from "../deps";
import { categoryLabel, formatNumber, formatSeconds, hourLabel, percent, plural } from "../format";
import { emptyState, h, pageHeader, section } from "../ui";

const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

function totals(data: Insights): HTMLElement {
  const item = (label: string, value: number, unit = "") =>
    h("div", { className: "stat" }, [
      h("dt", { textContent: label }),
      h("dd", {}, [
        formatNumber(value),
        unit && h("span", { className: "unit", textContent: unit }),
      ]),
    ]);
  return h("dl", { className: "stats wide" }, [
    item("Words", data.words),
    item("Dictations", data.dictations),
    item("Spoken", data.minutes_spoken, "min"),
    item("Speed", data.wpm, "wpm"),
    item("Time saved", data.minutes_saved, "min"),
    item("Auto-fixes", data.fixes),
  ]);
}

function accuracy(data: Insights, navigate: (page: string) => void): HTMLElement {
  if (data.accuracy === null) {
    return emptyState(
      "No accuracy score yet",
      "Turn on the nightly teacher and it will re-check your dictations with a bigger model, then show how often the everyday model got them right.",
      [
        h("button", {
          type: "button",
          className: "btn",
          textContent: "Open Learning",
          onclick: () => navigate("learning"),
        }),
      ],
    );
  }
  return h("p", { className: "big-figure" }, [
    h("span", { className: "figure", textContent: percent(data.accuracy) }),
    h("span", {
      className: "hint",
      textContent: `of words right, from ${plural(data.reviewed, "reviewed dictation")}`,
    }),
  ]);
}

function render(data: Insights, navigate: (page: string) => void): HTMLElement[] {
  if (data.dictations === 0) {
    return [
      emptyState(
        "Nothing to show yet",
        "Your insights fill in as you dictate: how much you have said, where you say it, and how quickly it is typed.",
      ),
    ];
  }
  const hours = Array.from({ length: 24 }, (_, hour) => hourLabel(hour));
  return [
    totals(data),
    section("Activity", "Words dictated each day.", [
      calendar(data.days),
      h("p", { className: "streaks" }, [
        h("span", {}, [
          h("strong", { textContent: plural(data.streak, "day") }),
          " current streak",
        ]),
        h("span", {}, [
          h("strong", { textContent: plural(data.longest_streak, "day") }),
          " longest streak",
        ]),
        h("span", {}, [h("strong", { textContent: plural(data.active_days, "day") }), " active"]),
      ]),
    ]),
    h("div", { className: "columns" }, [
      section("Apps", "Where your words go.", [shareList(data.apps)]),
      section("Kinds of writing", "How each app was treated.", [
        shareList(data.categories, categoryLabel),
      ]),
    ]),
    h("div", { className: "columns" }, [
      section("Time of day", "Words by hour.", [histogram(data.hours, hours, 6, "word")]),
      section("Day of week", "Words by weekday.", [histogram(data.weekdays, WEEKDAYS, 1, "word")]),
    ]),
    h("div", { className: "columns" }, [
      section("Speed", "From letting go of the shortcut to text appearing.", [
        h("dl", { className: "stats" }, [
          h("div", { className: "stat" }, [
            h("dt", { textContent: "Typical" }),
            h("dd", { textContent: formatSeconds(data.median_ms) }),
          ]),
          h("div", { className: "stat" }, [
            h("dt", { textContent: "Slowest 5%" }),
            h("dd", { textContent: formatSeconds(data.p95_ms) }),
          ]),
          h("div", { className: "stat" }, [
            h("dt", { textContent: "Tidied by language model" }),
            h("dd", { textContent: percent(data.rewritten_share) }),
          ]),
        ]),
      ]),
      section("Accuracy", "Measured by the nightly teacher.", [accuracy(data, navigate)]),
    ]),
  ];
}

export function createInsightsPage({ api, navigate }: Deps): Page {
  const body = h("div");
  const el = h("div", { className: "page insights" }, [
    pageHeader("Insights", "A quiet record of how you use your voice."),
    body,
  ]);
  return {
    el,
    async show() {
      body.replaceChildren(...render(await api.getInsights(), navigate));
    },
  };
}
