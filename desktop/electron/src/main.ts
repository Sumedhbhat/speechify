import {
  app,
  ClipboardItem,
  BrowserWindow,
  ipcMain,
  Tray,
  Menu,
  nativeImage,
  clipboard,
  dialog,
  shell,
  nativeTheme,
  globalShortcut,
  systemPreferences,
  powerMonitor,
  session,
} from "electron";
import { fork } from "node:child_process";
import { killTree } from "./python-engine";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { Settings, HoldKey, pcmWav, Timings, BoundedQueue } from "./core";
import { Store } from "./store";
import { Models } from "./models";
import { Engine } from "./engine";
import { Output, guardedPaste } from "./output";
import { PythonEngine } from "./python-engine";
import { Learning } from "./learning";
const name = "speechify-electron-candidate";
app.setName("Speechify Electron Candidate");
app.setAppUserModelId("org.sumedhbhat.speechify.electron");
const root = path.join(
  process.platform === "win32"
    ? process.env.APPDATA || app.getPath("appData")
    : process.platform === "darwin"
      ? path.join(os.homedir(), "Library/Application Support")
      : process.env.XDG_DATA_HOME || path.join(os.homedir(), ".local/share"),
  name,
);
app.setPath("userData", root);
let settings: Settings,
  store: Store,
  models: Models,
  engine: Engine,
  logs: Timings,
  window: BrowserWindow | undefined,
  captureWindow: BrowserWindow,
  tray: Tray,
  indicator: BrowserWindow | undefined,
  quitting = false,
  hold: HoldKey | undefined,
  hook: any,
  status = "Starting",
  error = "",
  devices: any[] = [],
  recent = "",
  level = 0,
  target = "",
  recording = false,
  activeId = "";
let latestPastedId: number | undefined;
let replacing = false;
let captureReady = false;
let pendingLoad: Promise<void> | undefined;
let workGeneration = 0;
const dictations = new BoundedQueue(4);
let lastKey = Date.now(),
  learning: Learning,
  pythonEngine: PythonEngine,
  inference: Engine | PythonEngine;
const output = new Output();
const marker = "web application/x-speechify-candidate-owner";
const clip = {
  read: async () => {
    const items = await clipboard.read();
    let stamp = "";
    for (const item of items)
      if (item.types.includes(marker))
        stamp = await (await item.getType(marker)).text();
    return {
      text: await clipboard.readText(),
      html: (await clipboard.has("text/html")) ? "present" : "",
      rtf: (await clipboard.has("text/rtf")) ? "present" : "",
      items,
      stamp,
    };
  },
  write: async (v: any) => {
    if (v.items) await clipboard.write(v.items);
    else
      await clipboard.write([
        new ClipboardItem({ "text/plain": v.text, [marker]: v.stamp || "" }),
      ]);
  },
};

