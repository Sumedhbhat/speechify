import { afterEach, vi } from "vitest";

afterEach(() => {
  vi.useRealTimers();
  document.body.replaceChildren();
  delete document.documentElement.dataset.theme;
});
