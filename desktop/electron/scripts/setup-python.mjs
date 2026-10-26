// Explicit, optional install. Invoked by the user, never during ordinary startup.
import os from "node:os";
import path from "node:path";
import fs from "node:fs";
import { spawnSync } from "node:child_process";
const kind = process.argv[2];
if (!["inference", "training"].includes(kind))
  throw Error("Usage: node scripts/setup-python.mjs inference|training");
const base =
  process.platform === "darwin"
    ? path.join(os.homedir(), "Library/Application Support")
    : process.platform === "win32"
      ? process.env.APPDATA
      : process.env.XDG_DATA_HOME || path.join(os.homedir(), ".local/share");
const env = path.join(
  base,
  "speechify-electron-candidate",
  "environments",
  kind,
);
fs.mkdirSync(path.dirname(env), { recursive: true });
function run(args) {
  const r = spawnSync("uv", args, { stdio: "inherit" });
  if (r.error)
    throw Error(
      "Install uv 0.13.0 or newer and Python 3.12, then retry: " +
        r.error.message,
    );
  if (r.status) process.exit(r.status);
}
run(["venv", "--python", "3.12", env]);
const python = path.join(
  env,
  process.platform === "win32" ? "Scripts/python.exe" : "bin/python",
);
run([
  "pip",
  "sync",
  "--python",
  python,
  "--require-hashes",
  path.resolve("resources/python/requirements", kind + ".lock"),
]);
console.log("Set " + kind + "_python in Settings to:\n" + python);
