import { execFileSync } from "node:child_process";
import { appendFileSync, mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

const STEPS = {
  lint: ["lint", "test-report"],
  test: ["test", "build"],
  coverage: ["coverage"],
};
const TARGETS = [...Object.values(STEPS).flat(), "release", "measure"];
const env = process.env;
const moonBin = path.join(
  "node_modules",
  ".bin",
  process.platform === "win32" ? "moon.cmd" : "moon",
);

function moon(args) {
  const { SPEECHIFY_REQUIRE_NATIVE, ...rest } = env;
  const out = execFileSync(moonBin, args, {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    shell: process.platform === "win32",
    stdio: ["ignore", "pipe", "inherit"],
    env: { ...rest, MOON_COLOR: "false" },
  });
  return JSON.parse(out.slice(out.indexOf("{")));
}

function commitExists(sha) {
  if (!sha || /^0+$/.test(sha)) return false;
  try {
    execFileSync("git", ["cat-file", "-e", `${sha}^{commit}`], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

function isAncestor(sha) {
  try {
    execFileSync("git", ["merge-base", "--is-ancestor", sha, "HEAD"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

function lastMeasured() {
  const gh = (args) =>
    execFileSync("gh", args, { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] });
  try {
    const found = gh([
      "api",
      "--paginate",
      `repos/${env.GITHUB_REPOSITORY}/releases?per_page=100`,
      "--jq",
      '.[] | select((.draft | not) and (.tag_name | test("-nightly[.]")) and any(.assets[]; .name == "performance.json")) | .tag_name',
    ]);
    const tag = found.split("\n")[0].trim();
    if (!tag) return null;
    const dir = mkdtempSync(path.join(tmpdir(), "baseline-"));
    gh(["release", "download", tag, "--repo", env.GITHUB_REPOSITORY, "--pattern", "performance.json", "--dir", dir]);
    const sha = JSON.parse(readFileSync(path.join(dir, "performance.json"), "utf8")).head_sha;
    return { tag, sha };
  } catch {
    return null;
  }
}

const taskName = (id) => id.slice(id.lastIndexOf(":") + 1);

function everyTask() {
  const { tasks } = moon(["query", "tasks"]);
  const ids = Object.entries(tasks).flatMap(([project, list]) =>
    Object.keys(list).map((task) => `${project}:${task}`),
  );
  return Object.fromEntries(
    ids.filter((id) => TARGETS.includes(taskName(id))).map((id) => [id, {}]),
  );
}

function affectedTasks(base) {
  const { tasks = {} } = moon(["query", "affected", `${base}:HEAD`, "--downstream", "deep"]);
  return Object.fromEntries(Object.entries(tasks).filter(([id]) => TARGETS.includes(taskName(id))));
}

function why(state) {
  const files = [...(state.files ?? [])].sort();
  const shown = files.slice(0, 3).map((f) => `\`${f}\``);
  if (files.length > 3) shown.push(`and ${files.length - 3} more`);
  for (const name of state.env ?? []) shown.push(`\`$${name}\` is set`);
  for (const id of state.upstream ?? []) shown.push(`\`${id}\` is affected`);
  if (!shown.length) shown.push(prBase ? "affected" : "everything");
  return shown.join(", ");
}

const event = env.EVENT_NAME;
const onMain = env.GIT_REF === "refs/heads/main";
const prBase = event === "pull_request" && commitExists(env.PR_BASE) ? env.PR_BASE : null;
const tasks = prBase ? affectedTasks(prBase) : everyTask();

let measure = false;
let measureWhy = "measured only on pull requests and on main";
if (prBase) {
  measure = "perf:measure" in tasks;
  measureWhy = measure ? why(tasks["perf:measure"]) : "nothing it measures changed";
} else if (event === "pull_request") {
  measure = true;
  measureWhy = "the base commit is not in this clone";
} else if (onMain) {
  const last = lastMeasured();
  if (last && commitExists(last.sha) && isAncestor(last.sha)) {
    const since = affectedTasks(last.sha);
    measure = "perf:measure" in since;
    measureWhy = measure
      ? why(since["perf:measure"])
      : `nothing it measures changed since \`${last.sha.slice(0, 10)}\`, measured for ${last.tag}`;
  } else {
    measure = true;
    measureWhy = last ? "the last measured commit is not an ancestor of this one" : "no earlier measurement to compare with";
  }
}

const ids = Object.keys(tasks).sort();
const outputs = {};
for (const [step, names] of Object.entries(STEPS)) {
  const selected = ids.filter((id) => names.includes(taskName(id)));
  outputs[step] = selected.length > 0;
  outputs[`${step}_tasks`] = selected.join(" ");
}
outputs.fixtures = "core:test" in tasks;
outputs.native = ids.some((id) => id.startsWith("desktop:"));
outputs.measure = measure;
for (const [key, value] of Object.entries(outputs)) {
  appendFileSync(env.GITHUB_OUTPUT ?? "/dev/stdout", `${key}=${value}\n`);
}

const rows = ids.map((id) => `| \`${id}\` | ${why(tasks[id])} |`);
const summary = [
  `### What this run checks`,
  ``,
  prBase
    ? `Tasks whose inputs changed since \`${prBase.slice(0, 10)}\`, found by \`moon query affected\`.`
    : event === "pull_request"
      ? `Every task: the base commit is not in this clone.`
      : `Every task: this is not a pull request.`,
  ``,
  `Performance gate: **${measure ? "runs" : "skipped"}** (${measureWhy}).`,
  ``,
  ...(rows.length ? [`| Task | Why |`, `| --- | --- |`, ...rows] : [`Nothing to run.`]),
  ``,
].join("\n");
if (env.GITHUB_STEP_SUMMARY) appendFileSync(env.GITHUB_STEP_SUMMARY, summary);
else console.log(summary);
