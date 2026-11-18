import { beforeEach, describe, expect, it } from "vitest";
import { createSettingsPage } from "../src/pages/settings";
import { byLabel, byText, change, makeDeps, settle, status } from "./fakes";

const mac = status({ platform: "macos", updates: true });

function updates() {
  const section = [...document.querySelectorAll("section")].find(
    (el) => el.querySelector("h2")?.textContent === "Updates",
  );
  return {
    section,
    line: section?.querySelector("[role=status]")?.textContent,
    button: section?.querySelector("button") as HTMLButtonElement | undefined,
  };
}

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

  it("downloads diagnostics locally and shows the saved path", async () => {
    await setup();
    const button = byText<HTMLButtonElement>(document, "button", "Download diagnostics");
    button.click();
    expect(button.disabled).toBe(true);
    await settle();
    expect(ctx.api.exportDiagnostics).toHaveBeenCalledOnce();
    expect(document.body.textContent).toContain("/home/me/Downloads/speechify-diagnostics.json");
    expect(document.body.textContent).toContain("nothing is uploaded");
    expect(button.disabled).toBe(false);
  });

  it("lets the user retry after diagnostics cannot be saved", async () => {
    ctx.api.exportDiagnostics.mockRejectedValueOnce("Downloads is not writable.");
    await setup();
    const button = byText<HTMLButtonElement>(document, "button", "Download diagnostics");
    button.click();
    await settle();
    expect(document.body.textContent).toContain("Downloads is not writable.");
    expect(button.disabled).toBe(false);
    button.click();
    await settle();
    expect(ctx.api.exportDiagnostics).toHaveBeenCalledTimes(2);
    expect(document.body.textContent).not.toContain("Downloads is not writable.");
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

  it("leaves out updates where the build cannot update itself", async () => {
    await setup();
    expect(updates().section).toBeUndefined();
    expect(ctx.api.checkUpdate).not.toHaveBeenCalled();
  });

  it("says when Speechify is up to date and checks again on request", async () => {
    ctx.api.getStatus.mockResolvedValue(mac);
    await setup();
    await settle();
    expect(updates().line).toBe("Speechify is up to date.");
    expect(updates().button!.textContent).toBe("Check again");
    ctx.api.checkUpdate.mockResolvedValueOnce({ version: "1.3.0", notes: "" });
    updates().button!.click();
    expect(updates().button!.hidden).toBe(true);
    expect(updates().line).toBe("Checking for updates…");
    await settle();
    expect(updates().line).toBe("Speechify 1.3.0 is available.");
    expect(ctx.api.checkUpdate).toHaveBeenCalledTimes(2);
  });

  it("installs an available update and offers a retry when it fails", async () => {
    ctx.api.getStatus.mockResolvedValue(mac);
    ctx.api.checkUpdate.mockResolvedValue({ version: "1.3.0", notes: "Fixes" });
    ctx.api.installUpdate.mockRejectedValueOnce("Could not install the update: offline");
    await setup();
    await settle();
    const button = updates().button!;
    expect(button.textContent).toBe("Install and restart");
    expect(button.className).toBe("btn primary");
    button.click();
    expect(button.disabled).toBe(true);
    expect(updates().line).toBe("Installing the update. Speechify will restart by itself.");
    await settle();
    expect(updates().line).toBe("Could not install the update: offline");
    expect(button.textContent).toBe("Try again");
    expect(button.disabled).toBe(false);
    button.click();
    await settle();
    expect(ctx.api.installUpdate).toHaveBeenCalledTimes(2);
  });

  it("shows why a check failed and tries again", async () => {
    ctx.api.getStatus.mockResolvedValue(mac);
    ctx.api.checkUpdate.mockRejectedValueOnce("Could not check for updates: offline");
    await setup();
    await settle();
    expect(updates().line).toBe("Could not check for updates: offline");
    expect(updates().button!.textContent).toBe("Try again");
    updates().button!.click();
    await settle();
    expect(updates().line).toBe("Speechify is up to date.");
  });
});
