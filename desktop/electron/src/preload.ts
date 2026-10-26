import { contextBridge, ipcRenderer } from "electron";
contextBridge.exposeInMainWorld("speechify", {
  call: (method: string, args?: unknown) =>
    ipcRenderer.invoke("call", method, args),
  onLevel: (fn: (level: number) => void) =>
    ipcRenderer.on("level", (_e, n) => fn(n)),
  onState: (fn: (s: any) => void) => {
    ipcRenderer.on("state", (_e, s) => fn(s));
  },
  onNavigate: (fn: (s: string) => void) =>
    ipcRenderer.on("navigate", (_e, s) => fn(s)),
});
