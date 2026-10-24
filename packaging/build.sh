#!/usr/bin/env bash
# Builds the installer for the platform it runs on, into dist/:
#
#   Windows  Speechify-<version>-windows-x64-setup.exe   (Inno Setup)
#   macOS    Speechify-<version>-macos-<arch>.dmg
#   Linux    Speechify-<version>-x86_64.AppImage and speechify_<version>_amd64.deb
#
#   packaging/build.sh 0.2.0-nightly.57
#
# Needs uv. On Linux also a C compiler and ALSA headers (libasound2-dev) for
# PortAudio; on Windows, Inno Setup 6 (set ISCC if it isn't in the default
# place). Downloads are pinned by checksum.
set -euo pipefail

version=${1:?usage: packaging/build.sh <version>}
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
build=$root/packaging/build
mkdir -p "$build" dist

case $(uname -s) in
    Linux) os=linux ;;
    Darwin) os=macos ;;
    MINGW* | MSYS* | CYGWIN*) os=windows ;;
    *) echo "Unsupported platform: $(uname -s)" >&2; exit 1 ;;
esac

download() { # URL SHA256 FILE
    [ -f "$3" ] && sha256 "$3" | grep -qx "$2" && return
    curl -fsSL "$1" -o "$3"
    if ! sha256 "$3" | grep -qx "$2"; then
        echo "Checksum mismatch for $1" >&2
        rm -f "$3"
        exit 1
    fi
}

