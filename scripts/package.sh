#!/usr/bin/env bash
# Packages a `cargo build --profile dist` of the app into dist/, named after
# the version in Cargo.toml: what the release workflow publishes (see
# README, "Releases"). Runs on the platform it packages.
#
#   scripts/package.sh macos     dist/AudioBoard-vX.Y.Z-macos-<arch>.zip, holding AudioBoard.app
#   scripts/package.sh linux     dist/AudioBoard-vX.Y.Z-linux-<arch>.tar.gz
#   scripts/package.sh windows   dist/AudioBoard-vX.Y.Z-windows-x86_64.zip
#
# macOS: with both target/aarch64-apple-darwin/dist and
# target/x86_64-apple-darwin/dist built, the app is universal (lipo);
# otherwise it holds the one built, or target/dist (a plain build).
set -euo pipefail
cd "$(dirname "$0")/.."
platform=${1:?usage: scripts/package.sh macos|linux|windows}
version=$(scripts/version.sh)
out=dist
stage=target/package
rm -rf "$stage"
mkdir -p "$out" "$stage"

# The first of the given files that exists.
first() {
    for f in "$@"; do
        [ -f "$f" ] && { echo "$f"; return; }
    done
    echo "no build found: tried $*" >&2
    exit 1
}

case "$platform" in
    macos)
        arm=target/aarch64-apple-darwin/dist/audioboard
        x64=target/x86_64-apple-darwin/dist/audioboard
        app="$stage/AudioBoard.app"
        mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
        if [ -f "$arm" ] && [ -f "$x64" ]; then
            lipo -create -output "$app/Contents/MacOS/audioboard" "$arm" "$x64"
            arch=universal
        else
            bin=$(first "$arm" "$x64" target/dist/audioboard)
            cp "$bin" "$app/Contents/MacOS/audioboard"
            case "$(lipo -archs "$bin")" in
                arm64) arch=arm64 ;;
                x86_64) arch=x86_64 ;;
                *) arch=universal ;;
            esac
        fi
        sed "s/@VERSION@/$version/g" scripts/macos/Info.plist >"$app/Contents/Info.plist"
        cp scripts/macos/AudioBoard.icns "$app/Contents/Resources/"
        # Ad-hoc signature: no Developer ID, but a sealed bundle that Apple
        # Silicon runs (see README, "Releases", for the first launch).
        codesign --force --deep --sign - "$app"
        codesign --verify --strict "$app"
        file="$out/AudioBoard-v$version-macos-$arch.zip"
        rm -f "$file"
        # Without the build machine's extended attributes (no __MACOSX entries).
        ditto -c -k --norsrc --noextattr --noqtn --keepParent "$app" "$file"
        ;;
    linux)
        bin=$(first target/x86_64-unknown-linux-gnu/dist/audioboard target/dist/audioboard)
        name="AudioBoard-v$version-linux-$(uname -m)"
        mkdir -p "$stage/$name"
        cp "$bin" "$stage/$name/audioboard"
        cp README.md "$stage/$name/"
        file="$out/$name.tar.gz"
        tar -C "$stage" -czf "$file" "$name"
        ;;
    windows)
        bin=$(first target/x86_64-pc-windows-msvc/dist/audioboard.exe target/dist/audioboard.exe)
        name="AudioBoard-v$version-windows-x86_64"
        mkdir -p "$stage/$name"
        cp "$bin" "$stage/$name/audioboard.exe"
        cp README.md "$stage/$name/"
        file="$out/$name.zip"
        rm -f "$file"
        if command -v 7z >/dev/null; then
            (cd "$stage" && 7z a -tzip -bso0 "../../$file" "$name")
        else
            (cd "$stage" && zip -qr "../../$file" "$name")
        fi
        ;;
    *)
        echo "platform must be macos, linux or windows, not '$platform'" >&2
        exit 1
        ;;
esac

echo "$file"
if [ -n "${GITHUB_OUTPUT:-}" ]; then
    echo "file=$file" >>"$GITHUB_OUTPUT"
fi
