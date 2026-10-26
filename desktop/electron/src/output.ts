import { randomUUID } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
const exec = promisify(execFile);
export interface ClipboardPort {
  read(): Promise<{
    text: string;
    html: string;
    rtf: string;
    items?: any[];
    stamp?: string;
  }>;
  write(v: {
    text: string;
    html?: string;
    rtf?: string;
    items?: any[];
    stamp?: string;
  }): Promise<void>;
}
export async function guardedPaste(
  text: string,
  target: string,
  current: () => Promise<string>,
  key: () => Promise<void>,
  clip: ClipboardPort,
  delay = (ms: number) => new Promise((r) => setTimeout(r, ms)),
) {
  if (!target || (await current()) !== target)
    throw Error("Focus changed; text saved without pasting");
  const before = await clip.read();
  const stamp = randomUUID();
  await clip.write({ text, stamp });
  try {
    if ((await current()) !== target)
      throw Error("Focus changed; text saved without pasting");
    await key();
    await delay(350);
  } finally {
    const now = await clip.read();
    if (now.stamp === stamp && now.text === text && !now.html && !now.rtf)
      await clip.write(before);
  }
}
export class Output {
  async focus(): Promise<string> {
    if (process.platform === "darwin")
      return (
        await exec("/usr/bin/osascript", [
          "-e",
          'tell application "System Events" to get unix id of first application process whose frontmost is true',
        ])
      ).stdout.trim();
    if (process.platform === "win32")
      return (
        await exec(
          "powershell.exe",
          [
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            `Add-Type 'using System; using System.Runtime.InteropServices; public class W { [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow(); }'; [W]::GetForegroundWindow().ToInt64()`,
          ],
          { windowsHide: true },
        )
      ).stdout.trim();
    if (process.env.XDG_SESSION_TYPE === "wayland")
      throw Error(
        "Wayland global focus/paste requires compositor integration; use Copy text",
      );
    return (await exec("xdotool", ["getactivewindow"])).stdout.trim();
  }
  async key(target: string) {
    if (!/^\d+$/.test(target)) throw Error("Invalid focus target");
    if (process.platform === "darwin") {
      await exec("/usr/bin/osascript", [
        "-e",
        `tell application "System Events"
if (unix id of first application process whose frontmost is true) is not ${target} then error "Focus changed"
key up {control, option, shift, command}
keystroke "v" using command down
end tell`,
      ]);
    } else if (process.platform === "win32") {
      await exec(
        "powershell.exe",
        [
          "-NoProfile",
          "-NonInteractive",
          "-Command",
          `Add-Type 'using System; using System.Runtime.InteropServices; public class W { [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow(); [DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, UIntPtr x); }'; if ([W]::GetForegroundWindow().ToInt64() -ne ${target}) { throw 'Focus changed' }; foreach ($k in 16,17,18,91,92) { [W]::keybd_event($k,0,2,[UIntPtr]::Zero) }; [W]::keybd_event(17,0,0,[UIntPtr]::Zero); [W]::keybd_event(86,0,0,[UIntPtr]::Zero); [W]::keybd_event(86,0,2,[UIntPtr]::Zero); [W]::keybd_event(17,0,2,[UIntPtr]::Zero)`,
        ],
        { windowsHide: true },
      );
    } else {
      if ((await this.focus()) !== target) throw Error("Focus changed");
      await exec("xdotool", ["key", "--clearmodifiers", "ctrl+v"]);
    }
  }
}
