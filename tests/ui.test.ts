import { describe, expect, it, vi } from "vitest";
import {
  confirmButton,
  debounce,
  emptyState,
  errorText,
  flashSaved,
  h,
  icon,
  iconButton,
  nextId,
  pageHeader,
  section,
  toast,
} from "../src/ui";
import { toasts } from "./fakes";

describe("h", () => {
  it("sets properties, attributes, listeners and children", () => {
    const onclick = vi.fn();
    const el = h(
      "button",
      { className: "x", "aria-label": "Go", role: "tab", title: undefined, onclick },
      ["a", null, false, undefined, h("span", { textContent: "b" })],
    );
    expect(el.className).toBe("x");
    expect(el.getAttribute("aria-label")).toBe("Go");
    expect(el.getAttribute("role")).toBe("tab");
    expect(el.hasAttribute("title")).toBe(false);
    expect(el.textContent).toBe("ab");
    el.click();
    expect(onclick).toHaveBeenCalledOnce();
  });

  it("treats non-function on* props as properties", () => {
    const el = h("div", { one: "value" });
    expect((el as unknown as Record<string, string>).one).toBe("value");
    expect(h("p").outerHTML).toBe("<p></p>");
  });
});

describe("icons and buttons", () => {
  it("renders decorative svg icons", () => {
    const el = icon("mic");
    expect(el.getAttribute("aria-hidden")).toBe("true");
    expect(el.querySelector("svg")).not.toBeNull();
  });

  it("labels icon buttons and accepts extra props", () => {
    const onclick = vi.fn();
    const button = iconButton("copy", "Copy", onclick, { "data-action": "copy" });
    expect(button.getAttribute("aria-label")).toBe("Copy");
    expect(button.dataset.action).toBe("copy");
    button.click();
    expect(onclick).toHaveBeenCalled();
    expect(iconButton("trash", "Delete", onclick).type).toBe("button");
  });
});

describe("errorText", () => {
  it("passes sentences through and unwraps errors", () => {
    expect(errorText("Bad shortcut.")).toBe("Bad shortcut.");
    expect(errorText(new Error("Boom"))).toBe("Boom");
    expect(errorText(42)).toBe("Something went wrong.");
  });
});

describe("toast and saved flash", () => {
  it("adds toasts to one live region and removes them later", () => {
    vi.useFakeTimers();
    toast("Copied");
    toast("Nope", "error");
    expect(document.querySelectorAll("#toasts").length).toBe(1);
    expect(document.getElementById("toasts")!.getAttribute("aria-live")).toBe("polite");
    expect(toasts()).toEqual(["Copied", "Nope"]);
    expect(document.querySelector(".toast.error")!.getAttribute("role")).toBe("alert");
    vi.advanceTimersByTime(3500);
    expect(toasts()).toEqual(["Nope"]);
    vi.advanceTimersByTime(2500);
    expect(toasts()).toEqual([]);
  });

  it("flashes Saved and hides it after a moment", () => {
    vi.useFakeTimers();
    flashSaved();
    flashSaved();
    const el = document.getElementById("saved")!;
    expect(el.textContent).toBe("Saved");
    expect(el.classList.contains("show")).toBe(true);
    vi.advanceTimersByTime(1500);
    expect(el.classList.contains("show")).toBe(false);
  });
});

describe("debounce", () => {
  it("runs the last call after the wait, or right away on flush", () => {
    vi.useFakeTimers();
    const fn = vi.fn();
    const run = debounce(fn, 100);
    run(1);
    run(2);
    vi.advanceTimersByTime(99);
    expect(fn).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(fn).toHaveBeenCalledExactlyOnceWith(2);
    run(3);
    run.flush();
    expect(fn).toHaveBeenLastCalledWith(3);
    run.flush();
    vi.advanceTimersByTime(200);
    expect(fn).toHaveBeenCalledTimes(2);
  });
});

describe("confirmButton", () => {
  it("asks once before running and resets after a while", () => {
    vi.useFakeTimers();
    const action = vi.fn();
    const button = confirmButton("Clear", "Really?", action);
    button.click();
    expect(button.textContent).toBe("Really?");
    expect(button.classList.contains("armed")).toBe(true);
    vi.advanceTimersByTime(4000);
    expect(button.textContent).toBe("Clear");
    button.click();
    button.click();
    expect(action).toHaveBeenCalledOnce();
    expect(button.textContent).toBe("Clear");
  });
});

describe("layout helpers", () => {
  it("builds sections, headers and empty states", () => {
    expect(section("Title", "Desc", ["x"]).querySelector(".lede")!.textContent).toBe("Desc");
    expect(section("Title", "", []).querySelector(".lede")).toBeNull();
    expect(pageHeader("Home", "Hello").querySelector("h1")!.textContent).toBe("Home");
    const empty = emptyState("Nothing", "Yet", [h("button")]);
    expect(empty.querySelector(".empty-title")!.textContent).toBe("Nothing");
    expect(empty.querySelector("button")).not.toBeNull();
    expect(emptyState("A", "B").children.length).toBe(2);
    expect(nextId("x")).not.toBe(nextId("x"));
  });
});
