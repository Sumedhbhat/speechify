#!/usr/bin/env python3
"""Type-check platform FFI adapters on a non-native host (not an OS/build test).
Only cfg/link annotations are adapted; function bodies and FFI signatures are intact.
Windows child process extensions still require a native Windows build.
"""
import pathlib, re, subprocess, tempfile
src = pathlib.Path(__file__).resolve().parents[1] / 'src-tauri/src'
with tempfile.TemporaryDirectory(prefix='speechify-adapter-check-') as tmp:
    root = pathlib.Path(tmp)
    for name in ['platform', 'replacement', 'mac_clipboard']:
        text = (src / f'{name}.rs').read_text()
        text = text.replace('target_os = "linux"', 'candidate_linux').replace('target_os = "macos"', 'candidate_macos')
        text = re.sub(r'\bwindows\b(?=[,)])', 'candidate_windows', text)
        text = re.sub(r',\s*kind\s*=\s*"framework"', '', text)
        (root / f'{name}.rs').write_text(text)
    (root / 'check.rs').write_text('mod config { pub type Result<T> = std::result::Result<T,String>; }\nmod platform;\nmod replacement;\n#[cfg(candidate_macos)] mod mac_clipboard;\nfn main() {}\n')
    for platform in ['windows', 'macos']:
        subprocess.run(['rustc', '--edition=2021', '--emit=metadata', '--cfg', f'candidate_{platform}', '-A', 'dead_code', str(root / 'check.rs'), '-o', str(root / 'check.rmeta')], check=True)
        print(f'{platform}: adapter Rust type check passed (host metadata only; no native linking/runtime)', flush=True)
