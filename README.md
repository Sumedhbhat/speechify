# speechify

Hold a key, talk, let go, and the text is pasted wherever your cursor is. It
runs locally on Windows, Linux and macOS, on your NVIDIA GPU if you have one,
lets you pick from 14 speech models, and gets better as you correct it.

## Install on Windows

In PowerShell:

```powershell
irm https://github.com/Sumedhbhat/speechify/releases/latest/download/install.ps1 | iex
```

This installs the latest stable release and adds Speechify to the Start menu.
Run the same command again to update. For nightly builds, run
`$env:SPEECHIFY_CHANNEL = "nightly"` first. To start Speechify with Windows,
download `install.ps1` and run it with `-Autostart`. Channels and versions are
explained in [docs/releases.md](docs/releases.md).

The first start downloads the selected model and writes a config file to
`%APPDATA%\speechify\config.toml`. You don't need to install CUDA yourself;
each model's CUDA libraries come from pip. A recent NVIDIA driver is all you need.

From a source checkout instead: `uv sync`, then `start.bat`.

## Install on Linux or macOS

In a terminal:

```bash
curl -fsSL https://github.com/Sumedhbhat/speechify/releases/latest/download/install.sh | bash
```

Run the same command again to update. For nightly builds, use
`| SPEECHIFY_CHANNEL=nightly bash`. To start Speechify when you log in, use
`| bash -s -- --autostart`.

It installs Speechify to `~/.local/share/speechify-app`, adds a `speechify`
command to `~/.local/bin`, and adds Speechify to your app menu (Linux) or
`~/Applications` (macOS). On Linux it also offers to install what's missing:
PortAudio for the microphone, and `wl-clipboard` or `xclip` for pasting. It
asks before using `sudo`, and you don't need a C compiler.

**On a Mac**, push to talk is **Right Option**, since Mac keyboards have no Right
Ctrl. The first start asks for three permissions: Microphone, Input Monitoring
(to see the talk key) and Accessibility (to paste). Allow all three, then start
Speechify again.

**On Linux with Wayland** (the default on Ubuntu and Fedora), apps can't see
each other's keys, so Speechify reads the keyboard from `/dev/input` and pastes
through a virtual keyboard. That needs your user in the `input` group; the
installer offers to set it up, and you log out and back in once afterwards.
On X11 nothing extra is needed.

**Without an NVIDIA GPU** (every Mac, and many Linux PCs), the first start
picks NVIDIA Parakeet TDT 0.6B v3 instead of Whisper, because it's the fastest
accurate model on a CPU. It runs with int8 weights there: a 0.7 GB download
instead of 2.5 GB, and nearly twice as fast at the same accuracy. Models are
installed with their CPU-only packages, which are a fraction of the size.
Linux PCs with the NVIDIA driver get the same GPU setup as Windows.

From a source checkout: `uv sync --no-install-package evdev`, then `./start.sh`.

## Using it

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

All of these keys can be changed in `config.toml`.

## Models

Open the model picker with **Ctrl+Alt+M**, or run `speechify models`. It lists
every model with its accuracy and speed rating, the GPU memory it needs, and
whether it's already on this PC. Tick **Only show models on this PC** to see
just the ones you can switch to instantly. Pick one and press **Use this
model**: if it isn't downloaded yet, it's installed first, with progress shown
in the window. Your choice is saved for next time.

| Model | Accuracy | Speed | VRAM | Languages |
| --- | --- | --- | --- | --- |
| IBM Granite Speech 4.1 2B | ★★★★★ | ★★★ | 6 GB | en, fr, de, es, pt, ja |
| Cohere Transcribe | ★★★★½ | ★★★ | 5 GB | 14 |
| Qwen3-ASR 1.7B | ★★★★½ | ★★★½ | 5 GB | 52 |
| NVIDIA Parakeet TDT 0.6B v3 | ★★★★ | ★★★★★ | 2 GB | 25 European |
| NVIDIA Parakeet TDT 0.6B v2 | ★★★★ | ★★★★★ | 2 GB | English |
| NVIDIA Canary 1B v2 | ★★★★ | ★★★★ | 3 GB | 25 European |
| Qwen3-ASR 0.6B | ★★★★ | ★★★★ | 2.5 GB | 52 |
| Mistral Voxtral Mini 3B | ★★★★ | ★★½ | 9 GB | 8 |
| Kyutai STT 2.6B | ★★★★ | ★★★ | 6 GB | English |
| Kyutai STT 1B | ★★★½ | ★★★★ | 3 GB | English, French |
| **Whisper large-v3-turbo** (default) | ★★★½ | ★★★★ | 2 GB | 99 |
| Whisper large-v3 | ★★★½ | ★★½ | 4 GB | 99 |
| Distil-Whisper large-v3 | ★★★ | ★★★★½ | 2 GB | English |
| Moonshine base | ★★ | ★★★★★ | 1 GB | English |

