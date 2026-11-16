// Pure formatting helpers: sizes, durations, numbers, local days.

import type { Category, Outcome } from "./api";

const DAY_MS = 86_400_000;

export function formatBytes(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  return `${Math.max(1, Math.round(bytes / 1e6))} MB`;
}

/** Milliseconds as seconds, e.g. "0.8 s". */
export function formatSeconds(ms: number): string {
  return `${(ms / 1000).toFixed(1)} s`;
}

export function formatNumber(value: number): string {
  return Math.round(value).toLocaleString();
}

export function percent(share: number): string {
  return `${Math.round(share * 100)}%`;
}

export function plural(count: number, one: string, many = `${one}s`): string {
  return `${formatNumber(count)} ${count === 1 ? one : many}`;
}

/** The local calendar day (YYYY-MM-DD) of a timestamp taken at `offsetMin`. */
export function dayKey(unixSeconds: number, offsetMin: number): string {
  return new Date(unixSeconds * 1000 + offsetMin * 60_000).toISOString().slice(0, 10);
}

/** Today's local day key for this machine. */
export function todayKey(now: Date = new Date()): string {
  return dayKey(now.getTime() / 1000, -now.getTimezoneOffset());
}

function keyDate(key: string): Date {
  return new Date(`${key}T00:00:00Z`);
}

/** "Today", "Yesterday" or a written-out date ("Monday, 3 November"). */
export function dayLabel(key: string, today: string): string {
  if (key === today) return "Today";
  const yesterday = new Date(keyDate(today).getTime() - DAY_MS).toISOString().slice(0, 10);
  if (key === yesterday) return "Yesterday";
  const sameYear = key.slice(0, 4) === today.slice(0, 4);
  return keyDate(key).toLocaleDateString(undefined, {
    timeZone: "UTC",
    weekday: "long",
    day: "numeric",
    month: "long",
    year: sameYear ? undefined : "numeric",
  });
}

/** Short date for tooltips, e.g. "3 Nov 2026". */
export function shortDate(key: string): string {
  return keyDate(key).toLocaleDateString(undefined, {
    timeZone: "UTC",
    day: "numeric",
    month: "short",
    year: "numeric",
  });
}

export function monthName(key: string): string {
  return keyDate(key).toLocaleDateString(undefined, { timeZone: "UTC", month: "short" });
}

/** Wall-clock time of a dictation in the zone it was made in. */
export function timeOfDay(unixSeconds: number, offsetMin: number): string {
  return new Date(unixSeconds * 1000 + offsetMin * 60_000).toLocaleTimeString(undefined, {
    timeZone: "UTC",
    hour: "numeric",
    minute: "2-digit",
  });
}

/** Local date and time of a Unix timestamp on this machine. */
export function dateTime(unixSeconds: number): string {
  return new Date(unixSeconds * 1000).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

/** "12 am", "3 pm"... for the hour pickers and histograms. */
export function hourLabel(hour: number): string {
  const h12 = hour % 12 === 0 ? 12 : hour % 12;
  return `${h12} ${hour < 12 ? "am" : "pm"}`;
}

export function greeting(hour: number): string {
  if (hour < 5) return "Working late";
  if (hour < 12) return "Good morning";
  if (hour < 18) return "Good afternoon";
  return "Good evening";
}

export const CATEGORY_LABELS: Record<Category, string> = {
  personal: "Personal",
  work: "Work",
  email: "Email",
  code: "Code",
  terminal: "Terminal",
  assistant: "AI assistant",
  document: "Document",
  other: "Other",
};

export function categoryLabel(name: string): string {
  return CATEGORY_LABELS[name as Category] ?? name;
}

export const OUTCOME_LABELS: Record<Outcome, string> = {
  pasted: "Typed into the app",
  copied: "Copied (no text box was focused)",
  cancelled: "Cancelled",
  failed: "Failed",
};
