// Hand-built charts: activity calendar (SVG), share bars and histograms (DOM).

import type { Day, Share } from "./api";
import { formatNumber, monthName, percent, plural, shortDate } from "./format";
import { h } from "./ui";

const SVG = "http://www.w3.org/2000/svg";
const CELL = 11;
const GAP = 3;
const TOP = 16;
const LEFT = 26;

function svg<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number>) {
  const el = document.createElementNS(SVG, tag);
  for (const [key, value] of Object.entries(attrs)) el.setAttribute(key, String(value));
  return el;
}

/** GitHub-style grid: one column per week, Monday at the top. */
export function calendar(days: Day[]): HTMLElement {
  const weeks = Math.ceil(days.length / 7);
  const width = LEFT + weeks * (CELL + GAP);
  const height = TOP + 7 * (CELL + GAP);
  const total = days.reduce((sum, day) => sum + day.words, 0);
  const chart = svg("svg", {
    viewBox: `0 0 ${width} ${height}`,
    width,
    height,
    role: "img",
    "aria-label": `${plural(total, "word")} over the last ${plural(weeks, "week")}`,
  });
  ["Mon", "Wed", "Fri"].forEach((label, i) => {
    const text = svg("text", { x: 0, y: TOP + i * 2 * (CELL + GAP) + CELL - 2, class: "axis" });
    text.textContent = label;
    chart.append(text);
  });
  let month = "";
  days.forEach((day, i) => {
    const week = Math.floor(i / 7);
    const x = LEFT + week * (CELL + GAP);
    const name = monthName(day.date);
    if (i % 7 === 0 && name !== month) {
      month = name;
      const text = svg("text", { x, y: TOP - 5, class: "axis" });
      text.textContent = name;
      chart.append(text);
    }
    const cell = svg("rect", {
      x,
      y: TOP + (i % 7) * (CELL + GAP),
      width: CELL,
      height: CELL,
      rx: 2,
      class: `cell band-${day.band}`,
      "data-date": day.date,
    });
    const title = svg("title", {});
    title.textContent = `${shortDate(day.date)}: ${plural(day.words, "word")}`;
    cell.append(title);
    chart.append(cell);
  });
  const legend = h("div", { className: "legend", "aria-hidden": "true" }, [
    "Less",
    ...[0, 1, 2, 3, 4].map((band) => h("span", { className: `swatch band-${band}` })),
    "More",
  ]);
  return h("figure", { className: "calendar" }, [
    h("div", { className: "calendar-scroll" }, [chart]),
    legend,
  ]);
}

/** Ranked list of names with a proportional bar for each share. */
export function shareList(
  shares: Share[],
  label: (name: string) => string = (name) => name,
): HTMLElement {
  return h(
    "ol",
    { className: "shares" },
    shares.map((share) =>
      h("li", {}, [
        h("span", { className: "share-name", textContent: label(share.name) }),
        h("span", { className: "share-words", textContent: `${formatNumber(share.words)} words` }),
        h("span", { className: "share-track", "aria-hidden": "true" }, [
          h("span", { className: "share-fill", style: `width: ${percent(share.share)}` }),
        ]),
        h("span", { className: "sr-only", textContent: `${percent(share.share)} of words` }),
      ]),
    ),
  );
}

/** Vertical bars with a label under some of them. */
export function histogram(
  values: number[],
  labels: string[],
  every: number,
  unit: string,
): HTMLElement {
  const max = Math.max(1, ...values);
  return h(
    "div",
    { className: "histogram", role: "list" },
    values.map((value, i) =>
      h(
        "div",
        { className: "column", role: "listitem", title: `${labels[i]}: ${plural(value, unit)}` },
        [
          h("span", { className: "bar", style: `height: ${(value / max) * 100}%` }),
          h("span", {
            className: "tick",
            "aria-hidden": "true",
            textContent: i % every === 0 ? labels[i] : "",
          }),
          h("span", { className: "sr-only", textContent: `${labels[i]}: ${plural(value, unit)}` }),
        ],
      ),
    ),
  );
}
