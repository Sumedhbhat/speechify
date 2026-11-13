import { afterEach, vi } from "vitest";

// Dates and numbers render in English whatever the machine's locale, so
// expectations such as "Tuesday" or "Nov" hold everywhere.
for (const name of ["toLocaleDateString", "toLocaleTimeString", "toLocaleString"] as const) {
  const original = Date.prototype[name];
  Date.prototype[name] = function (
    this: Date,
    locales?: Intl.LocalesArgument,
    options?: Intl.DateTimeFormatOptions,
  ) {
    return original.call(this, locales ?? "en-US", options);
  };
}
const toLocaleNumber = Number.prototype.toLocaleString;
Number.prototype.toLocaleString = function (
  this: number,
  locales?: Intl.LocalesArgument,
  options?: Intl.NumberFormatOptions,
) {
  return toLocaleNumber.call(this, locales ?? "en-US", options);
};

afterEach(() => {
  vi.useRealTimers();
  document.body.replaceChildren();
  delete document.documentElement.dataset.theme;
});
