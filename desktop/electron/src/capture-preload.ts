import { contextBridge, ipcRenderer } from "electron";
contextBridge.exposeInMainWorld("capture", {
  on: (fn: (s: any) => void) => ipcRenderer.on("capture", (_e, s) => fn(s)),
  send: (event: string, data: any) =>
    ipcRenderer.send("capture-event", event, data),
});
