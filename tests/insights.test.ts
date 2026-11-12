import { describe, expect, it } from "vitest";
import { calendar, histogram, shareList } from "../src/charts";
import { createInsightsPage } from "../src/pages/insights";
import { byText, insights, makeDeps } from "./fakes";

async function setup(data = insights()) {
  const ctx = makeDeps();
  ctx.api.getInsights.mockResolvedValue(data);
  const page = createInsightsPage(ctx.deps);
  document.body.append(page.el);
  await page.show();
  return ctx;
}

describe("insights page", () => {
  it("shows totals, activity, shares, histograms, speed and accuracy", async () => {
    await setup();
    const stats = document.querySelector(".stats.wide")!.textContent;
    expect(stats).toContain((12_345).toLocaleString());
    expect(stats).toContain("142wpm");
    expect(document.querySelectorAll(".cell")).toHaveLength(14);
    expect(document.querySelector(".streaks")!.textContent).toContain("4 days current streak");
    expect(document.querySelector(".streaks")!.textContent).toContain("12 days longest streak");
    expect(byText(document, ".share-name", "Work")).toBeTruthy();
    expect(byText(document, ".share-name", "mystery")).toBeTruthy();
    expect(document.querySelectorAll(".histogram")[0]!.children).toHaveLength(24);
    expect(document.querySelectorAll(".histogram")[1]!.children).toHaveLength(7);
    expect(document.body.textContent).toContain("0.8 s");
    expect(document.body.textContent).toContain("1.9 s");
    expect(document.body.textContent).toContain("42%");
    expect(document.querySelector(".figure")!.textContent).toBe("93%");
    expect(document.body.textContent).toContain("from 12 reviewed dictations");
  });

  it("explains a missing accuracy score and links to Learning", async () => {
    const ctx = await setup(insights({ accuracy: null }));
    byText<HTMLButtonElement>(document, "button", "Open Learning").click();
    expect(ctx.navigate).toHaveBeenCalledWith("learning");
  });

  it("has an empty state before the first dictation", async () => {
    await setup(insights({ dictations: 0 }));
    expect(document.querySelector(".empty-title")!.textContent).toBe("Nothing to show yet");
    expect(document.querySelector(".calendar")).toBeNull();
  });
});

describe("charts", () => {
  it("draws a week-column calendar with month labels and tooltips", () => {
    const days = Array.from({ length: 21 }, (_, i) => ({
      date: new Date(Date.UTC(2026, 9, 19 + i)).toISOString().slice(0, 10),
      words: i === 0 ? 1 : 0,
      band: i === 0 ? 1 : 0,
    }));
    const el = calendar(days);
    const svg = el.querySelector("svg")!;
    expect(svg.getAttribute("aria-label")).toBe("1 word over the last 3 weeks");
    const cells = el.querySelectorAll("rect");
    expect(cells[0]!.getAttribute("class")).toBe("cell band-1");
    expect(cells[0]!.getAttribute("y")).toBe(cells[7]!.getAttribute("y"));
    expect(Number(cells[7]!.getAttribute("x"))).toBeGreaterThan(
      Number(cells[0]!.getAttribute("x")),
    );
    expect(cells[0]!.querySelector("title")!.textContent).toMatch(/1 word$/);
    const labels = [...el.querySelectorAll("text")].map((t) => t.textContent);
    expect(labels).toEqual(["Mon", "Wed", "Fri", "Oct", "Nov"]);
    expect(el.querySelectorAll(".legend .swatch")).toHaveLength(5);
  });

  it("draws share bars and histograms that cope with zeros", () => {
    const list = shareList([{ name: "Slack", words: 10, dictations: 1, share: 0.25 }]);
    expect(list.querySelector<HTMLElement>(".share-fill")!.style.width).toBe("25%");
    const chart = histogram([0, 0], ["a", "b"], 2, "dictation");
    const bars = chart.querySelectorAll<HTMLElement>(".bar");
    expect(bars[0]!.style.height).toBe("0%");
    expect([...chart.querySelectorAll(".tick")].map((t) => t.textContent)).toEqual(["a", ""]);
    expect(chart.querySelector(".column")!.getAttribute("title")).toBe("a: 0 dictations");
    expect(
      histogram([2, 4], ["x", "y"], 1, "dictation").querySelectorAll<HTMLElement>(".bar")[0]!.style
        .height,
    ).toBe("50%");
  });
});
