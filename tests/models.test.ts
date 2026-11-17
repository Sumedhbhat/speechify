import { beforeEach, describe, expect, it } from "vitest";
import { createModelsPage } from "../src/pages/models";
import type { Page } from "../src/deps";
import { byLabel, byText, change, makeDeps, model, settle, status, toasts } from "./fakes";

let ctx: ReturnType<typeof makeDeps>;
let page: Page;

async function setup() {
  page = createModelsPage(ctx.deps);
  document.body.append(page.el);
  await page.show();
}

const card = (name: string) => byText(document, ".model h3", name).parentElement!;
const button = (root: ParentNode, text: string) => byText<HTMLButtonElement>(root, "button", text);

beforeEach(() => {
  ctx = makeDeps();
  ctx.api.models.mockResolvedValue([
    model(),
    model({
      id: "small",
      name: "Whisper small",
      bytes: 480_000_000,
      wants_gpu: false,
      active: false,
      teacher: false,
    }),
    model({ id: "large", name: "Whisper large", installed: false, active: false, teacher: true }),
    model({ id: "medium", name: "Whisper medium", active: false, teacher: true }),
  ]);
});

describe("models page", () => {
  it("shows speech models with their size and state", async () => {
    await setup();
    const turbo = card("Whisper turbo");
    expect(turbo.classList.contains("active")).toBe(true);
    expect(turbo.textContent).toContain("1.6 GB");
    expect(turbo.textContent).toContain("Needs a graphics card");
    expect(turbo.textContent).toContain("Used for dictation");
    expect(button(turbo, "In use").disabled).toBe(true);
    expect(card("Whisper small").textContent).toContain("480 MB");
    expect(card("Whisper small").textContent).not.toContain("Needs a graphics card");
    expect(card("Whisper large").textContent).toContain("Teacher");
    expect(button(card("Whisper large"), "Download")).toBeTruthy();
    expect(button(card("Whisper medium"), "Teacher").disabled).toBe(true);
  });

  it("picks models for dictation and for the teacher", async () => {
    await setup();
    button(card("Whisper small"), "Use for dictation").click();
    await settle();
    expect(ctx.api.saveSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({ speech: expect.objectContaining({ model: "small" }) }),
    );
    expect(ctx.api.models).toHaveBeenCalledTimes(2);

    button(card("Whisper small"), "Use as teacher").click();
    await settle();
    expect(ctx.api.saveSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({ teacher: expect.objectContaining({ speech_model: "small" }) }),
    );

    ctx.api.saveSettings.mockRejectedValueOnce("That model is not installed.");
    button(card("Whisper small"), "Use for dictation").click();
    await settle();
    expect(ctx.api.models).toHaveBeenCalledTimes(3);
    expect(ctx.onError).toHaveBeenCalledWith("That model is not installed.");
  });

  it("downloads with live progress and can cancel", async () => {
    await setup();
    button(card("Whisper large"), "Download").click();
    await settle();
    expect(ctx.api.downloadModel).toHaveBeenCalledWith("large");
    const bar = card("Whisper large").querySelector("progress")!;
    expect(card("Whisper large").querySelector(".meter")!.textContent).toBe("1 MB of 1.6 GB");

    ctx.emit("modelProgress", {
      id: "large",
      done: 800_000_000,
      total: 1_600_000_000,
      finished: false,
      error: "",
    });
    expect(card("Whisper large").querySelector("progress")).toBe(bar);
    expect(bar.value).toBe(800_000_000);
    expect(card("Whisper large").querySelector(".meter")!.textContent).toBe("800 MB of 1.6 GB");

    button(card("Whisper large"), "Cancel").click();
    await settle();
    expect(ctx.api.cancelDownload).toHaveBeenCalledWith("large");

    ctx.api.cancelDownload.mockRejectedValueOnce("Already finished.");
    button(card("Whisper large"), "Cancel").click();
    await settle();
    expect(toasts()).toContain("Already finished.");

    ctx.api.models.mockResolvedValueOnce([
      model({ id: "large", name: "Whisper large", active: false }),
    ]);
    ctx.emit("modelProgress", { id: "large", done: 1, total: 1, finished: true, error: "" });
    await settle();
    expect(card("Whisper large").querySelector("progress")).toBeNull();
    expect(button(card("Whisper large"), "Use for dictation")).toBeTruthy();
  });

  it("shows progress started elsewhere and failed downloads", async () => {
    await setup();
    ctx.emit("modelProgress", { id: "large", done: 0, total: 0, finished: false, error: "" });
    expect(card("Whisper large").querySelector(".meter")!.textContent).toBe("Starting");
    expect(card("Whisper large").querySelector("progress")!.hasAttribute("value")).toBe(false);

    ctx.emit("modelProgress", {
      id: "large",
      done: 0,
      total: 0,
      finished: true,
      error: "The download was interrupted.",
    });
    await settle();
    expect(toasts()).toContain("The download was interrupted.");

    ctx.api.downloadModel.mockRejectedValueOnce("No space left on the disk.");
    button(card("Whisper large"), "Download").click();
    await settle();
    expect(toasts()).toContain("No space left on the disk.");
    expect(card("Whisper large").querySelector("progress")).toBeNull();
  });

  it("ignores progress for a card that is not on screen", async () => {
    page = createModelsPage(ctx.deps);
    ctx.emit("modelProgress", { id: "large", done: 1, total: 10, finished: false, error: "" });
    ctx.emit("modelProgress", { id: "large", done: 2, total: 10, finished: false, error: "" });
    document.body.append(page.el);
    await page.show();
    expect(card("Whisper large").querySelector("progress")!.value).toBe(2);
  });

  it("deletes an installed model after confirming", async () => {
    await setup();
    const remove = button(card("Whisper small"), "Delete");
    remove.click();
    remove.click();
    await settle();
    expect(ctx.api.deleteModel).toHaveBeenCalledWith("small");
    expect(ctx.api.models).toHaveBeenCalledTimes(2);
  });

  it("picks a device and shows what the model runs on", async () => {
    await setup();
    expect(document.body.textContent).toContain("Running on vulkan NVIDIA GeForce RTX 4060");
    change(byLabel<HTMLSelectElement>(document, "Run speech on"), "cpu");
    await settle();
    expect(ctx.api.saveSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({ speech: expect.objectContaining({ device: "cpu" }) }),
    );

    ctx.api.getStatus.mockResolvedValue(status({ speech_device: "" }));
    await page.show();
    expect(document.body.textContent).toContain("The model loads when you first dictate.");

    ctx.api.getStatus.mockResolvedValue(status({ speech_error: "Vulkan is not available." }));
    await page.show();
    expect(document.querySelector(".status-note.error")!.textContent).toBe(
      "Vulkan is not available.",
    );
  });

  it("configures the language model", async () => {
    await setup();
    expect(document.querySelector(".status-note.ok")!.textContent).toBe("Connected");
    expect(
      [...document.querySelectorAll("#rewrite-models option")].map(
        (o) => (o as HTMLOptionElement).value,
      ),
    ).toEqual(["qwen2.5:1.5b", "llama3.2:3b"]);
    change(byLabel<HTMLInputElement>(document, "Tidy text with a language model"), false);
    change(byLabel<HTMLInputElement>(document, "Server"), "http://localhost:8080");
    change(byLabel<HTMLInputElement>(document, "Model"), "llama3.2:3b");
    change(byLabel<HTMLInputElement>(document, "Give up after"), "4000");
    change(byLabel<HTMLInputElement>(document, "Skip short dictations"), "2");
    change(byLabel<HTMLInputElement>(document, "Address"), "http://box:9000");
    change(byLabel<HTMLInputElement>(document, "Key"), "secret");
    change(byLabel<HTMLInputElement>(document, "Model name"), "whisper-1");
    await settle();
    expect(ctx.api.saveSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({
        rewrite: expect.objectContaining({
          enabled: false,
          url: "http://localhost:8080",
          model: "llama3.2:3b",
          timeout_ms: 4000,
          min_words: 2,
        }),
        speech: expect.objectContaining({
          remote_url: "http://box:9000",
          remote_key: "secret",
          remote_model: "whisper-1",
        }),
      }),
    );
    expect(byLabel<HTMLInputElement>(document, "Key").type).toBe("password");
  });

  it("downloads a language model through Ollama", async () => {
    await setup();
    button(document, "Download with Ollama").click();
    await settle();
    expect(ctx.api.pullRewriteModel).toHaveBeenCalledWith("qwen2.5:1.5b");
    expect(document.querySelector(".pull")!.textContent).toBe("Starting");

    ctx.emit("rewriteProgress", {
      id: "qwen2.5:1.5b",
      done: 500_000_000,
      total: 1_000_000_000,
      finished: false,
      error: "",
    });
    expect(document.querySelector(".pull")!.textContent).toBe("500 MB of 1.0 GB");
    ctx.emit("rewriteProgress", {
      id: "qwen2.5:1.5b",
      done: 1,
      total: 1,
      finished: true,
      error: "",
    });
    expect(document.querySelector(".pull")!.textContent).toBe("");
    expect(toasts()).toContain("qwen2.5:1.5b is ready");

    ctx.emit("rewriteProgress", {
      id: "x",
      done: 0,
      total: 0,
      finished: true,
      error: "Ollama refused: no such model.",
    });
    expect(toasts()).toContain("Ollama refused: no such model.");

    ctx.api.pullRewriteModel.mockRejectedValueOnce("Ollama is not running.");
    button(document, "Download with Ollama").click();
    await settle();
    expect(toasts()).toContain("Ollama is not running.");
  });

  it("explains an unreachable language-model server", async () => {
    ctx.api.getStatus.mockResolvedValue(status({ rewrite_reachable: false }));
    await setup();
    expect(document.querySelector(".status-note.error")!.textContent).toContain(
      "Can't reach the server",
    );
    expect(button(document, "Download with Ollama").disabled).toBe(true);
  });

  it("checks the language-model server again on refresh", async () => {
    ctx.api.getStatus.mockResolvedValue(status({ rewrite_reachable: false, rewrite_models: [] }));
    await setup();
    expect(button(document, "Download with Ollama").disabled).toBe(true);

    ctx.api.getStatus.mockResolvedValue(status());
    const refresh = button(document, "Refresh");
    refresh.click();
    expect(refresh.disabled).toBe(true);
    await settle();
    expect(document.querySelector(".status-note.ok")!.textContent).toBe("Connected");
    expect(button(document, "Download with Ollama").disabled).toBe(false);
    expect(
      [...document.querySelectorAll("#rewrite-models option")].map(
        (o) => (o as HTMLOptionElement).value,
      ),
    ).toEqual(["qwen2.5:1.5b", "llama3.2:3b"]);
    expect(button(document, "Refresh").disabled).toBe(false);

    // Keyboard focus comes back to Refresh, but not away from a field the user moved to.
    refresh.focus();
    refresh.click();
    // A browser drops focus from a button when it is disabled; jsdom does not.
    byLabel<HTMLInputElement>(document, "Server").focus();
    byLabel<HTMLInputElement>(document, "Server").blur();
    expect(document.activeElement).toBe(document.body);
    await settle();
    expect(document.activeElement).toBe(refresh);
    refresh.click();
    byLabel<HTMLInputElement>(document, "Server").focus();
    await settle();
    expect(document.activeElement).toBe(byLabel(document, "Server"));

    // Fields keep text typed while the check is still running.
    let answer!: (value: ReturnType<typeof status>) => void;
    ctx.api.getStatus.mockReturnValueOnce(new Promise((resolve) => (answer = resolve)));
    const field = byLabel<HTMLInputElement>(document, "Model");
    button(document, "Refresh").click();
    await settle();
    expect(button(document, "Refresh").disabled).toBe(true);
    field.value = "mistral:7b";
    answer(status({ rewrite_reachable: false, rewrite_models: [] }));
    await settle();
    expect(byLabel<HTMLInputElement>(document, "Model")).toBe(field);
    expect(field.value).toBe("mistral:7b");
    expect(document.querySelector(".status-note.error")!.textContent).toContain(
      "Can't reach the server",
    );
    expect(document.querySelectorAll("#rewrite-models option")).toHaveLength(0);
    expect(button(document, "Download with Ollama").disabled).toBe(true);

    // A server address typed just before clicking is saved before the server is asked.
    let saved!: () => void;
    ctx.api.saveSettings.mockReturnValueOnce(new Promise<void>((resolve) => (saved = resolve)));
    ctx.api.getStatus.mockResolvedValue(status());
    const asked = ctx.api.getStatus.mock.calls.length;
    change(byLabel<HTMLInputElement>(document, "Server"), "http://localhost:11500");
    button(document, "Refresh").click();
    await settle();
    expect(ctx.api.getStatus).toHaveBeenCalledTimes(asked);
    expect(button(document, "Refresh").disabled).toBe(true);
    saved();
    await settle();
    expect(ctx.api.getStatus).toHaveBeenCalledTimes(asked + 1);
    expect(document.querySelector(".status-note.ok")!.textContent).toBe("Connected");
    expect(button(document, "Refresh").disabled).toBe(false);

    ctx.api.getStatus.mockRejectedValueOnce("Speechify is not responding.");
    button(document, "Refresh").click();
    await settle();
    expect(toasts()).toContain("Speechify is not responding.");
    expect(button(document, "Refresh").disabled).toBe(false);
  });
});
