import { execFileSync } from "node:child_process";
import { dirname, relative, resolve } from "node:path";

const root = resolve(import.meta.dirname, "../..");
const run = (bin, args) =>
  execFileSync(bin, args, {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    shell: process.platform === "win32",
  });

const moon = process.platform === "win32" ? "moon.cmd" : "moon";
const out = run(resolve(root, "node_modules/.bin", moon), ["query", "projects"]);
const projects = JSON.parse(out.slice(out.indexOf("{"))).projects;
const bySource = new Map(projects.map((project) => [project.source, project]));

const metadata = JSON.parse(
  run("cargo", ["metadata", "--no-deps", "--format-version", "1", "--offline"]),
);
const dirOf = (manifest) => relative(root, dirname(manifest)).split("\\").join("/");
const crates = new Map(metadata.packages.map((pkg) => [dirOf(pkg.manifest_path), pkg]));

const problems = [];
for (const [dir, pkg] of crates) {
  const project = bySource.get(dir);
  if (!project) {
    problems.push(`${dir}/Cargo.toml has no moon.yml listed in .moon/workspace.yml`);
    continue;
  }
  const cargo = new Set(
    pkg.dependencies
      .filter((dep) => dep.path)
      .map((dep) => bySource.get(relative(root, dep.path).split("\\").join("/"))?.id ?? dep.name),
  );
  const declared = new Set(
    (project.dependencies ?? [])
      .map((dep) => dep.id)
      .filter((id) => crates.has(projects.find((p) => p.id === id).source)),
  );
  for (const id of cargo) {
    if (!declared.has(id)) problems.push(`${dir}/moon.yml: add "${id}" to dependsOn (Cargo.toml uses it)`);
  }
  for (const id of declared) {
    if (!cargo.has(id)) problems.push(`${dir}/moon.yml: "${id}" is in dependsOn but not in Cargo.toml`);
  }
}

if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log(`moon's project graph matches Cargo for ${crates.size} crates.`);
