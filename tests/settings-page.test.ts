import { beforeEach, describe, expect, it } from "vitest";
import { createSettingsPage } from "../src/pages/settings";
import { byLabel, change, makeDeps, settle, status } from "./fakes";

let ctx: ReturnType<typeof makeDeps>;

async function setup() {
  const page = createSettingsPage(ctx.deps);
  document.body.append(page.el);
  await page.show();
}

beforeEach(() => {
  ctx = makeDeps();
});

describe("settings page", () => {
  it("shows shortcuts with a format hint and saves them", async () => {
    await setup();
    expect(document.body.textContent).toContain("Ctrl+Meta+Space or Alt+Shift+D");
    const push = byLabel<HTMLInputElement>(document, "Push to talk");
    expect(push.value).toBe("Ctrl+Meta+Space");
    change(push, "Ctrl+Alt+Space");
    await settle();
    expect(ctx.api.saveSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({
        hotkeys: expect.objectContaining({ push_to_talk: "Ctrl+Alt+Space" }),
      }),
    );
  });

  it("shows the shell's reason when a shortcut is rejected and reverts it", async () => {
    await setup();
    ctx.api.saveSettings.mockRejectedValueOnce("“Ctrl+Nope” is not a key.");
    const cancel = byLabel<HTMLInputElement>(document, "Cancel");
    change(cancel, "Ctrl+Nope");
    await settle();
    expect(cancel.value).toBe("Escape");
    expect(cancel.closest(".field")!.querySelector(".field-error")!.textContent).toBe(
      "“Ctrl+Nope” is not a key.",
    );
  });

  it("explains shortcuts on Wayland", async () => {
    ctx.api.getStatus.mockResolvedValue(status({ wayland: true, shortcuts: false }));
    await setup();
    expect(document.body.textContent).toContain("Global shortcuts are not available on Wayland");
  });

  it("asks for Accessibility access when the keyboard cannot be read outside Wayland", async () => {
    ctx.api.getStatus.mockResolvedValue(status({ platform: "macos", shortcuts: false }));
    await setup();
    expect(document.body.textContent).toContain("Speechify cannot see your keyboard yet");
    expect(document.body.textContent).not.toContain("Wayland");
  });

  it("offers the command-line toggle when the keyboard cannot be read on Linux outside Wayland", async () => {
    ctx.api.getStatus.mockResolvedValue(
      status({ platform: "linux", wayland: false, shortcuts: false }),
    );
    await setup();
    expect(document.body.textContent).toContain(
      "Speechify cannot read the keyboard on this system",
    );
    expect(document.body.textContent).not.toContain("Accessibility");
  });

  it("saves the rest of the controls", async () => {
    await setup();
    const mic = byLabel<HTMLSelectElement>(document, "Microphone");
    expect([...mic.options].map((o) => o.textContent)).toEqual([
      "System default",
      "USB Mic",
      "Built-in",
    ]);
    change(mic, "USB Mic");
    change(byLabel<HTMLSelectElement>(document, "Language"), "hi");
    change(byLabel<HTMLInputElement>(document, "Say “press enter” to send"), true);
    change(byLabel<HTMLInputElement>(document, "Learn from my corrections"), false);
    change(byLabel<HTMLInputElement>(document, "Play sounds"), false);
    change(byLabel<HTMLInputElement>(document, "Mute other audio"), true);
    change(byLabel<HTMLInputElement>(document, "Always show the bar"), false);
    change(byLabel<HTMLInputElement>(document, "Open at login"), true);
    change(byLabel<HTMLSelectElement>(document, "History"), "day");
    change(byLabel<HTMLInputElement>(document, "Keep recordings"), false);
    change(byLabel<HTMLSelectElement>(document, "Theme"), "light");
    await settle();
    expect(ctx.api.saveSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({
        microphone: "USB Mic",
        speech: expect.objectContaining({ language: "hi" }),
        press_enter: true,
        auto_learn: false,
        sounds: false,
        mute_while_dictating: true,
        show_bar_always: false,
        launch_at_login: true,
        retention: "day",
        keep_audio: false,
        theme: "light",
      }),
    );
    expect(document.querySelector(".about")!.textContent).toBe("Speechify 1.2.3 · linux");
  });
});
