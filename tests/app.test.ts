import { describe, expect, it, vi } from "vitest";
import { applyTheme, createApp } from "../src/app";
import { byText, fakeApi, fakeEvents, model, toasts, settings, settle, status } from "./fakes";

function setup(api = fakeApi()) {
  const { events, emit } = fakeEvents();
  const root = document.createElement("div");
  document.body.append(root);
  const app = createApp(root, { api, events });
  return { api, emit, root, app };
}

const current = (root: HTMLElement) => root.querySelector("[aria-current=page]")!.textContent;
const statusLine = (root: HTMLElement) => root.querySelector(".status-line")!.textContent;

describe("app shell", () => {
  it("shows the sidebar and opens Home", async () => {
    const { root } = setup();
    await settle();
    expect(root.querySelector(".brand")!.textContent).toBe("Speechify");
    expect(root.querySelectorAll(".nav-link").length).toBe(9);
    expect(current(root)).toBe("Home");
    expect(root.querySelector(".page.home")).not.toBeNull();
    expect(statusLine(root)).toBe("Ready · Whisper turbo on vulkan");
    expect(root.querySelector<HTMLElement>(".status-dot")!.dataset.state).toBe("ready");
  });

  it("switches pages from the nav and from navigate events, building each page once", async () => {
    const { root, emit, api } = setup();
    await settle();
    const home = root.querySelector(".page.home");
    for (const link of root.querySelectorAll<HTMLButtonElement>(".nav-link")) {
      link.click();
      await settle();
      expect(current(root)).toBe(link.textContent);
    }
    expect(root.querySelector(".page.settings")).not.toBeNull();
    emit("navigate", "models");
    await settle();
    expect(current(root)).toBe("Models");
    expect(root.querySelectorAll("[aria-current=page]").length).toBe(1);
    emit("navigate", "nowhere");
    expect(current(root)).toBe("Home");
    expect(root.querySelector(".page.home")).toBe(home);
    // Home on start, Home and Insights from the nav, Home again for the unknown page.
    expect(api.getInsights).toHaveBeenCalledTimes(4);
  });

  it("reports a page that fails to load", async () => {
    const api = fakeApi();
    api.getInsights.mockRejectedValueOnce("The database is locked.");
    setup(api);
    await settle();
    // A lasting message with a way to retry, rather than a passing toast.
    expect(document.querySelector(".empty-title")!.textContent).toBe("This page did not load");
    expect(document.body.textContent).toContain("The database is locked.");
    byText<HTMLButtonElement>(document, "button", "Try again").click();
    await settle();
    expect(document.body.textContent).not.toContain("This page did not load");
    expect(document.querySelector(".page.home .checklist")).not.toBeNull();
  });

  it("does not cover a page the user has moved on to with an old failure", async () => {
    const api = fakeApi();
    let fail: (reason: unknown) => void = () => {};
    api.getInsights.mockReturnValueOnce(new Promise((_, reject) => (fail = reject)));
    setup(api);
    document.querySelector<HTMLButtonElement>('[data-page="notes"]')!.click();
    fail("The database is locked.");
    await settle();
    expect(document.querySelector(".page.notes")).not.toBeNull();
    expect(document.body.textContent).not.toContain("This page did not load");
  });

  it("describes every speech state in the status line", async () => {
    vi.useFakeTimers();
    const api = fakeApi();
    const { root } = setup(api);
    await settle();

    api.getStatus.mockResolvedValue(status({ speech_ready: false }));
    vi.advanceTimersByTime(15_000);
    await settle();
    expect(statusLine(root)).toBe("Loading Whisper turbo");
    expect(root.querySelector<HTMLElement>(".status-dot")!.dataset.state).toBe("busy");

    api.models.mockResolvedValue([model({ active: false })]);
    vi.advanceTimersByTime(15_000);
    await settle();
    expect(statusLine(root)).toBe("Choose a speech model");

    api.getStatus.mockResolvedValue(status({ speech_error: "The model file is damaged." }));
    vi.advanceTimersByTime(15_000);
    await settle();
    expect(statusLine(root)).toBe("The model file is damaged.");

    api.models.mockResolvedValue([model()]);
    api.getStatus.mockResolvedValue(status({ speech_device: "" }));
    vi.advanceTimersByTime(15_000);
    await settle();
    expect(statusLine(root)).toBe("Ready · Whisper turbo");

    api.getStatus.mockRejectedValue("Speechify is not responding.");
    vi.advanceTimersByTime(15_000);
    await settle();
    expect(root.querySelector<HTMLElement>(".status-dot")!.dataset.state).toBe("error");
    expect(statusLine(root)).toBe("Speechify is not responding.");
  });

  it("applies the saved theme and follows changes", async () => {
    const api = fakeApi();
    api.getSettings.mockResolvedValue({ ...settings(), theme: "dark" });
    const { app } = setup(api);
    await settle();
    expect(document.documentElement.dataset.theme).toBe("dark");
    app.navigate("settings");
    await settle();
    const theme = [...document.querySelectorAll("select")].find((s) =>
      s.querySelector("option[value=light]"),
    )!;
    theme.value = "system";
    theme.dispatchEvent(new Event("change"));
    await settle();
    expect(document.documentElement.dataset.theme).toBeUndefined();
    expect(document.getElementById("saved")!.textContent).toBe("Saved");
  });

  it("shows an error toast when settings cannot be read or saved", async () => {
    const api = fakeApi();
    api.getSettings.mockRejectedValue("Settings are unreadable.");
    setup(api);
    await settle();
    expect(toasts()).toContain("Settings are unreadable.");
  });

  it("toasts a rejected save", async () => {
    const api = fakeApi();
    api.saveSettings.mockRejectedValue("Not allowed.");
    const { app } = setup(api);
    app.navigate("settings");
    await settle();
    const sounds = document.querySelector<HTMLInputElement>("input[role=switch]")!;
    sounds.checked = !sounds.checked;
    sounds.dispatchEvent(new Event("change"));
    await settle();
    expect(toasts()).toContain("Not allowed.");
  });
});

describe("applyTheme", () => {
  it("sets light and dark and clears for system", () => {
    applyTheme("light");
    expect(document.documentElement.dataset.theme).toBe("light");
    applyTheme("system");
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });
});
