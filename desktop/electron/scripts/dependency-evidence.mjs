import fs from "node:fs";
const lock = JSON.parse(fs.readFileSync("package-lock.json"));
const results = [];
const entries = Object.entries(lock.packages).filter(([key]) =>
  key.startsWith("node_modules/"),
);
let cursor = 0;
await Promise.all(
  Array.from({ length: 6 }, async () => {
    while (cursor < entries.length) {
      const [key, pkg] = entries[cursor++];
      const name = key.slice(key.lastIndexOf("node_modules/") + 13);
      const r = await fetch(
        "https://registry.npmjs.org/" + encodeURIComponent(name),
      );
      if (!r.ok) throw Error("Registry lookup failed: " + name);
      const meta = await r.json();
      const published = meta.time[pkg.version];
      results.push({
        name,
        version: pkg.version,
        published,
        ageRule:
          published < "2026-10-11T00:00:00Z"
            ? "passed"
            : name === "electron"
              ? "security exception GHSA-jr5w-w6x2-84p9"
              : name === "undici"
                ? "security exception GHSA-xjj6-f33q-929c, GHSA-4qv8-2p9j-2685, GHSA-f72v-6w45-q472"
                : "FAILED",
      });
    }
  }),
);
results.sort((a, b) => a.name.localeCompare(b.name));
fs.writeFileSync(
  "dependency-evidence.json",
  JSON.stringify(
    { cutoff: "2026-10-11T00:00:00Z", dependencies: results },
    null,
    2,
  ) + "\n",
);
const failed = results.filter((r) => r.ageRule === "FAILED");
console.log(JSON.stringify(failed));
if (failed.length) process.exitCode = 1;
