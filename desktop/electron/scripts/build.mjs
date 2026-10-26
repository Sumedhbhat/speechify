import { spawnSync } from "node:child_process";
import fs from "node:fs";
const result = spawnSync(
  process.execPath,
  ["node_modules/typescript/bin/tsc"],
  { stdio: "inherit" },
);
if (result.status) process.exit(result.status);
for (const name of fs.readdirSync("src").filter((n) => n.endsWith(".json")))
  fs.copyFileSync("src/" + name, "dist/" + name);