How it stays fast:

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

All models benefit from the auto-fix rules. Whisper models also use your
learned vocabulary as hints and can be fine-tuned on your recordings. Cohere
Transcribe is gated: accept its terms on Hugging Face and run
`%APPDATA%\speechify\engines\transformers\Scripts\hf.exe auth login` first
(on Linux or macOS, `engines/transformers/bin/hf auth login` in the data folder).
Canary-Qwen 2.5B and Voxtral Mini 4B Realtime aren't included because they need
NVIDIA NeMo or vLLM, which don't run on Windows or macOS.

The ratings assume an NVIDIA GPU. On a CPU, everything is slower, Parakeet and
Moonshine the least; the LLM-based models can take several seconds.

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

Every recording that isn't excluded is used for fine-tuning. Edited ones train
on your text and the rest train on what was pasted. `speechify export <folder>`
copies them out as a standard Hugging Face audiofolder dataset (WAVs plus
`metadata.csv`), so you can train other models on the same data.

## How it learns

Every fix you save does three things:

1. **Vocabulary hint, which helps right away.** The words you corrected to
   (`kubectl`, `GitHub`, names) are passed to Whisper as a glossary on every
   later dictation, so it's much more likely to hear them correctly.
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

This trains a LoRA adapter on your saved samples and checks it against a
held-out slice of your audio. It only saves the new model, and with `--apply`
switches to it, if your word error rate went down. Fine-tuned models also show
up in the model picker. You'll want at
least 50 samples; a few hundred is better. Run it every few weeks; each run
starts again from the base model and trains on all of your recordings.

### Why not reinforcement learning after every fix?

Updating the model's weights after a single correction would make it
overfit to that one clip and get worse at everything else. What actually works
in practice: instant fixes through the vocabulary and rules, and a periodic
fine-tune on the corrections you've collected, kept only if it scores better.

## Commands

```
speechify            start dictation
speechify models     list models; `models use <id>` installs and selects one
speechify --version  print the installed version
speechify history    browse, replay and edit every recording
speechify export D   copy recordings + text to folder D
speechify stats      counts of samples, rules, vocabulary
speechify rules      list learned fixes (* = applied automatically)
speechify forget X   delete a bad rule or vocabulary term
speechify devices    list microphones (set input_device in config)
speechify where      print the config and data folder
```

## Why it should feel faster than Handy

- The mic stream stays open all the time and keeps 300 ms of pre-roll, so
  there's no startup delay and the first word isn't cut off.
- The model is loaded and warmed up once, then stays loaded. With Whisper
  large-v3-turbo each sentence is a single GPU pass of roughly 0.2–0.5 s on an
  RTX card. Parakeet is faster still.
- Text is pasted through the clipboard in one go instead of typed one character at a time.
  The clipboard is restored afterwards.

The console prints the latency of every dictation (total, and model time). If
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
- **Nothing pastes into an admin app**: Windows blocks input from normal apps
  into elevated windows. Run speechify as admin as well.
- **Wrong mic**: run `speechify devices` and set `input_device`.
- **A model fails to load**: the console shows the error from its worker.
  Delete its environment folder under `engines` in the data folder to
  reinstall it from scratch.
- **Where's the console on Linux or macOS?** Started from the app menu,
  Speechify logs to `~/.local/state/speechify/speechify.log` (Linux) or
  `~/Library/Logs/Speechify.log` (macOS). Run `speechify` in a terminal to
  watch it live.
- **Mac: the talk key does nothing**: open System Settings > Privacy &
  Security, and allow Speechify under Input Monitoring and Accessibility. If
  you start it from a terminal, allow the terminal app instead.
- **Linux: "can't read the keyboard"**: on Wayland, run
  `sudo usermod -aG input $USER` and log out and back in, or run the installer
  again.
- **Linux: Ctrl+V doesn't paste in a terminal**: terminals paste with
  Ctrl+Shift+V, so dictate into other apps, or paste the text yourself.
- **Linux on Wayland with a non-QWERTY layout**: pasting presses Ctrl and the
  key in the QWERTY V position, which a layout like Dvorak reads as a
  different letter. Use an X11 session or a QWERTY layout for now.
