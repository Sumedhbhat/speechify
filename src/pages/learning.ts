// Learning: the nightly teacher that re-checks dictations with bigger models.

import type { TeacherReport, TeacherStatus } from "../api";
import { number, select, text, toggle } from "../controls";
import type { Deps, Page } from "../deps";
import { dateTime, hourLabel, percent, plural } from "../format";
import { emptyState, errorText, h, pageHeader, section, toast } from "../ui";

function report(last: TeacherReport): HTMLElement {
  const item = (label: string, value: string) =>
    h("div", { className: "stat" }, [
      h("dt", { textContent: label }),
      h("dd", { textContent: value }),
    ]);
  return h("div", { className: "report" }, [
    h("dl", { className: "stats" }, [
      item("Reviewed", String(last.reviewed)),
      item("Accuracy", last.average_wer === null ? "–" : percent(1 - last.average_wer)),
      item("Corrections learned", String(last.learned.length)),
      item("Examples added", String(last.examples)),
    ]),
    last.learned.length > 0 &&
      h(
        "ul",
        { className: "learned" },
        last.learned.map((r) =>
          h("li", {}, [
            h("span", { className: "heard", textContent: r.from }),
            " → ",
            h("strong", { textContent: r.to }),
          ]),
        ),
      ),
  ]);
}

export function createLearningPage({ api, events, settings: store }: Deps): Page {
  const controls = h("div");
  const state = h("div");
  const exported = h("p", { className: "hint", "aria-live": "polite" });
  const run = h("button", {
    type: "button",
    className: "btn primary",
    textContent: "Review now",
    onclick: () => void start(),
  });
  const exportButton = h("button", {
    type: "button",
    className: "btn",
    textContent: "Export training data",
    onclick: () => void exportData(),
  });

  function renderStatus(status: TeacherStatus): void {
    run.disabled = status.running;
    run.textContent = status.running ? "Reviewing…" : "Review now";
    state.replaceChildren(
      h("p", { className: "status-note" }, [
        status.last_run === null ? "Not run yet." : `Last review ${dateTime(status.last_run)}.`,
        ` ${plural(status.waiting, "dictation")} waiting.`,
      ]),
      status.report
        ? report(status.report)
        : emptyState(
            "No report yet",
            "After the first review you will see what it checked and learned.",
          ),
    );
  }

  async function refresh(): Promise<void> {
    renderStatus(await api.teacherStatus());
  }

  async function start(): Promise<void> {
    run.disabled = true;
    run.textContent = "Reviewing…";
    try {
      await api.runTeacher();
    } catch (error) {
      toast(errorText(error), "error");
      await refresh();
    }
  }

  async function exportData(): Promise<void> {
    try {
      const path = await api.exportTraining();
      exported.replaceChildren("Saved to ", h("code", { textContent: path }));
    } catch (error) {
      toast(errorText(error), "error");
    }
  }

  void events.teacherFinished(({ report: done, error }) => {
    if (error) toast(error, "error");
    else
      toast(
        done ? `Review finished: ${plural(done.reviewed, "dictation")} checked` : "Review finished",
      );
    void refresh().catch((failure: unknown) => toast(errorText(failure), "error"));
  });

  const el = h("div", { className: "page learning" }, [
    pageHeader("Learning", "Speechify gets better at your voice while you sleep."),
    h("div", { className: "prose" }, [
      h("p", {
        textContent:
          "Once a day, a bigger speech model and a bigger language model listen again to dictations whose recordings were kept, and compare them with what was typed.",
      }),
      h("p", {
        textContent:
          "Mistakes that keep coming back become dictionary corrections, and the best results become examples in the everyday model’s instructions. Nothing is retrained. Speechify must be running, and the review pauses when you start dictating.",
      }),
    ]),
    section("Schedule", "", [controls]),
    section("Last review", "", [
      state,
      h("div", { className: "row" }, [run, exportButton]),
      exported,
    ]),
  ]);

  return {
    el,
    async show() {
      const [settings, status] = await Promise.all([store.load(), api.teacherStatus()]);
      controls.replaceChildren(
        toggle(store, settings, {
          label: "Review every night",
          hint: "Runs once a day, after the hour below, while Speechify is open and you are not dictating. It works hard for a while.",
          get: (s) => s.teacher.enabled,
          set: (s, v) => (s.teacher.enabled = v),
        }),
        select(store, settings, {
          label: "Start at",
          options: Array.from({ length: 24 }, (_, hour) => [String(hour), hourLabel(hour)]),
          get: (s) => String(s.teacher.hour),
          set: (s, v) => (s.teacher.hour = Number(v)),
        }),
        text(store, settings, {
          label: "Teacher language model",
          hint: "Reached through the server set in Models. A larger model than the everyday one, if your computer can run it.",
          placeholder: "qwen3.5:9b-q4_K_M",
          get: (s) => s.teacher.rewrite_model,
          set: (s, v) => (s.teacher.rewrite_model = v),
        }),
        number(store, settings, {
          label: "Learn a correction after",
          hint: "How many times the same mistake must appear before it is added to your dictionary. A fix you make by hand in History counts straight away.",
          min: 1,
          max: 20,
          unit: "times",
          get: (s) => s.teacher.learn_after,
          set: (s, v) => (s.teacher.learn_after = v),
        }),
      );
      renderStatus(status);
    },
  };
}
