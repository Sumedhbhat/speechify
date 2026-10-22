#!/usr/bin/env bash
# Installs or updates Speechify on Linux or macOS from GitHub releases.
#
#   curl -fsSL https://github.com/Sumedhbhat/speechify/releases/latest/download/install.sh | bash
#
# Options (when piping, pass them after `bash -s --`):
#   --channel C    stable (default), nightly, or a tag like v0.1.2; or set SPEECHIFY_CHANNEL
#   --autostart    also start Speechify when you log in
#   --yes          install system packages without asking
#   --from ZIP     install a local speechify-<version>.zip instead of downloading (used by CI)
#
# Your recordings, corrections and settings live in ~/.local/share/speechify
# (Linux) or ~/Library/Application Support/speechify (macOS) and are never
# touched by installs or updates.
#
# Everything is inside main(), so a `curl | bash` that gets cut off midway
# runs nothing, and commands that read stdin can't eat the rest of the script.

set -euo pipefail

main() {
    local channel=${SPEECHIFY_CHANNEL:-stable} autostart=0 from=""
    yes=0
    while [ $# -gt 0 ]; do
        case $1 in
            --channel) channel=${2:?--channel needs a value}; shift 2 ;;
            --autostart) autostart=1; shift ;;
            --yes | -y) yes=1; shift ;;
            --from) from=${2:?--from needs a file}; shift 2 ;;
            *) die "Unknown option '$1'. See the top of install.sh for the options." ;;
        esac
    done

    local os
    os=$(uname -s)
    case $os in
        Linux | Darwin) ;;
        *) die "This installer is for Linux and macOS. On Windows, use install.ps1." ;;
    esac
    local repo=Sumedhbhat/speechify
    local install_dir=${SPEECHIFY_INSTALL_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/speechify-app}
    local bin_dir=$HOME/.local/bin
    case $install_dir in
        "" | / | "$HOME" | "$HOME/") die "Refusing to install into '$install_dir'." ;;
    esac

    if [ "$os" = Linux ]; then
        system_packages
        wayland_access
    fi
    ensure_uv

    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    local zip version
    if [ -n "$from" ]; then
        zip=$tmp/$(basename "$from")
        cp "$from" "$zip"
    else
        local info zip_url sums_url
        info=$(release_info "$repo" "$channel")
        read -r version zip_url sums_url <<<"$info"
        say "Installing Speechify $version ($channel)"
        zip=$tmp/speechify-$version.zip
        curl -fsSL "$zip_url" -o "$zip"
        curl -fsSL "$sums_url" -o "$tmp/SHA256SUMS"
        verify "$zip" "$tmp/SHA256SUMS"
    fi

    mkdir -p "$tmp/unpacked"
    if command -v unzip >/dev/null; then
        unzip -q "$zip" -d "$tmp/unpacked"
    else
        uv run --no-project --python 3.12 python -m zipfile -e "$zip" "$tmp/unpacked"
    fi
    local unpacked
    unpacked=$(find "$tmp/unpacked" -mindepth 1 -maxdepth 1 -type d -name 'speechify-*' | head -n 1)
    [ -n "$unpacked" ] || die "$(basename "$zip") doesn't contain a speechify-<version> folder."
    rm -rf "$install_dir"
    mkdir -p "$(dirname "$install_dir")"
    mv "$unpacked" "$install_dir"
    chmod +x "$install_dir/start.sh"

    # pynput lists evdev for Linux, but only needs it for a backend Speechify
    # doesn't use (linuxkeys.py reads /dev/input itself), and evdev has to be
    # compiled. Skipping it means no C compiler is needed.
    local skip=()
    [ "$os" = Linux ] && skip=(--no-install-package evdev)
    # ${a[@]+...}: macOS's bash 3.2 treats an empty array as unset under set -u.
    (cd "$install_dir" && uv sync --frozen --no-dev ${skip[@]+"${skip[@]}"} </dev/null)
    local installed
    installed=$("$install_dir/start.sh" --version)

    write_launchers "$os" "$install_dir" "$bin_dir" "$autostart"

    say ""
    say "$installed installed to $install_dir"
    if [ "$os" = Darwin ]; then
        say "Start it from ~/Applications/Speechify (Spotlight finds it), then hold Right Option to talk."
        say "The first start asks for Microphone, Input Monitoring and Accessibility access: allow all"
        say "three, then start Speechify again."
    else
        say "Start it from your app menu (Speechify), then hold Right Ctrl to talk."
    fi
    say "The first start downloads the speech model. Logs: $(log_file "$os")"
    case ":$PATH:" in
        *":$bin_dir:"*) ;;
        *) say "Add $bin_dir to your PATH to use the speechify command." ;;
    esac
    if [ -n "${relogin:-}" ]; then
        say ""
        say "Log out and back in once first, so the new 'input' group membership applies."
    fi
}

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

