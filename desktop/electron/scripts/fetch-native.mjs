import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
const pins = {
  "linux-x64": [
    "manylinux_2_27_x86_64.manylinux_2_28_x86_64",
    "9647bff7a7486810c2778fb9b86a6ac0302688620bfad8f31b560697423641aa",
  ],
  "linux-arm64": [
    "manylinux_2_27_aarch64.manylinux_2_28_aarch64",
    "cffc38c389fb5933270c6b7e282c18dcb5b88798005c021a4f6644372d3f9ffa",
  ],
  "darwin-arm64": [
    "macosx_11_0_arm64",
    "8b5fe9f76fcb88d64981c55837f74bd0ccba013ac7f1487216a59a1ef8dadc82",
  ],
  "darwin-x64": [
    "macosx_13_0_x86_64",
    "a03335e8512b0f8a35ed6b3409beca6530161060dbf5f6d31615cca51edf3abd",
  ],
  "win32-x64": [
    "win_amd64",
    "8bb3e962cf9405054129a8ca9ecf9c9df7f4f044b9d6467bf94d5823ff1c9533",
  ],
};
const key = process.platform + "-" + process.arch,
  pin = pins[key];
if (!pin) throw Error("Unsupported native target " + key);
const response = await fetch(
  "https://pypi.org/pypi/transcribe-cpp-native/0.3.1/json",
);
if (!response.ok) throw Error("PyPI metadata failed");
const metadata = await response.json();
const artifact = metadata.urls.find((f) => f.digests.sha256 === pin[1]);
if (!artifact) throw Error("Pinned wheel absent");
const result = await fetch(artifact.url);
if (!result.ok) throw Error("Native download failed");
const data = Buffer.from(await result.arrayBuffer());
if (createHash("sha256").update(data).digest("hex") !== pin[1])
  throw Error("Native wheel checksum mismatch");
const temp = fs.mkdtempSync(
  path.join(os.tmpdir(), "speechify-electron-native-"),
);
try {
  fs.writeFileSync(path.join(temp, "native.zip"), data);
  const unpack = path.join(temp, "unpacked");
  let r;
  if (process.platform === "win32")
    r = spawnSync(
      "powershell.exe",
      [
        "-NoProfile",
        "-Command",
        "Expand-Archive -LiteralPath $env:SPEECHIFY_ZIP -DestinationPath $env:SPEECHIFY_UNPACK",
      ],
      {
        env: {
          ...process.env,
          SPEECHIFY_ZIP: path.join(temp, "native.zip"),
          SPEECHIFY_UNPACK: unpack,
        },
        stdio: "inherit",
      },
    );
  else
    r = spawnSync(
      "unzip",
      ["-q", path.join(temp, "native.zip"), "-d", unpack],
      { stdio: "inherit" },
    );
  if (r.status) throw Error("Native extraction failed");
  fs.mkdirSync("resources/native", { recursive: true });
  fs.cpSync(
    path.join(unpack, "transcribe_cpp_native/_native"),
    "resources/native",
    { recursive: true },
  );
  const info = fs.readdirSync(unpack).find((n) => n.endsWith(".dist-info"));
  fs.cpSync(path.join(unpack, info, "licenses"), "resources/native/licenses", {
    recursive: true,
  });
  fs.writeFileSync(
    "resources/native/provenance.json",
    JSON.stringify(
      { version: "0.3.1", target: key, url: artifact.url, sha256: pin[1] },
      null,
      2,
    ),
  );
} finally {
  fs.rmSync(temp, { recursive: true, force: true });
}
