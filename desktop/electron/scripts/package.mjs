import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
const product = "Speechify Electron Candidate",
  slug = "speechify-electron-candidate",
  id = "org.sumedhbhat.speechify.electron";
function run(cmd, args) {
  const r = spawnSync(cmd, args, { stdio: "inherit" });
  if (r.error) throw r.error;
  if (r.status) throw Error(cmd + " failed " + r.status);
}
if (!fs.existsSync("resources/native/provenance.json"))
  throw Error("Run npm run native first");
const prov = JSON.parse(fs.readFileSync("resources/native/provenance.json"));
if (prov.target !== process.platform + "-" + process.arch)
  throw Error("Native resources are for another platform");
const dest = path.resolve(
  "artifacts",
  slug + "-" + process.platform + "-" + process.arch,
);
fs.rmSync(dest, { recursive: true, force: true });
fs.mkdirSync(dest, { recursive: true });
fs.cpSync("node_modules/electron/dist", dest, { recursive: true });
let appRoot;
if (process.platform === "darwin") {
  const bundle = path.join(dest, product + ".app");
  fs.renameSync(path.join(dest, "Electron.app"), bundle);
  const contents = path.join(bundle, "Contents");
  fs.renameSync(
    path.join(contents, "MacOS/Electron"),
    path.join(contents, "MacOS", slug),
  );
  // Electron distributes binary plists on some versions: use Apple's plist tooling.
  const plist = path.join(contents, "Info.plist");
  const set = (key, value) => {
    spawnSync("/usr/libexec/PlistBuddy", ["-c", `Delete :${key}`, plist], {
      stdio: "ignore",
    });
    run("/usr/libexec/PlistBuddy", [
      "-c",
      `Add :${key} string ${value}`,
      plist,
    ]);
  };
  const add = (key, value) =>
    run("/usr/libexec/PlistBuddy", [
      "-c",
      `Add :${key} string ${value}`,
      plist,
    ]);
  set("CFBundleIdentifier", id);
  set("CFBundleName", product);
  set("CFBundleDisplayName", product);
  set("CFBundleExecutable", slug);
  set("CFBundleShortVersionString", "0.1.0");
  set("CFBundleVersion", "1");
  // Delete existing usage strings if supplied by a later Electron build, then add.
  for (const [key, value] of Object.entries({
    NSMicrophoneUsageDescription:
      "Hold your configured talk key to dictate into the focused application.",
    NSAppleEventsUsageDescription:
      "Speechify checks the focused text control before inserting dictation.",
  })) {
    spawnSync("/usr/libexec/PlistBuddy", ["-c", `Delete :${key}`, plist], {
      stdio: "ignore",
    });
    add(key, value);
  }
  const rewriteHelpers = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const f = path.join(dir, entry.name);
      if (entry.isDirectory()) rewriteHelpers(f);
      else if (entry.name === "Info.plist") {
        const result = spawnSync(
          "/usr/libexec/PlistBuddy",
          ["-c", "Print :CFBundleIdentifier", f],
          { encoding: "utf8" },
        );
        if (
          result.status === 0 &&
          result.stdout.trim().startsWith("com.github.Electron")
        )
          run("/usr/libexec/PlistBuddy", [
            "-c",
            "Set :CFBundleIdentifier " +
              result.stdout.trim().replace("com.github.Electron", id),
            f,
          ]);
      }
    }
  };
  rewriteHelpers(path.join(contents, "Frameworks"));
  appRoot = path.join(contents, "Resources/app");
} else {
  fs.renameSync(
    path.join(dest, process.platform === "win32" ? "electron.exe" : "electron"),
    path.join(dest, slug + (process.platform === "win32" ? ".exe" : "")),
  );
  appRoot = path.join(dest, "resources/app");
}
fs.mkdirSync(appRoot, { recursive: true });
for (const n of ["dist", "ui", "resources"])
  fs.cpSync(n, path.join(appRoot, n), { recursive: true });
fs.copyFileSync("package.json", path.join(appRoot, "package.json"));
fs.copyFileSync("package-lock.json", path.join(appRoot, "package-lock.json"));
fs.mkdirSync(path.join(appRoot, "node_modules"), { recursive: true });
for (const n of [
  "koffi",
  "@koromix",
  "smol-toml",
  "uiohook-napi",
  "node-gyp-build",
])
  if (fs.existsSync("node_modules/" + n))
    fs.cpSync("node_modules/" + n, path.join(appRoot, "node_modules", n), {
      recursive: true,
    });