# ask QUESTION: yes unless the user says no. Without a terminal to ask on,
# only --yes counts as a yes.
ask() {
    [ "$yes" = 1 ] && return 0
    if (: </dev/tty) 2>/dev/null; then
        local reply
        printf '%s [Y/n] ' "$1" >/dev/tty
        read -r reply </dev/tty || return 1
        case $reply in [nN]*) return 1 ;; esac
        return 0
    fi
    return 1
}

is_wayland() { [ -n "${WAYLAND_DISPLAY:-}" ] || [ "${XDG_SESSION_TYPE:-}" = wayland ]; }

# PortAudio for the microphone, and the clipboard tool pyperclip uses.
system_packages() {
    local missing=()
    if ! { ldconfig -p 2>/dev/null || /sbin/ldconfig -p 2>/dev/null || true; } | grep -q 'libportaudio\.so\.2'; then
        missing+=(portaudio)
    fi
    if is_wayland; then
        command -v wl-copy >/dev/null || missing+=(wl-clipboard)
    else
        command -v xclip >/dev/null || command -v xsel >/dev/null || missing+=(xclip)
    fi
    [ ${#missing[@]} -eq 0 ] && return

    local pm install names=()
    for pm in apt-get dnf pacman zypper; do command -v $pm >/dev/null && break; pm=""; done
    local p
    for p in "${missing[@]}"; do
        case $pm:$p in
            apt-get:portaudio | zypper:portaudio) names+=(libportaudio2) ;;
            *) names+=("$p") ;;
        esac
    done
    case $pm in
        apt-get) install="sudo apt-get install -y ${names[*]}" ;;
        dnf) install="sudo dnf install -y ${names[*]}" ;;
        pacman) install="sudo pacman -S --needed --noconfirm ${names[*]}" ;;
        zypper) install="sudo zypper install -y ${names[*]}" ;;
        *) say "Speechify needs these system packages; install them with your package manager: ${missing[*]}"; return ;;
    esac
    if ask "Speechify needs ${names[*]}. Install with: $install ?"; then
        [ "$pm" = apt-get ] && sudo apt-get update -qq
        $install
    else
        say "Skipped. Install them yourself before starting Speechify: $install"
    fi
}

# Wayland only lets Speechify see and type keys through /dev/input and
# /dev/uinput (see src/speechify/linuxkeys.py).
wayland_access() {
    is_wayland || return 0
    local rule=/etc/udev/rules.d/60-speechify-uinput.rules
    local user=${USER:-$(id -un)} in_group=0
    id -nG "$user" | tr ' ' '\n' | grep -qx input && in_group=1
    [ "$in_group" = 1 ] && [ -f "$rule" ] && return
    say ""
    say "On Wayland, Speechify reads the talk key from /dev/input and pastes through a virtual"
    say "keyboard (/dev/uinput). That needs your user in the 'input' group, which lets programs"
    say "you run read keystrokes, the same access every app has on X11."
    if ! ask "Add $user to the input group and allow it to use /dev/uinput (uses sudo)?"; then
        say "Skipped. Speechify will explain how to do this when it starts."
        return
    fi
    if [ "$in_group" = 0 ]; then
        sudo usermod -aG input "$user"
        relogin=1
    fi
    echo 'KERNEL=="uinput", SUBSYSTEM=="misc", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"' |
        sudo tee "$rule" >/dev/null
    echo uinput | sudo tee /etc/modules-load.d/speechify-uinput.conf >/dev/null
    sudo modprobe uinput || true
    sudo udevadm control --reload-rules
    sudo udevadm trigger --subsystem-match=misc --sysname-match=uinput || true
}

ensure_uv() {
    export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"
    command -v uv >/dev/null && return
    say "Installing uv (the Python package manager Speechify runs on)"
    curl -LsSf https://astral.sh/uv/install.sh | sh
    command -v uv >/dev/null || die "uv didn't install; see https://docs.astral.sh/uv/"
}

# release_info REPO CHANNEL -> "version zip_url sums_url"
release_info() {
    local api=https://api.github.com/repos/$1/releases url
    case $2 in
        stable) url=$api/latest ;;
        nightly) url="$api?per_page=30" ;;
        v[0-9]*) url=$api/tags/$2 ;;
        *) die "Unknown channel '$2'. Use stable, nightly, or a tag like v0.1.2." ;;
    esac
    curl -fsSL -H "Accept: application/vnd.github+json" -H "User-Agent: speechify-installer" "$url" |
        uv run --no-project --python 3.12 python -c '
