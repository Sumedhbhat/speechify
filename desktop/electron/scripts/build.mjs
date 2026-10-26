import { spawnSync } from "node:child_process";
import fs from "node:fs";
const result = spawnSync(
  process.execPath,
  ["node_modules/typescript/bin/tsc"],
  { stdio: "inherit" },
);
if (result.error) throw result.error;
if (result.status) process.exit(result.status);
const browser = spawnSync(
  process.execPath,
  ["node_modules/typescript/bin/tsc", "-p", "tsconfig.browser.json"],
  { stdio: "inherit" },
);
if (browser.error) throw browser.error;
if (browser.status) process.exit(browser.status);
for (const name of fs.readdirSync("src").filter((n) => n.endsWith(".json")))
  fs.copyFileSync("src/" + name, "dist/" + name);

if (process.platform === "darwin") {
  const helper = spawnSync(
    "xcrun",
    [
      "swiftc",
      "-O",
      "-framework",
      "Cocoa",
      "-framework",
      "ApplicationServices",
      "resources/macos/focus.swift",
      "-o",
      "resources/macos/focus-helper",
    ],
    { stdio: "inherit" },
  );
  if (helper.error) throw helper.error;
  if (helper.status !== 0) process.exit(helper.status || 1);
}