if (process.platform === "linux") {
  const filename = slug + "-linux-" + process.arch + ".tar.gz";
  run("tar", [
    "-czf",
    path.resolve("artifacts", filename),
    "-C",
    path.dirname(dest),
    path.basename(dest),
  ]);
  const deb = path.resolve("artifacts/deb-root");
  fs.rmSync(deb, { recursive: true, force: true });
  fs.mkdirSync(path.join(deb, "opt", slug), { recursive: true });
  fs.cpSync(dest, path.join(deb, "opt", slug), { recursive: true });
  fs.mkdirSync(path.join(deb, "DEBIAN"), { recursive: true });
  fs.writeFileSync(
    path.join(deb, "DEBIAN/control"),
    `Package: ${slug}\nVersion: 0.1.0\nArchitecture: ${process.arch === "arm64" ? "arm64" : "amd64"}\nMaintainer: Speechify Candidate\nDescription: Isolated Electron dictation comparison candidate\nDepends: libgtk-3-0, libnss3, libasound2t64 | libasound2, libxtst6, libx11-6, xdotool\n`,
  );
  fs.chmodSync(path.join(deb, "opt", slug, "chrome-sandbox"), 0o4755);
  const share = path.join(deb, "usr/share/applications");
  fs.mkdirSync(share, { recursive: true });
  fs.writeFileSync(
    path.join(share, id + ".desktop"),
    `[Desktop Entry]\nName=${product}\nExec=/opt/${slug}/${slug}\nType=Application\nCategories=AudioVideo;Utility;\nStartupWMClass=${slug}\n`,
  );
  run("dpkg-deb", [
    "--root-owner-group",
    "--build",
    deb,
    path.resolve("artifacts", slug + "-linux-" + process.arch + ".deb"),
  ]);
  fs.rmSync(deb, { recursive: true, force: true });
}
if (process.platform === "darwin") {
  const bundle = path.join(dest, product + ".app");
  const identity = process.env.SPEECHIFY_SIGN_IDENTITY || "-";
  const sign = (file, entitlements = false) =>
    run("codesign", [
      "--force",
      "--sign",
      identity,
      ...(entitlements
        ? ["--entitlements", "resources/entitlements.mac.plist"]
        : []),
      ...(identity === "-" ? [] : ["--options", "runtime", "--timestamp"]),
      file,
    ]);
  // Sign nested Mach-O resources inside-out. --deep alone can miss Node addons
  // and libraries stored in Resources rather than standard bundle code folders.
  const signNested = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (entry.isSymbolicLink()) continue;
      const f = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        signNested(f);
        if (/\.(app|framework|xpc)$/.test(entry.name))
          sign(f, entry.name.endsWith(".app"));
      } else {
        const fd = fs.openSync(f, "r"),
          magic = Buffer.alloc(4);
        try {
          fs.readSync(fd, magic, 0, 4, 0);
        } finally {
          fs.closeSync(fd);
        }
        if (
          [
            "cffaedfe",
            "cefaedfe",
            "cafebabe",
            "bebafeca",
            "feedfacf",
            "feedface",
          ].includes(magic.toString("hex"))
        )
          sign(f, f.endsWith("focus-helper"));
      }
    }
  };
  signNested(path.join(bundle, "Contents"));
  sign(bundle, true);
  run("codesign", ["--verify", "--deep", "--strict", "--verbose=2", bundle]);
  const dmg = path.resolve("artifacts", slug + "-mac-" + process.arch + ".dmg");
  fs.rmSync(dmg, { force: true });
  run("hdiutil", [
    "create",
    "-volname",
    product,
    "-srcfolder",
    dest,
    "-ov",
    "-format",
    "UDZO",
    dmg,
  ]);
}
if (process.platform === "win32") {
  process.env.SPEECHIFY_PACKAGE = dest;
  process.env.SPEECHIFY_ARCHIVE = path.resolve(
    "artifacts",
    slug + "-windows-" + process.arch + ".zip",
  );
  run("powershell.exe", [
    "-NoProfile",
    "-Command",
    "Compress-Archive -Path $env:SPEECHIFY_PACKAGE -DestinationPath $env:SPEECHIFY_ARCHIVE -Force",
  ]);
}
const files = fs
  .readdirSync("artifacts")
  .filter((n) => /\.(tar.gz|deb|dmg|zip|exe)$/.test(n));
let manifest = "";
for (const n of files) {
  const h = createHash("sha256");
  for await (const chunk of fs.createReadStream(path.join("artifacts", n)))
    h.update(chunk);
  manifest += h.digest("hex") + "  " + n + "\n";
}
fs.writeFileSync("artifacts/SHA256SUMS", manifest);
console.log(dest);