import json, sys
data = json.load(sys.stdin)
if isinstance(data, list):  # nightly: the newest prerelease
    data = next((r for r in data if r["prerelease"]), None)
    if data is None:
        sys.exit("No nightly release found.")
tag = data["tag_name"]
version = tag.lstrip("v")
assets = {a["name"]: a["browser_download_url"] for a in data["assets"]}
name = "speechify-" + version + ".zip"
if name not in assets or "SHA256SUMS" not in assets:
    sys.exit("Release " + tag + " is missing its assets.")
print(version, assets[name], assets["SHA256SUMS"])
'
}

verify() {
    local name expected actual
    name=$(basename "$1")
    expected=$(awk -v n="$name" '$2 == n || $2 == "*" n { print $1 }' "$2")
    if command -v sha256sum >/dev/null; then
        actual=$(sha256sum "$1" | cut -d' ' -f1)
    else
        actual=$(shasum -a 256 "$1" | cut -d' ' -f1)
    fi
    if [ -z "$expected" ] || [ "$actual" != "$expected" ]; then
        die "Checksum mismatch for $name."
    fi
}

log_file() {
    if [ "$1" = Darwin ]; then
        echo "$HOME/Library/Logs/Speechify.log"
    else
        echo "${XDG_STATE_HOME:-$HOME/.local/state}/speechify/speechify.log"
    fi
}

# write_if_changed FILE: replace FILE with stdin only if it differs. Keeps the
# macOS app bundle byte-identical across updates, so permissions you granted
# it aren't reset.
write_if_changed() {
    local new
    new=$(mktemp)
    cat >"$new"
    if [ -f "$1" ] && cmp -s "$new" "$1"; then
        rm -f "$new"
    else
        mkdir -p "$(dirname "$1")"
        mv "$new" "$1"
    fi
}

write_launchers() {
    local os=$1 dir=$2 bin_dir=$3 autostart=$4 log
    log=$(log_file "$os")

    # Started from a menu there's no terminal, so output goes to a log file.
    write_if_changed "$dir/launch.sh" <<EOF
#!/bin/sh
mkdir -p "$(dirname "$log")"
exec "$dir/start.sh" >>"$log" 2>&1
EOF
    chmod +x "$dir/launch.sh"

    write_if_changed "$bin_dir/speechify" <<EOF
#!/bin/sh
exec "$dir/start.sh" "\$@"
EOF
    chmod +x "$bin_dir/speechify"

    if [ "$os" = Darwin ]; then
        # A tiny app bundle, so macOS asks for Microphone, Input Monitoring
        # and Accessibility on behalf of "Speechify" (and needs the
        # microphone usage text below to allow recording at all).
        local app=$HOME/Applications/Speechify.app
        write_if_changed "$app/Contents/MacOS/Speechify" <<EOF
#!/bin/sh
exec "$dir/launch.sh"
EOF
        chmod +x "$app/Contents/MacOS/Speechify"
        write_if_changed "$app/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>Speechify</string>
    <key>CFBundleIdentifier</key><string>io.github.sumedhbhat.speechify</string>
    <key>CFBundleName</key><string>Speechify</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>LSUIElement</key><true/>
    <key>NSMicrophoneUsageDescription</key>
    <string>Speechify records while you hold the talk key and transcribes it on this Mac.</string>
</dict>
</plist>
EOF
        if [ "$autostart" = 1 ]; then
            write_if_changed "$HOME/Library/LaunchAgents/io.github.sumedhbhat.speechify.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>io.github.sumedhbhat.speechify</string>
    <key>ProgramArguments</key>
    <array><string>/usr/bin/open</string><string>-g</string><string>-a</string><string>$app</string></array>
    <key>RunAtLoad</key><true/>
</dict>
</plist>
EOF
        fi
        return
    fi

    local entry
    entry="[Desktop Entry]
Type=Application
Name=Speechify
Comment=Hold a key, talk, and the text is pasted
Exec=\"$dir/launch.sh\"
Icon=audio-input-microphone
Terminal=false
Categories=Utility;Accessibility;"
    local apps=${XDG_DATA_HOME:-$HOME/.local/share}/applications
    write_if_changed "$apps/speechify.desktop" <<<"$entry"
    local auto=${XDG_CONFIG_HOME:-$HOME/.config}/autostart/speechify.desktop
    if [ "$autostart" = 1 ] || [ -f "$auto" ]; then
        write_if_changed "$auto" <<<"$entry"
    fi
}

main "$@"
