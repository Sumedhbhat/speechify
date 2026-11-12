import { describe, expect, it, vi } from "vitest";
import { createSettingsStore } from "../src/settings";
import { fakeApi, settings } from "./fakes";

describe("settings store", () => {
  it("fetches once and reuses the result", async () => {
    const api = fakeApi();
    const store = createSettingsStore(api, vi.fn(), vi.fn());
    await store.load();
    await store.load();
    expect(api.getSettings).toHaveBeenCalledOnce();
  });

  it("retries after a failed fetch", async () => {
    const api = fakeApi();
    api.getSettings.mockRejectedValueOnce("Settings are unreadable.");
    const onError = vi.fn();
    const store = createSettingsStore(api, vi.fn(), onError);
    await expect(store.load()).rejects.toBe("Settings are unreadable.");
    expect(await store.update((s) => (s.sounds = false))).toBe("");
    expect(api.getSettings).toHaveBeenCalledTimes(2);
  });

  it("saves a changed copy and tells listeners", async () => {
    const api = fakeApi();
    const onSaved = vi.fn();
    const listener = vi.fn();
    const store = createSettingsStore(api, onSaved, vi.fn());
    store.subscribe(listener);
    const before = await store.load();
    expect(await store.update((s) => (s.theme = "dark"))).toBe("");
    expect(before.theme).toBe("system");
    expect(api.saveSettings).toHaveBeenCalledWith({ ...settings(), theme: "dark" });
    expect((await store.load()).theme).toBe("dark");
    expect(listener).toHaveBeenCalledWith(expect.objectContaining({ theme: "dark" }));
    expect(onSaved).toHaveBeenCalledOnce();
  });

  it("keeps the old settings when a save is rejected", async () => {
    const api = fakeApi();
    api.saveSettings.mockRejectedValueOnce("That shortcut is taken.");
    const onError = vi.fn();
    const store = createSettingsStore(api, vi.fn(), onError);
    expect(await store.update((s) => (s.hotkeys.cancel = "A"))).toBe("That shortcut is taken.");
    expect(onError).toHaveBeenCalledWith("That shortcut is taken.");
    expect((await store.load()).hotkeys.cancel).toBe("Escape");
  });

  it("runs quick successive changes in order", async () => {
    const api = fakeApi();
    const store = createSettingsStore(api, vi.fn(), vi.fn());
    await Promise.all([
      store.update((s) => (s.sounds = false)),
      store.update((s) => (s.press_enter = true)),
    ]);
    expect(api.saveSettings.mock.calls[1]![0]).toMatchObject({ sounds: false, press_enter: true });
  });
});
