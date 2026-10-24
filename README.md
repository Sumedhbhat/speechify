# speechify

Hold a key, talk, let go, and the text is pasted wherever your cursor is. It
runs locally on Windows, Linux and macOS, on whatever GPU the machine has
(NVIDIA, AMD, Intel or Apple Silicon), lets you pick from 14 speech models,
and gets better as you correct it.

## Install

Download the installer for your system from the
[latest release](https://github.com/Sumedhbhat/speechify/releases/latest):

| System | File | |
| --- | --- | --- |
| Windows 10 and 11 | `Speechify-<version>-windows-x64-setup.exe` | Run it. No admin rights needed. |
| macOS 11+, Apple silicon | `Speechify-<version>-macos-arm64.dmg` | Open it and drag Speechify to Applications. Intel Macs can run it from source (below). |
| Ubuntu, Debian, Mint, Pop!_OS | `speechify_<version>_amd64.deb` | Open it with Software Install, or `sudo apt install ./speechify_*.deb`. |
| Any other Linux | `Speechify-<version>-x86_64.AppImage` | Make it executable (`chmod +x`) and run it. |

To update, download and install the new version over the old one. Your
recordings, corrections, settings and models are kept. Nightly builds are on the
[releases page](https://github.com/Sumedhbhat/speechify/releases) as
prereleases; see [docs/releases.md](docs/releases.md).

Each installer carries everything Speechify needs to start, including its own
Python. The first start downloads the selected speech model and creates a
config file (`speechify where` prints its folder). You don't need to install
CUDA; each model's CUDA libraries come from pip. A recent NVIDIA driver is all
you need.

**On Windows**, Speechify is in the Start menu, and the installer can also start
it when you sign in. It runs in a minimized console window that shows each
dictation; close that window to quit. New terminals get a `speechify` command.

**On a Mac**, the app isn't signed with an Apple Developer ID yet, so the
first time you open it macOS says it can't check it for malware. Open System
Settings > Privacy & Security, scroll down and click **Open Anyway**. To start
it when you log in, add it under System Settings > General > Login Items.

Push to talk on a Mac is **Right Option**, since Mac keyboards have no Right
Ctrl. The first start asks for Accessibility (to see the talk key and to
paste); allow it, then start Speechify again. The first time you hold the talk
key, macOS asks for the Microphone too.
Until the app is signed, macOS treats each update as a new app and asks for
these again.

**On Linux**, Speechify is in your app menu (the AppImage adds itself the first
time you run it). The `.deb` also adds a `speechify` command. Pasting needs
`wl-clipboard` (Wayland) or `xclip` (X11); the `.deb` installs it, and the
AppImage tells you if it's missing.

With Wayland (the default on Ubuntu and Fedora), apps can't see
each other's keys, so Speechify reads the keyboard from `/dev/input` and pastes
through a virtual keyboard. That needs your user in the `input` group. The
first start offers to set it up, asking for your password, and then carries
on; from the next login it's permanent. On X11 nothing extra is needed.

Without an NVIDIA GPU, the first start picks NVIDIA Parakeet TDT 0.6B v3
instead of Whisper, because it's the fastest accurate model there. On an AMD
or Intel GPU (built in or separate) it runs through Vulkan, and on Apple
Silicon through Metal, using [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp),
the engine Handy uses: a 0.7 GB download. With no usable GPU at all it runs on
the CPU with int8 weights, also 0.7 GB. Linux PCs with the NVIDIA driver get
the same CUDA setup as Windows. See [GPUs](#gpus) for what runs where.

To run from a source checkout instead, install
[uv](https://docs.astral.sh/uv/), then `uv sync` and `start.bat` on Windows, or
`uv sync --no-install-package evdev` and `./start.sh` on Linux and macOS.

## Using it

Speechify runs in the background. Its icon (in the tray on Windows and Linux,
the menu bar on a Mac) opens the settings, the history of your dictations, the
model picker and the log, and quits it. The icon turns red while you hold the
talk key. Starting Speechify again while it's running opens its settings, so
the Start menu, app menu or Dock entry always gets you there. The first start
opens the settings with a short introduction.

A pill at the bottom of the screen shows what Speechify is doing: bars that
move with your voice while you hold the key, a wave of dots while it
transcribes, then a check mark with how long it took. The microphone is only
open while you hold the key, so the system's mic indicator lights up only then.

| Do this | What happens |
| --- | --- |
| Hold **Right Ctrl** (Mac: **Right Option**) and talk, then let go | Transcribes and pastes into the focused app |
| Tap the key, or use it in a shortcut (Ctrl+C) | Ignored. Nothing is recorded |
| **Ctrl+Alt+F** (Mac: **Control+Option+F**) | Opens the last dictation for fixing |
| **Ctrl+Alt+H** (Mac: **Control+Option+H**) | Opens every saved recording |
| **Ctrl+Alt+M** (Mac: **Control+Option+M**) | Opens the model picker |

In the fix window, type the correction or hold Right Ctrl and say it. Then:

- **Ctrl+Enter** saves the fix and also swaps the text in the app you pasted into
- **Shift+Enter** just saves it

All of these keys can be changed in the settings.

## Settings

Open **Settings…** from the tray or menu bar icon. Everything there is saved to
`config.toml` and applied straight away, without a restart:

- **General**: the talk key (pick one, or press **Press a key…** and press it),
  the shortest press that counts, and starting Speechify when you sign in.
- **Microphone**: which microphone, and whether to keep it open all the time.
- **Shortcuts**: the fix, history and model picker shortcuts.
- **Model**: the speech model, the device it runs on, the language, Whisper's
  beam size and the optional LLM cleanup. Changing these reloads the model.
- **Learning**: saving audio, automatic fixes, and retraining while you're away.

`config.toml` can still be edited by hand (**Open data folder** in the
settings); restart Speechify afterwards.

## Models

Open the model picker with **Ctrl+Alt+M**, or run `speechify models`. It lists
every model with its accuracy and speed rating, the GPU memory it needs, and
whether it's already on this PC. Tick **Only show models on this PC** to see
just the ones you can switch to instantly. Pick one and press **Use this
model**: if it isn't downloaded yet, it's installed first, with progress shown
in the window. Your choice is saved for next time.

| Model | Accuracy | Speed | VRAM | Languages | AMD, Intel, Apple GPUs |
| --- | --- | --- | --- | --- | --- |
| IBM Granite Speech 4.1 2B | ★★★★★ | ★★★ | 6 GB | en, fr, de, es, pt, ja | ✓ 2.6 GB |
| Cohere Transcribe | ★★★★½ | ★★★ | 5 GB | 14 | ✓ 2.4 GB, no login |
| Qwen3-ASR 1.7B | ★★★★½ | ★★★½ | 5 GB | 52 | ✓ 2.2 GB |
| NVIDIA Parakeet TDT 0.6B v3 | ★★★★ | ★★★★★ | 2 GB | 25 European | ✓ 0.7 GB (default) |
| NVIDIA Parakeet TDT 0.6B v2 | ★★★★ | ★★★★★ | 2 GB | English | ✓ 0.7 GB |
| NVIDIA Canary 1B v2 | ★★★★ | ★★★★ | 3 GB | 25 European | ✓ 1.1 GB |
| Qwen3-ASR 0.6B | ★★★★ | ★★★★ | 2.5 GB | 52 | ✓ 0.9 GB |
| Mistral Voxtral Mini 3B | ★★★★ | ★★½ | 9 GB | 8 | ✓ 5 GB |
| Kyutai STT 2.6B | ★★★★ | ★★★ | 6 GB | English | CPU only |
| Kyutai STT 1B | ★★★½ | ★★★★ | 3 GB | English, French | CPU only |
| **Whisper large-v3-turbo** (default with NVIDIA) | ★★★½ | ★★★★ | 2 GB | 99 | ✓ 0.9 GB |
| Whisper large-v3 | ★★★½ | ★★½ | 4 GB | 99 | ✓ 1.7 GB |
| Distil-Whisper large-v3 | ★★★ | ★★★★½ | 2 GB | English | CPU only |
| Moonshine base | ★★ | ★★★★★ | 1 GB | English | ✓ 0.08 GB |

The last column is the transcribe.cpp build used on GPUs other than NVIDIA's
(see [GPUs](#gpus)), with its download size. VRAM is for the NVIDIA builds; the
GGUF builds need about their download size in GPU memory, which on a built-in
GPU is shared with the system's RAM.

How it stays fast:

- **It uses the GPU you have.** NVIDIA GPUs run each model's CUDA build
  (CTranslate2, ONNX Runtime or PyTorch). Any other GPU runs the same models
  through transcribe.cpp: Vulkan on AMD and Intel GPUs, built in or separate,
  and Metal on Apple Silicon. On a Ryzen 5 3500U laptop on mains power,
  Parakeet v3 on its built-in Radeon Vega 8 transcribes 5 s of speech in
  0.55 s and 32 s in 2.2 s, against 1.1 s and 7.7 s on the same laptop's CPU,
  and loads in 1.3 s instead of 4.9 s, with the same text. (On battery that
  laptop holds its GPU at its lowest clock, and the two come out about even.)
- **A GPU problem never costs a dictation.** If Vulkan or Metal won't start,
  or a transcription on the GPU fails, Speechify switches that model to the
  CPU and says so in the console; if the GPU driver takes the model's process
  down, it's restarted on the CPU and the dictation is transcribed again. The
  model window and `speechify selftest --engine-only` show the device actually
  in use.
- **Only the selected model is loaded.** It runs in a separate worker process
  and stays loaded between dictations. Switching stops the old worker before
  starting the new one, so two models never compete for GPU memory. A
  dictation made while a model is loading waits for it instead of failing.
- **Each model family gets its own environment** under
  `engines` in the data folder, created the first time you use one of its
  models. They need conflicting libraries (Qwen3-ASR pins transformers 4.57,
  Cohere needs 5.4+), so this is the only way all of them can be installed
  side by side. The app itself needs just four small packages, which is why it
  starts instantly.
- **No network on startup.** Once a model is downloaded it's loaded in offline
  mode, which skips Hugging Face's version checks.
- Packages for these environments are only installed if they were released at
  least 14 days ago, the same rule as the repo's Dependabot setup.
  transcribe.cpp is pinned to an exact version with its file hashes, and each
  GGUF model to a Hugging Face revision whose SHA-256 is checked after the
  download.

All models benefit from the auto-fix rules. Whisper models also use your
learned vocabulary as hints and can be fine-tuned on your recordings. Cohere
Transcribe is gated: accept its terms on Hugging Face and run
`%APPDATA%\speechify\engines\transformers\Scripts\hf.exe auth login` first
(on Linux or macOS, `engines/transformers/bin/hf auth login` in the data folder).
Canary-Qwen 2.5B and Voxtral Mini 4B Realtime aren't included because they need
NVIDIA NeMo or vLLM, which don't run on Windows or macOS.

The ratings assume an NVIDIA GPU. On a CPU, everything is slower, Parakeet and
Moonshine the least; the LLM-based models can take several seconds.

### GPUs

`device = "auto"` in `config.toml` (the default) picks one of these:

| Machine | `device` | Models with a GGUF build | Others |
| --- | --- | --- | --- |
| NVIDIA GPU (Windows, or Linux with the NVIDIA driver) | `cuda` | their CUDA build | their CUDA build |
| AMD or Intel GPU, or another Vulkan GPU | `vulkan` | transcribe.cpp on Vulkan | CPU |
| Apple Silicon Mac | `metal` | transcribe.cpp on Metal | CPU |
| No GPU, or an Intel Mac | `cpu` | their own CPU build (int8 where there is one) | CPU |

Set `device` to one of those names to choose yourself. On Linux, Vulkan needs
your GPU's Mesa driver (`mesa-vulkan-drivers` on Ubuntu, usually installed
already). Whether transcribe.cpp's CUDA build would beat the current one on
NVIDIA GPUs is still open: `speechify bench` times both on your machine.

## Your recordings

The audio and text of every dictation are saved in the data folder: WAV
files in `audio`, with their text in `learning.db`. The data folder is
`%APPDATA%\speechify` on Windows, `~/.local/share/speechify` on Linux and
`~/Library/Application Support/speechify` on macOS; `speechify where` prints it. Open them with
**Ctrl+Alt+H** or `speechify history` to:

- **play** any recording back
- **edit its text.** Saving teaches the rules and vocabulary, and the
  edited text becomes the training target
- **revert** to what was originally pasted
- **exclude** a recording from training (a cough, background TV) without
  deleting it, or **delete** it for good

Every recording that isn't excluded can be used for fine-tuning. Edited ones
train on your text. The rest train on what a bigger "teacher" model heard
(see [Getting better on its own](#getting-better-on-its-own)), never on what
was pasted, so the model can't learn its own mistakes. `speechify export <folder>`
copies them out as a standard Hugging Face audiofolder dataset (WAVs plus
`metadata.csv`), so you can train other models on the same data.

## How it learns

Every fix you save does three things:

1. **Vocabulary hint, which helps right away.** The words you corrected to
   (`kubectl`, `GitHub`, names) are passed to Whisper as a glossary on every
   later dictation, so it's much more likely to hear them correctly. In
   English, a short punctuated sentence goes in front of it, glossary or not:
   without one, Whisper sometimes picks an all-lowercase, unpunctuated style
   for a whole dictation.
2. **Auto-replace rules.** Once you've made the same fix twice
   (`cube control` → `kubectl`), it gets applied automatically. You can change
   the threshold with `autocorrect_after`.
3. **Training data.** The audio and your corrected text are saved for fine-tuning.

Fine-tuning on your voice:

```
uv run scripts/finetune.py --apply
```

uv reads the training dependencies (PyTorch with CUDA, transformers, PEFT) from
the top of the script and builds a separate environment for them.

This trains a LoRA adapter on your labelled recordings and only saves the new
model, and with `--apply` switches to it, if it passes both gates described
below. Fine-tuned models also show up in the model picker. You'll want at
least 50 labelled recordings; a few hundred is better. Each run starts again
from the base model and trains on all of them. Speechify also does this by
itself.

### Getting better on its own

While you're away (no dictation for `idle_minutes`, 5 by default) and the
computer is on mains power, Speechify:

1. **Labels your recordings with a teacher model.** Granite Speech 4.1 2B, or
   Qwen3-ASR 1.7B if your GPU is too small for Granite, re-transcribes every
   recording that doesn't have a teacher transcript yet, newest first, with
   your learned vocabulary as a hint. Output that looks like a hallucination
   (empty, a repeated phrase, or too many or too few words for the audio) is
   rejected. The rest gets a confidence score, and only transcripts scoring at
   least `teacher_min_confidence` are trained on.
2. **Retrains the dictation model** after `retrain_after_labels` (300) new
   labelled recordings, or `retrain_after_days` (7) with at least one. The new
   model is switched in only if it passes two gates:
   - on a held-out slice of your recordings it has a lower word error rate
     than the model you use now. Once you've corrected 20 recordings, the slice
     is made of those; until then, of confident teacher labels;
   - on 50 LibriSpeech test-clean clips (downloaded once, about 10 MB) it's no
     more than `max_regression_wer` (0.5) points worse than the base model.

Each recording's training text is your correction if you made one, otherwise
the teacher's transcript if it's confident, otherwise nothing.

Dictation never waits for any of this. If your GPU has room for both models,
the teacher runs next to the dictation model and pauses while you talk.
Otherwise the dictation model is unloaded while the teacher or training runs,
and pressing the dictation key kills them instantly and reloads it while you
speak. Without an NVIDIA GPU the teacher doesn't run unless you set
`teacher_on_cpu = true`, and training needs one.

```
speechify learn status       labelled / pending / skipped, last runs, gate results
speechify learn run          label and retrain now, without waiting (quit the app first)
speechify models --rollback  go back to the previous version
```

The model window (Ctrl+Alt+M) has a **Roll back retrained model** button
that does the same as `--rollback` and switches straight away. The last
`keep_versions` (3) retrained versions are kept; older ones are deleted.
Set `learn_auto = false` to turn all of this off, or `teacher = "off"` to
stop just the teacher.

### Why not reinforcement learning after every fix?

Updating the model's weights after a single correction would make it
overfit to that one clip and get worse at everything else. What actually works
in practice: instant fixes through the vocabulary and rules, and a periodic
fine-tune on the corrections you've collected, kept only if it scores better.

## Commands

```
speechify            start dictation
speechify models     list models; `models use <id>` installs and selects one
speechify bench      time each engine for a model on this machine (`--cpu`, `--model`)
speechify selftest --engine-only   install, load and transcribe a test clip
speechify --version  print the installed version
speechify history    browse, replay and edit every recording
speechify export D   copy recordings + text to folder D
speechify stats      counts of samples, rules, vocabulary
speechify learn      teacher labels and retraining: `learn status`, `learn run`
speechify models --rollback   go back to the previous retrained model
speechify rules      list learned fixes (* = applied automatically)
speechify forget X   delete a bad rule or vocabulary term
speechify devices    list microphones (set input_device in config)
speechify where      print the config and data folder
```

## Why it should feel faster than Handy

- The mic opens on a background thread the moment the key goes down, so the
  keyboard never waits for it, and the pill's dot turns red as soon as audio
  is arriving. If you start talking before the dot is red and lose the first
  syllable, set `keep_mic_open = true`: the stream then stays open with 300 ms
  of pre-roll, at the cost of the mic always showing as in use.
- The model is loaded and warmed up once, then stays loaded. With Whisper
  large-v3-turbo each sentence is a single GPU pass of roughly 0.2–0.5 s on an
  RTX card. Parakeet is faster still.
- On AMD, Intel and Apple GPUs it runs the same engine and model files as
  Handy (transcribe.cpp, Q8_0 GGUF), so it's at least as fast there.
- Text is pasted through the clipboard in one go instead of typed one character at a time.
  The clipboard is restored afterwards.

The log (**Open log** in the menu) shows the latency of every dictation (total, and model time). If
it's slower than you'd like, switch to Parakeet v3, or set `beam_size = 1` for
roughly double Whisper's speed.

## Optional: LLM cleanup

Set `llm_url` and `llm_model` to any OpenAI-compatible endpoint (for example
Ollama at `http://localhost:11434/v1/chat/completions`) and each dictation
goes through a small model that fixes punctuation and misheard words, using
your glossary. This adds 100–500 ms, so it's off by default.

## Releases

Nightly builds come from `main` and stable releases from `release/X.Y`
branches. See [docs/releases.md](docs/releases.md) for how to cut a release
and ship a fix.

## Troubleshooting

- **"CUDA unavailable … falling back to CPU"**: update the NVIDIA driver.
- **"transcribe.cpp: vulkan didn't start … using the CPU instead"**: update the
  GPU driver (on Linux, install `mesa-vulkan-drivers`). `vulkaninfo --summary`
  should list your GPU. Set `device = "cpu"` to stop trying the GPU.
- **Nothing pastes into an admin app**: Windows blocks input from normal apps
  into elevated windows. Run speechify as admin as well.
- **Wrong mic**: pick it under Settings > Microphone.
- **A model fails to load**: the log shows the error from its worker.
  Delete its environment folder under `engines` in the data folder to
  reinstall it from scratch.
- **Where's the log?** **Open log** in the menu. Started from the Start menu,
  app menu or Dock, Speechify logs to `%LOCALAPPDATA%\Speechify\speechify.log`
  (Windows), `~/.local/state/speechify/speechify.log` (Linux) or
  `~/Library/Logs/Speechify.log` (macOS). Quit it from the menu and run
  `speechify` in a terminal to watch it live.
- **No tray icon on Linux**: GNOME shows tray icons only with the AppIndicator
  extension (Ubuntu has it; on Fedora install `gnome-shell-extension-appindicator`).
  Without one, start Speechify again to open its settings.
- **Mac: the talk key does nothing**: open System Settings > Privacy &
  Security, and allow Speechify under Accessibility. If
  you start it from a terminal, allow the terminal app instead.
- **Linux: "can't read the keyboard"**: on Wayland, start Speechify again and
  accept the setup it offers, or run `sudo usermod -aG input $USER` and log
  out and back in.
- **Linux: Ctrl+V doesn't paste in a terminal**: terminals paste with
  Ctrl+Shift+V, so dictate into other apps, or paste the text yourself.
- **Linux on Wayland with a non-QWERTY layout**: pasting presses Ctrl and the
  key in the QWERTY V position, which a layout like Dvorak reads as a
  different letter. Use an X11 session or a QWERTY layout for now.