function state() {
  return {
    config: settings.value,
    status,
    error,
    models: models.list(),
    history: store.rows(),
    rules: store.rules(),
    vocab: store.vocab(),
    versions: store.state("versions", []),
    teacher: store.state("teacher_status", "Not run"),
    trainingReport: store.state("training_report", null),
    training: store.state(
      "training_status",
      "Optional Python training environment required",
    ),
    device: inference.device,
    devices,
    recent,
    level,
    platform: process.platform,
    session: process.env.XDG_SESSION_TYPE || "",
    root,
    permissions: permissions(),
    replaceableId: output.replaceable ? latestPastedId : undefined,
  };
}
function broadcast() {
  if (quitting) return;
  if (window && !window.isDestroyed())
    window.webContents.send("state", state());
  if (indicator && !indicator.isDestroyed()) {
    indicator.webContents.send("state", { status, level, error });
    if (
      recording ||
      status === "Transcribing" ||
      status === "Needs attention" ||
      status === "Pasted"
    )
      indicator.showInactive();
    else indicator.hide();
  }
}
function fail(e: unknown) {
  if (quitting) return;
  error = String(e);
  status = "Needs attention";
  logs.log("error", { kind: e instanceof Error ? e.name : "Error" });
  broadcast();
}
function permissions() {
  return process.platform === "darwin"
    ? {
        microphone: systemPreferences.getMediaAccessStatus("microphone"),
        accessibility: systemPreferences.isTrustedAccessibilityClient(false),
        guidance:
          "Enable Microphone, Accessibility and Input Monitoring for this candidate in System Settings. Restart after changing Input Monitoring.",
      }
    : process.platform === "win32"
      ? {
          guidance:
            "Allow desktop microphone access in Privacy settings. Elevated applications cannot receive paste from an unelevated candidate (UIPI).",
        }
      : {
          guidance:
            process.env.XDG_SESSION_TYPE === "wayland"
              ? "Wayland: global hold keys and focus-safe paste require compositor support. This build disables global hooks/paste on Wayland. Copy results manually; no automatic privilege grants."
              : "X11: libXtst/libX11 for global keys; xdotool for target checks/paste. Audio uses Chromium/PipeWire or PulseAudio.",
        };
}
function show(page = "Dictation") {
  if (!window || window.isDestroyed()) {
    window = new BrowserWindow({
      width: 1060,
      height: 740,
      minWidth: 560,
      minHeight: 440,
      title: "Speechify Electron Candidate",
      backgroundColor: "#151923",
      webPreferences: {
        preload: path.join(__dirname, "preload.js"),
        contextIsolation: true,
        nodeIntegration: false,
        sandbox: true,
      },
    });
    window.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
    window.webContents.on("will-navigate", (e) => e.preventDefault());
    void window.loadFile(path.join(__dirname, "../ui/index.html"));
    window.webContents.once("did-finish-load", () => {
      broadcast();
      window?.webContents.send("navigate", page);
    });
    window.on("close", (e) => {
      if (!quitting) {
        e.preventDefault();
        window?.hide();
      }
    });
  } else {
    window.show();
    window.webContents.send("navigate", page);
    broadcast();
  }
}
function setStartup(enabled: boolean) {
  if (process.platform === "linux") {
    const dir = path.join(
      process.env.XDG_CONFIG_HOME || path.join(os.homedir(), ".config"),
      "autostart",
    );
    fs.mkdirSync(dir, { recursive: true });
    const f = path.join(dir, name + ".desktop");
    if (enabled)
      fs.writeFileSync(
        f,
        `[Desktop Entry]\nType=Application\nName=Speechify Electron Candidate\nExec="${process.execPath.replace(/"/g, '\\"')}"\nX-GNOME-Autostart-enabled=true\n`,
      );
    else fs.rmSync(f, { force: true });
  } else
    app.setLoginItemSettings({
      openAtLogin: enabled,
      path: process.execPath,
      name,
    });
}
function use(id: string, install = false): Promise<void> {
  if (pendingLoad)
    return Promise.reject(
      Error("Wait for the current model switch or cancel it"),
    );
  const p = loadModel(id, install);
  pendingLoad = p;
  return p.finally(() => {
    if (pendingLoad === p) pendingLoad = undefined;
  });
}
async function loadModel(id: string, install = false) {
  const old = settings.value.model;
  const hadActive = !!activeId;
  const generation = workGeneration;
  status = "Loading model";
  error = "";
  broadcast();
  engine.stop();
  pythonEngine.stop();
  activeId = "";
  try {
    const spec = models.list().find((m) => m.id === id);
    if (!spec) throw Error("Unknown model");
    if (spec.gguf) {
      inference = engine;
      const file = await models.file(id);
      if (generation !== workGeneration) throw Error("Cancelled");
      await engine.load({
        file,
        device: settings.value.device,
        threads: settings.value.onnx_threads,
        language: settings.value.language,
      });
    } else {
      if (!settings.value.inference_python)
        throw Error(
          "Set inference Python executable in Settings for this model",
        );
      inference = pythonEngine;
      const repo = id.startsWith("ft:")
        ? spec.repo
        : await models.optionalFile(id);
      if (generation !== workGeneration) throw Error("Cancelled");
      await pythonEngine.load({
        python: settings.value.inference_python,
        model: id,
        repo,
        install: false,
        opts: {
          device: ["auto", "vulkan", "metal"].includes(settings.value.device)
            ? "cpu"
            : settings.value.device,
          compute_type: settings.value.compute_type,
          language: settings.value.language,
          beam_size: settings.value.beam_size,
          onnx_threads: settings.value.onnx_threads,
        },
      });
    }
    if (generation !== workGeneration) {
      inference.stop();
      throw Error("Cancelled");
    }
    settings.save({ model: id });
    activeId = id;
    status = "Ready";
  } catch (e) {
    activeId = "";
    if (old !== id) {
      settings.save({ model: old });
      if (hadActive && generation === workGeneration) {
        try {
          await loadModel(old);
        } catch {
          activeId = "";
        }
      }
    }
    throw e;
  } finally {
    broadcast();
  }
}

