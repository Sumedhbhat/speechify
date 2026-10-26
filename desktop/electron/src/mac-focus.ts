import { spawn, ChildProcess } from "node:child_process";
import { createInterface } from "node:readline";
import path from "node:path";
import { killTree } from "./python-engine";
// One helper retains real AX object identity across focus checks. No transcript logging.
export class MacFocus {
  private child?: ChildProcess;
  private id = 0;
  private pending = new Map<
    number,
    {
      resolve: (v: any) => void;
      reject: (e: Error) => void;
      timer: NodeJS.Timeout;
    }
  >();
  request(type: string, args: any = {}): Promise<any> {
    if (!this.child) {
      const c = (this.child = spawn(
        path.join(__dirname, "../resources/macos/focus-helper"),
        [],
        {
          stdio: ["pipe", "pipe", "ignore"],
          detached: true,
        },
      ));
      createInterface({ input: c.stdout! }).on("line", (line) => {
        try {
          const m = JSON.parse(line),
            p = this.pending.get(m.id);
          if (p) {
            clearTimeout(p.timer);
            this.pending.delete(m.id);
            m.error ? p.reject(Error(m.error)) : p.resolve(m);
          }
        } catch {
          this.stop();
        }
      });
      c.on("error", () => {
        if (this.child === c) this.stop();
      });
      c.on("exit", () => {
        if (this.child === c) this.stop();
      });
      c.stdin!.on("error", () => {
        if (this.child === c) this.stop();
      });
    }
    return new Promise((resolve, reject) => {
      const id = ++this.id;
      const timer = setTimeout(() => this.stop(), 5000);
      this.pending.set(id, { resolve, reject, timer });
      this.child!.stdin!.write(JSON.stringify({ id, type, ...args }) + "\n");
    });
  }
  stop() {
    const c = this.child;
    this.child = undefined;
    killTree(c);
    for (const p of this.pending.values()) {
      clearTimeout(p.timer);
      p.reject(
        Error(
          "macOS focus helper unavailable; rebuild locally and check Accessibility permission",
        ),
      );
    }
    this.pending.clear();
  }
}
