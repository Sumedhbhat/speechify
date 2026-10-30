"""Check that the collector sees code executed in a child before exec."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


@unittest.skipUnless(sys.platform == "linux", "Linux continuous profiling")
class CoverageRuntime(unittest.TestCase):
    def test_pre_exec_counts_survive_replacing_the_child_process_image(self):
        with tempfile.TemporaryDirectory(prefix="speechify-coverage-runtime-") as directory:
            root = Path(directory)
            source = root / "probe.rs"
            source.write_text("""use std::os::unix::process::CommandExt;
fn main() {
    let mut command = std::process::Command::new("/usr/bin/true");
    unsafe { command.pre_exec(|| Ok(())); }
    assert!(command.status().unwrap().success());
}
""")
            executable = root / "probe"
            command = [
                "rustc",
                "--edition=2021",
                "-Cinstrument-coverage",
                "-Cllvm-args=-runtime-counter-relocation",
                str(source),
                "-o",
                str(executable),
            ]
            if linker := os.environ.get("CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER"):
                command += ["-C", f"linker={linker}"]
            subprocess.run(command, check=True, capture_output=True)
            subprocess.run(
                [str(executable)],
                check=True,
                env={**os.environ, "LLVM_PROFILE_FILE": str(root / "%p-%m%c.profraw")},
            )
            sysroot = Path(
                subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip()
            )
            host = next(
                line.split(": ", 1)[1]
                for line in subprocess.check_output(["rustc", "-vV"], text=True).splitlines()
                if line.startswith("host:")
            )
            tools = sysroot / "lib/rustlib" / host / "bin"
            profile = root / "merged.profdata"
            subprocess.run(
                [
                    str(tools / "llvm-profdata"),
                    "merge",
                    "-sparse",
                    *map(str, root.glob("*.profraw")),
                    "-o",
                    str(profile),
                ],
                check=True,
            )
            report = json.loads(
                subprocess.check_output(
                    [
                        str(tools / "llvm-cov"),
                        "export",
                        str(executable),
                        f"-instr-profile={profile}",
                    ]
                )
            )
            functions = report["data"][0]["functions"]
            self.assertEqual(len(functions), 2)
            self.assertTrue(all(function["count"] > 0 for function in functions), functions)


if __name__ == "__main__":
    unittest.main()