function bind() {
  hold?.cancel();
  globalShortcut.unregisterAll();
  if (!hook) {
    if (
      process.platform === "linux" &&
      process.env.XDG_SESSION_TYPE === "wayland"
    )
      return;
    hook = fork(path.join(__dirname, "hook-worker.js"), [], {
      env: { ...process.env, ELECTRON_RUN_AS_NODE: "1" },
      stdio: ["ignore", "ignore", "ignore", "ipc"],
      detached: process.platform !== "win32",
    });
    const child = hook;
    hook.on("message", (m: any) => {
      if (typeof m.code === "number") {
        if (m.down) lastKey = Date.now();
        hold?.event(m.code, m.down);
      }
    });
    hook.on("error", () => {
      if (hook === child) {
        hook = undefined;
        fail(
          Error("Global keyboard helper failed. Check platform permissions."),
        );
      }
    });
    hook.on("exit", () => {
      if (hook === child) {
        hook = undefined;
        hold?.cancel();
        fail(
          Error(
            "Global keyboard helper exited. Check permissions, then save Shortcuts to restart.",
          ),
        );
      }
    });
  }

  const { UiohookKey: k } = require("uiohook-napi");
  const aliases: any = {
    ctrl_r: "CtrlRight",
    ctrl_l: "Ctrl",
    ctrl: "Ctrl",
    alt_r: "AltRight",
    alt: "Alt",
    shift: "Shift",
    cmd: "Meta",
    meta: "Meta",
    scroll_lock: "ScrollLock",
  };
  const parts = settings.value.ptt_key.replace(/[<>]/g, "").split("+");
  const keys = parts.map((p) => {
    const key =
      aliases[p.toLowerCase()] ||
      Object.keys(k).find((n) => n.toLowerCase() === p.toLowerCase());
    if (!key || !k[key]) throw Error("Unknown hold key: " + p);
    return k[key];
  });
  if (new Set(keys).size !== keys.length) throw Error("Duplicate shortcut key");
  hold = new HoldKey(
    keys,
    settings.value.min_hold_ms,
    () => {
      learning?.preempt();
      startRecording();
    },
    (accept) => {
      recording = false;
      if (accept) targets.set(captureGeneration, target);
      if (targets.size > 4) targets.delete(targets.keys().next().value!);
      captureWindow.webContents.send("capture", {
        type: "stop",
        accept,
        id: captureGeneration,
      });
      status = accept ? "Transcribing" : activeId ? "Ready" : "Choose a model";
      broadcast();
    },
  );
  for (const [key, page] of [
    [settings.value.correct_hotkey, "History"],
    [settings.value.history_hotkey, "History"],
    [settings.value.models_hotkey, "Models"],
  ]) {
    const shortcut = key
      .replace(/[<>]/g, "")
      .replace(/ctrl/gi, "CommandOrControl")
      .replace(/\+/g, "+");
    if (!globalShortcut.register(shortcut, () => show(page)))
      throw Error("Shortcut conflict: " + key);
  }
}
function startRecording() {
  if (recording || replacing) return;
  if (!captureReady) {
    fail(Error("Audio service is not ready; try again shortly"));
    return;
  }
  latestPastedId = undefined;
  output.replaceable = false;
  learning?.preempt();
  recording = true;
  status = "Recording";
  error = "";
  target = "";
  const generation = ++captureGeneration;
  void output
    .focus()
    .then((t) => {
      if (generation === captureGeneration) target = t;
    })
    .catch((e) => {
      error = String(e);
    });
  captureWindow.webContents.send("capture", { type: "start", id: generation });
  broadcast();
}
let captureGeneration = 0;
let dictationsBusy = 0;
const targets = new Map<number, string>();
async function transcribe(
  samples: number[],
  focus: string,
  generation: number,
) {
  dictationsBusy++;
  const started = performance.now();
  const pcm = Float32Array.from(samples);
  const wav = Buffer.alloc(pcm.length * 2);
  pcm.forEach((v, i) =>
    wav.writeInt16LE(
      Math.max(-32768, Math.min(32767, Math.round(v * 32767))),
      i * 2,
    ),
  );
  try {
    if (generation !== workGeneration) throw Error("Cancelled");
    if (pendingLoad) await pendingLoad;
    if (generation !== workGeneration) throw Error("Cancelled");
    if (!activeId) throw Error("Install and select a model first");
    if (inference.device === "unloaded") await use(settings.value.model);
    const prompt = store
      .vocab()
      .map((r) => r.term)
      .join(", ")
      .slice(0, 450);
    const result = await inference.run(pcm, prompt);
    let text = store.apply(result.text, settings.value.autocorrect_after);
    if (settings.value.llm_url && text) {
      try {
        const response = await fetch(settings.value.llm_url, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({
            model: settings.value.llm_model,
            messages: [
              {
                role: "system",
                content:
                  "Correct punctuation and obvious transcription errors. Return only corrected text, preserving meaning.",
              },
              { role: "user", content: text },
            ],
            temperature: 0,
          }),
          signal: AbortSignal.timeout(15000),
        });
        if (!response.ok)
          throw Error("Cleanup endpoint HTTP " + response.status);
        text =
          (
            (await response.json()) as any
          ).choices?.[0]?.message?.content?.trim() || text;
      } catch {
        error = "Cleanup unavailable; using the local transcription.";
      }
    }
    if (!text) {
      status = "Ready";
      return;
    }
    if (generation !== workGeneration) throw Error("Cancelled");
    const sampleId = store.add(
      pcmWav(wav),
      result.text,
      text,
      settings.value.keep_all_audio,
    );
    latestPastedId = undefined;
    recent = text;
    await guardedPaste(
      text,
      focus,
      () =>
        generation === workGeneration ? output.focus() : Promise.resolve(""),
      () => {
        if (generation !== workGeneration) throw Error("Cancelled");
        return output.key(focus, text);
      },
      clip,
    );
    latestPastedId = sampleId;
    logs.log("dictation_complete", { ms: performance.now() - started });
    status = "Pasted";
    setTimeout(() => {
      if (status === "Pasted") {
        status = "Ready";
        broadcast();
      }
    }, 1800);
  } catch (e) {
    fail(e);
  } finally {
    dictationsBusy--;
    broadcast();
  }
}
async function call(method: string, args: any) {
  switch (method) {
    case "state":
      return state();
    case "history":
      return store.rows(args?.search, args?.filter);
    case "use":
      if (dictationsBusy || recording)
        throw Error("Finish or cancel dictation before switching models");
      learning.preempt();
      await use(String(args));
      break;
    case "install":
      void models.install(String(args)).catch(fail);
      break;
    case "cancelDownload":
      models.cancel(String(args));
      break;
    case "startRecording":
      startRecording();
      break;
    case "stopRecording":
      recording = false;
      targets.set(captureGeneration, target);
      captureWindow.webContents.send("capture", {
        type: "stop",
        accept: true,
        id: captureGeneration,
      });
      break;
    case "cancel":
      workGeneration++;
      captureGeneration++;
      targets.clear();
      latestPastedId = undefined;
      output.replaceable = false;
      recording = false;
      captureWindow.webContents.send("capture", {
        type: "stop",
        accept: false,
      });
      models.stop();
      dictations.cancel();
      hold?.cancel();
      engine.stop();
      pythonEngine.stop();
      learning.preempt();
      status = "Cancelled";
      break;
    case "copy":
      await clipboard.writeText(String(args));
      break;
    case "replace": {
      store.correct(args.id, String(args.text)); // Save survives a refused replacement.
      if (
        args.id !== latestPastedId ||
        !output.replaceable ||
        recording ||
        dictationsBusy ||
        replacing
      )
        throw Error(
          "Only the latest verified insertion can be replaced; correction saved only",
        );
      replacing = true;
      const generation = workGeneration;
      try {
        window?.hide();
        // Hiding Settings lets the OS reveal the previous app; never activate a remembered target.
        await new Promise((r) => setTimeout(r, 350));
        if (generation !== workGeneration)
          throw Error("Cancelled; correction saved only");
        await output.replace(String(args.text).trim());
        latestPastedId = undefined;
        status = "Correction replaced";
      } catch (e) {
        fail(e);
        throw e;
      } finally {
        replacing = false;
        broadcast();
      }
      break;
    }
    case "correct":
      store.correct(args.id, String(args.text));
      break;
    case "revert": {
      const row = store.get(args);
      store.correct(args, row.pasted);
      break;
    }
    case "exclude":
      store.exclude(args.id, !!args.value);
      break;
    case "delete":
      store.remove(Number(args));
      break;
    case "audio":
      return fs.readFileSync(store.audioFile(Number(args))).toString("base64");
    case "forget":
      store.forget(String(args));
      break;
    case "settings": {
      if (
        (dictationsBusy || recording) &&
        [
          "model",
          "device",
          "language",
          "onnx_threads",
          "compute_type",
          "beam_size",
        ].some((k) => k in args)
      )
        throw Error(
          "Finish or cancel dictation before changing inference settings",
        );
      const before = settings.value;
      settings.validate(args);
      settings.save(args);
      nativeTheme.themeSource = settings.value.theme as any;
      try {
        if (
          [
            "ptt_key",
            "correct_hotkey",
            "history_hotkey",
            "models_hotkey",
            "min_hold_ms",
          ].some((k) => k in args)
        )
          bind();
      } catch (e) {
        settings.save(before);
        bind();
        throw e;
      }
      if ("input_device" in args || "keep_mic_open" in args)
        captureWindow.webContents.send("capture", {
          type: "configure",
          device: settings.value.input_device,
          keep: settings.value.keep_mic_open,
        });
      if (before.startup !== settings.value.startup)
        setStartup(settings.value.startup);
      if ("model" in args && args.model !== before.model) {
        settings.save({ model: before.model });
        await use(args.model);
      } else if (
        activeId &&
        [
          "device",
          "language",
          "onnx_threads",
          "compute_type",
          "beam_size",
        ].some((k) => (before as any)[k] !== args[k] && k in args)
      )
        await use(settings.value.model);
      break;
    }
    case "exportConfig": {
      const d = await dialog.showSaveDialog({
        defaultPath: "speechify-electron-candidate.toml",
      });
      if (d.filePath) fs.writeFileSync(d.filePath, settings.export());
      break;
    }
    case "importConfig": {
      const d = await dialog.showOpenDialog({
        properties: ["openFile"],
        filters: [{ name: "TOML", extensions: ["toml"] }],
      });
      if (d.filePaths[0]) {
        const { parse } = require("smol-toml");
        await call("settings", parse(fs.readFileSync(d.filePaths[0], "utf8")));
      }
      break;
    }
    case "importData": {
      const d = await dialog.showOpenDialog({
        title: "Copy original Speechify data into candidate",
        properties: ["openDirectory"],
      });
      if (d.filePaths[0]) await store.import(d.filePaths[0]);
      break;
    }
    case "reference": {
      const d = await dialog.showOpenDialog({
        properties: ["openFile"],
        filters: [{ name: "Pinned GGUF model", extensions: ["gguf"] }],
      });
      if (d.filePaths[0]) await models.reference(String(args), d.filePaths[0]);
      break;
    }
    case "export": {
      const d = await dialog.showOpenDialog({
        properties: ["openDirectory", "createDirectory"],
      });
      if (d.filePaths[0])
        return store.export(
          d.filePaths[0],
          settings.value.teacher_min_confidence,
        );
      break;
    }
    case "teacher":
      void learning.runTeacher().catch(fail);
      break;
    case "train":
      void learning.train().catch(fail);
      break;
    case "cancelLearning":
      learning.preempt();
      break;
    case "rollback":
      await learning.rollback();
      break;
    case "logs":
      await shell.openPath(path.join(root, "logs"));
      break;
    case "permission":
      if (process.platform === "darwin") {
        await systemPreferences.askForMediaAccess("microphone");
        systemPreferences.isTrustedAccessibilityClient(true);
      }
      break;
    default:
      throw Error("Unknown action");
  }
  broadcast();
  return state();
}
if (!app.requestSingleInstanceLock()) app.quit();
else {
  app.on("second-instance", () => show());
  app.on("activate", () => show());
  app.on("window-all-closed", () => {});
  app.on("before-quit", () => {
    quitting = true;
    workGeneration++;
    targets.clear();
    dictations.cancel();
    output.stop();
    hold?.cancel();
    killTree(hook);
    globalShortcut.unregisterAll();
    models?.stop();
    learning?.stop();
    engine?.stop();
    pythonEngine?.stop();
    store?.close();
  });
  void app.whenReady().then(() => {
    settings = new Settings(root);
    store = new Store(root);
    logs = new Timings(root);
    models = new Models(path.join(root, "models"), broadcast);
    engine = new Engine(path.join(__dirname, "../resources/native"), logs);
    pythonEngine = new PythonEngine(
      path.join(__dirname, "../resources/python"),
      root,
      logs,
    );
    inference = engine;
    learning = new Learning(
      settings,
      store,
      models,
      path.join(__dirname, "../resources"),
      logs,
      broadcast,
      () =>
        !recording &&
        !dictationsBusy &&
        !pendingLoad &&
        !replacing &&
        !powerMonitor.isOnBatteryPower() &&
        Date.now() - lastKey >= settings.value.idle_minutes * 60000 &&
        powerMonitor.getSystemIdleTime() >= settings.value.idle_minutes * 60,
      () => {
        engine.stop();
        pythonEngine.stop();
      },
      use,
    );
    learning.start();
    powerMonitor.on("on-battery", () => learning.preempt());
    nativeTheme.themeSource = settings.value.theme as any;
    session.defaultSession.setPermissionRequestHandler((wc, permission, cb) =>
      cb(wc === captureWindow?.webContents && permission === "media"),
    );
    session.defaultSession.setPermissionCheckHandler(
      (wc, p) => wc === captureWindow?.webContents && p === "media",
    );
    captureWindow = new BrowserWindow({
      show: false,
      webPreferences: {
        preload: path.join(__dirname, "capture-preload.js"),
        contextIsolation: true,
        nodeIntegration: false,
        sandbox: true,
        backgroundThrottling: false,
      },
    });
    void captureWindow.loadFile(path.join(__dirname, "../ui/capture.html"));
    captureWindow.webContents.on("did-finish-load", () => {
      captureReady = true;
      captureWindow.webContents.send("capture", {
        type: "configure",
        device: settings.value.input_device,
        keep: settings.value.keep_mic_open,
      });
    });
    let audioCrashes = 0;
    captureWindow.webContents.on("render-process-gone", () => {
      captureReady = false;
      targets.clear();
      captureGeneration++;
      recording = false;
      hold?.cancel();
      fail(Error("Audio service crashed; restarting without changing focus"));
      if (++audioCrashes <= 2)
        setTimeout(() => {
          if (!quitting) captureWindow.webContents.reload();
        }, 250);
      else
        fail(
          Error(
            "Audio service repeatedly crashed; restart candidate and check audio permissions",
          ),
        );
    });
    ipcMain.handle("call", (e, m, a) => {
      if (e.sender !== window?.webContents) throw Error("Untrusted sender");
      return call(m, a);
    });
    ipcMain.on("capture-event", (e, event, data) => {
      if (quitting || e.sender !== captureWindow.webContents) return;
      if (event === "audio") {
        if (
          !Array.isArray(data.pcm) ||
          !data.pcm.length ||
          data.pcm.length > 16000 * 120 ||
          !data.pcm.every(
            (n: unknown) => typeof n === "number" && Number.isFinite(n),
          ) ||
          !targets.has(data.id)
        )
          return;
        const focus = targets.get(data.id) || "";
        targets.delete(data.id);
        const generation = workGeneration;
        void dictations
          .run(() => transcribe(data.pcm, focus, generation))
          .catch(fail);
      }
      if (event === "devices") devices = data;
      if (event === "level") {
        level = data;
        if (window && !window.isDestroyed() && window.isVisible())
          window.webContents.send("level", level);
        if (indicator && !indicator.isDestroyed() && indicator.isVisible())
          indicator.webContents.send("level", level);
        return;
      }
      if (event === "error") {
        recording = false;
        hold?.cancel();
        targets.delete(captureGeneration);
        fail(Error(data));
      }
      if (event === "missing-device") {
        settings.save({ input_device: "" });
        error = "Selected microphone missing; using system default";
      }
      if (event === "mic-ready") logs.log("mic_ready");
      broadcast();
    });
    const icon = nativeImage.createFromPath(
      path.join(__dirname, "../resources/tray.png"),
    );
    if (process.platform === "darwin") icon.setTemplateImage(true);
    tray = new Tray(icon);
    tray.setToolTip("Speechify Electron Candidate");
    tray.setContextMenu(
      Menu.buildFromTemplate([
        { label: "Settings", click: () => show("Settings") },
        { label: "History", click: () => show("History") },
        { label: "Models", click: () => show("Models") },
        {
          label: "Logs",
          click: () => void shell.openPath(path.join(root, "logs")),
        },
        { type: "separator" },
        { label: "Quit", click: () => app.quit() },
      ]),
    );
    tray.on("click", () => show());
    indicator = new BrowserWindow({
      width: 260,
      height: 64,
      show: false,
      frame: false,
      focusable: false,
      alwaysOnTop: true,
      skipTaskbar: true,
      resizable: false,
      webPreferences: {
        preload: path.join(__dirname, "preload.js"),
        sandbox: true,
        contextIsolation: true,
        nodeIntegration: false,
      },
    });
    indicator.setIgnoreMouseEvents(true);
    void indicator.loadFile(path.join(__dirname, "../ui/indicator.html"));
    status = "Choose an installed model";
    try {
      bind();
    } catch (e) {
      fail(e);
    }
    show();
    if (models.list().find((m) => m.id === settings.value.model)?.installed)
      void use(settings.value.model).catch(fail);
    logs.log("service_ready");
  });
}
