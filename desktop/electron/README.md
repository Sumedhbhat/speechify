# Speechify Electron Candidate

An isolated TypeScript desktop application, not a wrapper around Speechify's Python UI. The release uses Electron 44.9.3, Node's SQLite, and transcribe.cpp 0.3.1 through Koffi in a disposable persistent worker.

The acceptance inventory and known gaps are in [candidate-parity.md](../../docs/candidate-parity.md). Build and device handoff instructions are in [candidate-testing.md](../../docs/candidate-testing.md). Do not treat this candidate as acceptance-complete: global Wayland input/paste and safe cursor replacement remain missing, and platform device checks are pending.

```sh
cd desktop/electron
npm ci
node node_modules/electron/install.js
npm run native
npm test
npm run package
```

This builds **for the local OS and architecture only**. No app is installed or launched by these commands. `npm start` launches the application and activates its global key helper on supported platforms; run that only during coordinated device testing.

Ordinary dictation requires no Python. Optional Kyutai/Distil-Whisper/trained Whisper inference and CUDA fine-tuning use separately installed Python 3.12 environments:

```sh
node scripts/setup-python.mjs inference
node scripts/setup-python.mjs training
```

These commands require `uv` on PATH, install only in `speechify-electron-candidate/environments`, and print the executable paths to enter in Settings. They use universal, hash-locked requirements resolved with a 2026-10-11 cutoff. Native inference and the UI work without either environment. Training needs an NVIDIA CUDA GPU; macOS can use trained models but does not train them.
