import "./bar.css";
import { api, events } from "./api";
import { createBar } from "./bar-view";

const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
createBar(document.getElementById("bar")!, {
  api,
  events,
  frame: (callback) => requestAnimationFrame(callback),
  cancelFrame: (id) => cancelAnimationFrame(id),
  reducedMotion: () => motion.matches,
});
