// Native hook crashes must not take down the service or its saved data.
import { uIOhook } from "uiohook-napi";
uIOhook.on("keydown", (e) => process.send?.({ code: e.keycode, down: true }));
uIOhook.on("keyup", (e) => process.send?.({ code: e.keycode, down: false }));
process.on("disconnect", () => {
  uIOhook.stop();
  process.exit(0);
});
uIOhook.start();
process.send?.({ ready: true });
