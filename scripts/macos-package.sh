#!/usr/bin/env bash
# Signs the macOS app and writes the release files:
#   Speechify-<version>-mac-arm64.dmg              the installer
#   Speechify-<version>-mac-arm64.app.tar.gz(.sig) the in-app update and its signature
#
#   scripts/macos-package.sh <Speechify.app> <version> <output dir>
#
# macOS keeps Accessibility access only while an app's signature matches the
# one it was granted to. An ad-hoc signature pins one exact build, so every
# update loses it. Signing every release with the same certificate pins that
# certificate instead, and the permission survives updates. See docs/releases.md.
#
# Environment:
#   MACOS_CERTIFICATE_P12         base64 .p12 holding the signing certificate and key
#   MACOS_CERTIFICATE_PASSWORD    its password
#   TAURI_SIGNING_PRIVATE_KEY     the update key (contents, not a path); without it no update is written
#   TAURI_SIGNING_PRIVATE_KEY_PASSWORD
#   REQUIRE_SIGNING=1             fail instead of falling back to ad hoc (set for published releases)
set -euo pipefail

app="$1"
version="$2"
out="$3"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
entitlements="$root/src-tauri/Entitlements.plist"
name="Speechify-$version-mac-arm64"
mkdir -p "$out"

if [ -n "${MACOS_CERTIFICATE_P12:-}" ]; then
  work="$(mktemp -d)"
  keychain="$work/signing.keychain-db"
  keychain_password="$(openssl rand -hex 16)"
  trap 'security delete-keychain "$keychain" 2>/dev/null || true; rm -rf "$work"' EXIT
  printf '%s' "$MACOS_CERTIFICATE_P12" | base64 --decode > "$work/signing.p12"
  security create-keychain -p "$keychain_password" "$keychain"
  security set-keychain-settings "$keychain"
  security unlock-keychain -p "$keychain_password" "$keychain"
  security import "$work/signing.p12" -k "$keychain" -P "${MACOS_CERTIFICATE_PASSWORD:-}" -T /usr/bin/codesign >/dev/null
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" >/dev/null
  # Self-signed, so not "valid" to the system: list all identities, not just -v.
  identity="$(security find-identity -p codesigning "$keychain" | awk '/^ *1\)/ { print $2; exit }')"
  if [ -z "$identity" ]; then
    echo "MACOS_CERTIFICATE_P12 holds no code signing identity." >&2
    exit 1
  fi
  codesign --force --deep --sign "$identity" --keychain "$keychain" --entitlements "$entitlements" "$app"
  # The designated requirement must name the certificate, not this build's hash.
  requirement="$(codesign -d -r- "$app" 2>&1)"
  case "$requirement" in
    *"certificate root = H"*) ;;
    *)
      echo "Unexpected designated requirement: $requirement" >&2
      exit 1
      ;;
  esac
elif [ "${REQUIRE_SIGNING:-}" = 1 ]; then
  echo "MACOS_CERTIFICATE_P12 is not set. A release signed ad hoc would make every user allow Accessibility again." >&2
  exit 1
else
  echo "::warning::No signing certificate: signing ad hoc. Accessibility access will not survive updating to this build."
  codesign --force --deep --sign - --entitlements "$entitlements" "$app"
fi
codesign --verify --deep --strict --verbose=2 "$app"
codesign -d -r- "$app" 2>&1 | tail -n 1

stage="$(mktemp -d)"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
hdiutil create -volname Speechify -srcfolder "$stage" -ov -format UDZO "$out/$name.dmg"
rm -rf "$stage"

if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  # The updater unpacks this archive in place of the installed app.
  COPYFILE_DISABLE=1 tar -czf "$out/$name.app.tar.gz" -C "$(dirname "$app")" "$(basename "$app")"
  (cd "$root" && npx tauri signer sign "$out/$name.app.tar.gz" >/dev/null)
  test -s "$out/$name.app.tar.gz.sig"
elif [ "${REQUIRE_SIGNING:-}" = 1 ]; then
  echo "TAURI_SIGNING_PRIVATE_KEY is not set, so this release could not be installed as an update." >&2
  exit 1
fi
ls -l "$out"
