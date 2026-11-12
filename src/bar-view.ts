// The floating dictation bar: a small pill at the bottom of the screen that
// shows what dictation is doing. Pure DOM; the shell drives it through events.

import type { Bar, BarAction } from "./api";
import type { Api, Events } from "./deps";
import { errorText, h, icon } from "./ui";

export interface BarDeps {
  api: Pick<Api, "barAction" | "copyText">;
  events: Pick<Events, "bar" | "level">;
  frame: (callback: (time: number) => void) => number;
  cancelFrame: (id: number) => void;
  reducedMotion: () => boolean;
}

const BARS = 9;
// Taller in the middle so the waveform reads as a voice, not a meter.
const SHAPE = [0.45, 0.6, 0.78, 0.92, 1, 0.92, 0.78, 0.6, 0.45];
const DONE_MS = 1200;
const COPIED_MS = 1000;
const NOTICE_MS = 4000;
const ERROR_MS = 6000;

export function createBar(root: HTMLElement, deps: BarDeps): { show(bar: Bar): void } {
  const above = h("div", { className: "bar-above" });
  const pill = h("div", { className: "pill" });
  const live = h("div", { className: "sr-only", "aria-live": "polite" });
  root.append(h("div", { className: "bar-stack" }, [above, pill, live]));

  let timer: ReturnType<typeof setTimeout> | undefined;
  let frameId = 0;
  let level = 0;
  let smooth = 0;
  let generation = 0;

  const act = (action: BarAction) => void deps.api.barAction(action).catch(() => undefined);
  const later = (ms: number, then: () => void) => (timer = setTimeout(then, ms));
  const idle = () => show({ state: "idle" });

  function wave(): HTMLElement {
    const bars = Array.from({ length: BARS }, () => h("span", { className: "wave-bar" }));
    const still = deps.reducedMotion();
    const tick = (time: number) => {
      // Fast attack, slow release, and a gentle drift so steady speech still moves.
      const rate = level > smooth ? 0.6 : 0.12;
      smooth += (level - smooth) * rate;
      level *= 0.92;
      const loud = Math.min(1, smooth * 1.8);
      bars.forEach((bar, i) => {
        const drift = still ? 1 : 0.72 + 0.28 * Math.sin(time / 150 + i * 1.7);
        const scale = 0.14 + 0.86 * loud * SHAPE[i]! * drift;
        bar.style.transform = `scaleY(${scale.toFixed(3)})`;
      });
      frameId = deps.frame(tick);
    };
    frameId = deps.frame(tick);
    return h("div", { className: "wave", "aria-hidden": "true" }, bars);
  }

  function pillButton(name: Parameters<typeof icon>[0], label: string, action: BarAction) {
    return h(
      "button",
      {
        type: "button",
        className: `pill-btn ${action}`,
        "aria-label": label,
        title: label,
        onclick: () => act(action),
      },
      [icon(name)],
    );
  }

  function idlePill(): HTMLElement {
    return h(
      "button",
      {
        type: "button",
        className: "pill-idle",
        "aria-label": "Click to dictate",
        onclick: () => act("click"),
      },
      [icon("mic"), h("span", { className: "pill-hint", textContent: "Click to dictate" })],
    );
  }

  function card(text: string, message: string): HTMLElement {
    const note = h("p", { className: "card-message", textContent: message, hidden: !message });
    const gen = generation;
    const copy = h("button", {
      type: "button",
      className: "card-copy",
      textContent: "Copy",
      onclick: () =>
        deps.api.copyText(text).then(
          () => {
            if (gen !== generation) return;
            copy.textContent = "Copied";
            later(COPIED_MS, () => {
              act("dismiss");
              idle();
            });
          },
          (error: unknown) => {
            note.textContent = errorText(error);
            note.classList.add("error");
            note.hidden = false;
          },
        ),
    });
    return h("div", { className: "card", role: "dialog", "aria-label": "Dictated text" }, [
      h("div", { className: "card-text", tabIndex: 0, textContent: text }),
      h("div", { className: "card-actions" }, [copy, pillButton("close", "Dismiss", "dismiss")]),
      note,
    ]);
  }

  function notice(message: string, error: boolean, undo: boolean): HTMLElement {
    return h(
      "div",
      { className: `notice${error ? " error" : ""}`, role: error ? "alert" : "status" },
      [
        h("span", { textContent: message }),
        undo &&
          h("button", {
            type: "button",
            className: "notice-undo",
            textContent: "Undo",
            onclick: () => act("undo"),
          }),
      ],
    );
  }

  function show(bar: Bar): void {
    generation += 1;
    clearTimeout(timer);
    deps.cancelFrame(frameId);
    above.replaceChildren();
    pill.className = `pill ${bar.state}`;
    live.textContent = "";

    switch (bar.state) {
      case "idle":
        pill.replaceChildren(idlePill());
        break;
      case "listening":
        pill.classList.toggle("command", bar.command);
        pill.replaceChildren(
          pillButton("close", "Cancel dictation", "cancel"),
          bar.command ? h("span", { className: "pill-label", textContent: "Command" }) : "",
          wave(),
          bar.locked ? pillButton("stop", "Finish dictation", "click") : "",
        );
        live.textContent = bar.command ? "Listening for a command" : "Listening";
        break;
      case "processing":
        pill.replaceChildren(h("div", { className: "shimmer", "aria-hidden": "true" }));
        live.textContent = "Working on it";
        break;
      case "done": {
        const words = bar.words > 0 ? `${bar.words} ${bar.words === 1 ? "word" : "words"}` : "Done";
        pill.replaceChildren(
          icon("check"),
          h("span", { className: "pill-label", textContent: words }),
        );
        live.textContent = words;
        later(DONE_MS, idle);
        break;
      }
      case "card":
        pill.className = "pill idle";
        pill.replaceChildren(idlePill());
        above.replaceChildren(card(bar.text, bar.message));
        break;
      case "notice":
        pill.className = "pill idle";
        pill.replaceChildren(idlePill());
        above.replaceChildren(notice(bar.message, bar.error, bar.undo));
        later(bar.error ? ERROR_MS : NOTICE_MS, idle);
        break;
    }
  }

  void deps.events.bar(show);
  void deps.events.level((value) => (level = Math.max(0, Math.min(1, value))));
  idle();
  return { show };
}
