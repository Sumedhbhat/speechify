import { describe, expect, it } from "vitest";
import {
  categoryLabel,
  dateTime,
  dayKey,
  dayLabel,
  formatBytes,
  formatNumber,
  formatSeconds,
  greeting,
  hourLabel,
  monthName,
  percent,
  plural,
  shortDate,
  timeOfDay,
  todayKey,
} from "../src/format";

describe("format", () => {
  it("formats sizes, durations, numbers and shares", () => {
    expect(formatBytes(1_600_000_000)).toBe("1.6 GB");
    expect(formatBytes(75_000_000)).toBe("75 MB");
    expect(formatBytes(10)).toBe("1 MB");
    expect(formatSeconds(820)).toBe("0.8 s");
    expect(formatNumber(1234.4)).toBe((1234).toLocaleString());
    expect(percent(0.426)).toBe("43%");
    expect(plural(1, "day")).toBe("1 day");
    expect(plural(2, "day")).toBe("2 days");
    expect(plural(2, "match", "matches")).toBe("2 matches");
  });

  it("computes local days from a timestamp and its offset", () => {
    const noonUtc = Date.UTC(2026, 10, 10, 20, 0) / 1000;
    expect(dayKey(noonUtc, 0)).toBe("2026-11-10");
    expect(dayKey(noonUtc, 330)).toBe("2026-11-11");
    expect(dayKey(noonUtc, -480)).toBe("2026-11-10");
    const now = new Date();
    expect(todayKey(now)).toBe(dayKey(now.getTime() / 1000, -now.getTimezoneOffset()));
    expect(todayKey()).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });

  it("labels days relative to today", () => {
    expect(dayLabel("2026-11-10", "2026-11-10")).toBe("Today");
    expect(dayLabel("2026-11-09", "2026-11-10")).toBe("Yesterday");
    expect(dayLabel("2026-11-03", "2026-11-10")).toMatch(/Tuesday/);
    expect(dayLabel("2026-11-03", "2026-11-10")).not.toMatch(/2026/);
    expect(dayLabel("2025-12-31", "2026-01-02")).toMatch(/2025/);
    expect(dayLabel("2025-12-31", "2026-01-01")).toBe("Yesterday");
  });

  it("formats dates and times", () => {
    expect(shortDate("2026-11-03")).toMatch(/2026/);
    expect(monthName("2026-11-03")).toMatch(/Nov/);
    expect(timeOfDay(Date.UTC(2026, 0, 1, 9, 5) / 1000, 0)).toMatch(/9:05/);
    expect(dateTime(Date.UTC(2026, 0, 1) / 1000)).toMatch(/202[56]/);
  });

  it("names hours, greetings and categories", () => {
    expect(hourLabel(0)).toBe("12 am");
    expect(hourLabel(9)).toBe("9 am");
    expect(hourLabel(12)).toBe("12 pm");
    expect(hourLabel(15)).toBe("3 pm");
    expect(greeting(2)).toBe("Working late");
    expect(greeting(8)).toBe("Good morning");
    expect(greeting(13)).toBe("Good afternoon");
    expect(greeting(20)).toBe("Good evening");
    expect(categoryLabel("assistant")).toBe("AI assistant");
    expect(categoryLabel("custom")).toBe("custom");
  });
});
