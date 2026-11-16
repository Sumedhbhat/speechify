import { beforeEach, describe, expect, it } from "vitest";
import { createLearningPage } from "../src/pages/learning";
import { byLabel, byText, change, makeDeps, settle, teacher, toasts } from "./fakes";

let ctx: ReturnType<typeof makeDeps>;

async function setup() {
  const page = createLearningPage(ctx.deps);
  document.body.append(page.el);
  await page.show();
}

const run = () => byText<HTMLButtonElement>(document, "button", "Review now");

beforeEach(() => {
  ctx = makeDeps();
});

describe("learning page", () => {
  it("explains the teacher and shows an empty report", async () => {
    await setup();
    expect(document.querySelector(".prose")!.textContent).toContain("bigger speech model");
    expect(document.body.textContent).toContain("Not run yet. 5 dictations waiting.");
    expect(document.querySelector(".empty-title")!.textContent).toBe("No report yet");
  });

  it("saves the schedule settings", async () => {
    await setup();
    change(byLabel<HTMLInputElement>(document, "Review every night"), true);
    change(byLabel<HTMLSelectElement>(document, "Start at"), "4");
    change(byLabel<HTMLInputElement>(document, "Teacher language model"), "llama3.1:8b");
    change(byLabel<HTMLInputElement>(document, "Learn a correction after"), "5");
    await settle();
    expect(byLabel<HTMLSelectElement>(document, "Start at").selectedOptions[0]!.textContent).toBe(
      "4 am",
    );
    expect(ctx.api.saveSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({
        teacher: expect.objectContaining({
          enabled: true,
          hour: 4,
          rewrite_model: "llama3.1:8b",
          learn_after: 5,
        }),
      }),
    );
  });

  it("shows the last report", async () => {
    ctx.api.teacherStatus.mockResolvedValue(
      teacher({
        last_run: 1_700_000_000,
        report: {
          reviewed: 40,
          skipped: 2,
          average_wer: 0.08,
          learned: [{ from: "tory", to: "Tauri" }],
          pending: 0,
          examples: 6,
        },
      }),
    );
    await setup();
    expect(document.body.textContent).toMatch(/Last review .+\. 5 dictations waiting\./);
    const stats = document.querySelector(".report .stats")!.textContent;
    expect(stats).toContain("Reviewed40");
    expect(stats).toContain("Accuracy92%");
    expect(stats).toContain("Examples added6");
    expect(document.querySelector(".learned")!.textContent).toBe("tory → Tauri");
  });

  it("copes with a report that has no score or corrections", async () => {
    ctx.api.teacherStatus.mockResolvedValue(
      teacher({
        report: {
          reviewed: 0,
          skipped: 0,
          average_wer: null,
          learned: [],
          pending: 0,
          examples: 0,
        },
      }),
    );
    await setup();
    expect(document.querySelector(".report .stats")!.textContent).toContain("Accuracy–");
    expect(document.querySelector(".learned")).toBeNull();
  });

  it("runs a review now and reports when it finishes", async () => {
    await setup();
    run().click();
    expect(ctx.api.runTeacher).toHaveBeenCalled();
    const button = document.querySelector<HTMLButtonElement>(".learning .btn.primary")!;
    expect(button.disabled).toBe(true);
    expect(button.textContent).toBe("Reviewing…");

    ctx.emit("teacherFinished", {
      report: { reviewed: 3, skipped: 0, average_wer: 0.1, learned: [], pending: 0, examples: 1 },
      error: "",
    });
    await settle();
    expect(toasts()).toContain("Review finished: 3 dictations checked");
    expect(button.disabled).toBe(false);

    ctx.emit("teacherFinished", { report: null, error: "" });
    ctx.emit("teacherFinished", { report: null, error: "The teacher model is missing." });
    await settle();
    expect(toasts()).toContain("Review finished");
    expect(toasts()).toContain("The teacher model is missing.");

    ctx.api.teacherStatus.mockRejectedValueOnce("Status unavailable.");
    ctx.emit("teacherFinished", { report: null, error: "" });
    await settle();
    expect(toasts()).toContain("Status unavailable.");
  });

  it("is disabled while a review runs and recovers when starting fails", async () => {
    ctx.api.teacherStatus.mockResolvedValueOnce(teacher({ running: true }));
    await setup();
    const button = document.querySelector<HTMLButtonElement>(".learning .btn.primary")!;
    expect(button.disabled).toBe(true);
    expect(button.textContent).toBe("Reviewing…");
    ctx.emit("teacherFinished", { report: null, error: "" });
    await settle();
    ctx.api.runTeacher.mockRejectedValueOnce("No speech model for the teacher.");
    run().click();
    await settle();
    expect(toasts()).toContain("No speech model for the teacher.");
    expect(run().disabled).toBe(false);
  });

  it("exports training data and shows where it went", async () => {
    await setup();
    byText<HTMLButtonElement>(document, "button", "Export training data").click();
    await settle();
    expect(document.querySelector(".learning code")!.textContent).toBe(
      "/home/me/speechify-training.jsonl",
    );
    ctx.api.exportTraining.mockRejectedValueOnce("Nothing to export yet.");
    byText<HTMLButtonElement>(document, "button", "Export training data").click();
    await settle();
    expect(toasts()).toContain("Nothing to export yet.");
  });
});