sha256() {
    if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# PortAudio without JACK, so the only thing it needs from the system is ALSA
# (which PulseAudio and PipeWire also speak). Distros' libportaudio2 pulls in
# JACK, which most desktops don't have.
build_portaudio() {
    [ -f "$build/libportaudio.so.2" ] && return
    local work=$build/portaudio
    rm -rf "$work"
    mkdir -p "$work"
    download https://files.portaudio.com/archives/pa_stable_v190700_20210406.tgz \
        47efbf42c77c19a05d22e627d42873e991ec0c1357219c0d74ce6a2948cb2def "$work/portaudio.tgz"
    tar xzf "$work/portaudio.tgz" -C "$work"
    (cd "$work/portaudio" && ./configure --quiet --without-jack --without-oss --disable-static && make -s -j"$(nproc)")
    cp -L "$work/portaudio/lib/.libs/libportaudio.so.2" "$build/libportaudio.so.2"
}

# Windows' file properties show this.
windows_version_info() {
    local core=${version%%-*} build_no=0
    case $version in *-nightly.*) build_no=${version##*.} ;; esac
    local nums="${core//./, }, $build_no"
    cat >"$build/version_info.txt" <<EOF
VSVersionInfo(
  ffi=FixedFileInfo(filevers=($nums), prodvers=($nums)),
  kids=[
    StringFileInfo([StringTable('040904B0', [
      StringStruct('CompanyName', 'Speechify'),
      StringStruct('FileDescription', 'Speechify'),
      StringStruct('FileVersion', '$version'),
      StringStruct('ProductName', 'Speechify'),
      StringStruct('ProductVersion', '$version'),
      StringStruct('OriginalFilename', 'Speechify.exe')])]),
    VarFileInfo([VarStruct('Translation', [1033, 1200])])
  ]
)
EOF
}

package_windows() {
    local iscc=${ISCC:-"C:/Program Files (x86)/Inno Setup 6/ISCC.exe"}
    # See speechify.spec: `speechify` in a terminal finds .com before .exe.
    mv -f "$build/dist/Speechify/speechify-console.exe" "$build/dist/Speechify/speechify.com"
    SPEECHIFY_VERSION=$version "$iscc" -Q packaging/windows/speechify.iss
}

package_macos() {
    local stage=$build/dmg arch
    arch=$(uname -m)
    rm -rf "$stage"
    mkdir -p "$stage"
    # Ad-hoc, so macOS runs it on Apple silicon; see docs/releases.md about a Developer ID.
    codesign --force --deep --sign - "$build/dist/Speechify.app"
    cp -R "$build/dist/Speechify.app" "$stage/"
    ln -s /Applications "$stage/Applications"
    hdiutil create -volname Speechify -srcfolder "$stage" -ov -format UDZO "$root/dist/Speechify-$version-macos-$arch.dmg"
}

desktop_entry() { # EXEC
    cat <<EOF
[Desktop Entry]
Type=Application
Name=Speechify
Comment=Hold a key, talk, and the text is pasted
Exec=$1
Icon=speechify
Terminal=false
Categories=Utility;Accessibility;
EOF
}

package_appimage() {
    local appdir=$build/AppDir tools=$build/tools
    rm -rf "$appdir"
    mkdir -p "$appdir/usr/lib" "$tools"
    cp -R "$build/dist/speechify" "$appdir/usr/lib/speechify"
    cat >"$appdir/AppRun" <<'EOF'
#!/bin/sh
here=$(dirname "$(readlink -f "$0")")
export APPDIR="${APPDIR:-$here}"
exec "$here/usr/lib/speechify/speechify" "$@"
EOF
    chmod +x "$appdir/AppRun"
    desktop_entry speechify >"$appdir/speechify.desktop"
    cp "$build/icon.png" "$appdir/speechify.png"
    ln -s speechify.png "$appdir/.DirIcon"

    download https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage \
        ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0 "$tools/appimagetool"
    # The runtime that starts the AppImage. This one needs no libfuse2.
    download https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64 \
        2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d "$tools/runtime-x86_64"
    chmod +x "$tools/appimagetool"
    APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$tools/appimagetool" --no-appstream \
        --runtime-file "$tools/runtime-x86_64" "$appdir" "$root/dist/Speechify-$version-x86_64.AppImage"
}

package_deb() {
    local deb=$build/deb
    # Debian sorts "~" before anything, so 0.2.0~nightly.57 < 0.2.0.
    local deb_version=${version/-/\~}
    rm -rf "$deb"
    mkdir -p "$deb/DEBIAN" "$deb/opt" "$deb/usr/bin" "$deb/usr/share/applications" \
        "$deb/usr/share/icons/hicolor/256x256/apps" "$deb/usr/lib/udev/rules.d" "$deb/usr/lib/modules-load.d"
    cp -R "$build/dist/speechify" "$deb/opt/speechify"
    ln -s /opt/speechify/speechify "$deb/usr/bin/speechify"
    desktop_entry speechify >"$deb/usr/share/applications/speechify.desktop"
    cp "$build/icon.png" "$deb/usr/share/icons/hicolor/256x256/apps/speechify.png"
    # Lets the input group type through /dev/uinput, for Wayland (see linuxsetup.py).
    echo 'KERNEL=="uinput", SUBSYSTEM=="misc", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"' \
        >"$deb/usr/lib/udev/rules.d/60-speechify-uinput.rules"
    echo uinput >"$deb/usr/lib/modules-load.d/speechify-uinput.conf"
    cat >"$deb/DEBIAN/control" <<EOF
Package: speechify
Version: $deb_version
Architecture: amd64
Maintainer: Sumedh Bhat <Sumedhbhat@users.noreply.github.com>
Installed-Size: $(du -sk "$deb" | cut -f1)
Depends: libc6 (>= 2.35), libasound2t64 | libasound2
Recommends: wl-clipboard, xclip
Section: utils
Priority: optional
Homepage: https://github.com/Sumedhbhat/speechify
Description: Hold a key, talk, and the text is pasted
 Local speech-to-text dictation that runs on your own machine, with a choice
 of speech models, and learns from your corrections.
EOF
    cat >"$deb/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = configure ]; then
    modprobe uinput 2>/dev/null || true
    udevadm control --reload-rules 2>/dev/null || true
    udevadm trigger --subsystem-match=misc --sysname-match=uinput 2>/dev/null || true
fi
EOF
    chmod -R u=rwX,go=rX "$deb"
    chmod 0755 "$deb/DEBIAN/postinst"
    dpkg-deb --root-owner-group -Zxz --build "$deb" "$root/dist/speechify_${deb_version}_amd64.deb"
}

# --- build ------------------------------------------------------------------

# The app reads its version from here. Removed afterwards, so a source
# checkout goes back to the pyproject version.
echo "$version" >src/speechify/VERSION
trap 'rm -f "$root/src/speechify/VERSION"' EXIT

skip=()
[ "$os" = linux ] && skip=(--no-install-package evdev)  # see linuxkeys.py
uv sync --frozen --group build ${skip[@]+"${skip[@]}"}
uv run --frozen --no-sync python packaging/make_icons.py "$build"
case $os in
    linux) build_portaudio ;;
    windows) windows_version_info ;;
esac
uv run --frozen --no-sync pyinstaller --noconfirm --clean \
    --distpath "$build/dist" --workpath "$build/work" packaging/speechify.spec

case $os in
    windows) package_windows ;;
    macos) package_macos ;;
    linux)
        package_appimage
        package_deb
        ;;
esac
ls -l dist
