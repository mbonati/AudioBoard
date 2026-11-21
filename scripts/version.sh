#!/usr/bin/env bash
# The app's version: the `version` of `[package]` in Cargo.toml (and the
# `audioboard` entry of Cargo.lock, kept equal so `cargo build --locked`
# passes). Used by the release workflow (.github/workflows/release.yml);
# runs with bash and awk alone (macOS, Linux, Git Bash on Windows).
#
#   scripts/version.sh                       print the current version
#   scripts/version.sh next patch|minor|major print the version after a bump
#   scripts/version.sh set X.Y.Z             write X.Y.Z into Cargo.toml and Cargo.lock
set -euo pipefail
cd "$(dirname "$0")/.."

current() {
    awk -F'"' '/^\[/ { pkg = ($0 ~ /^\[package\][[:space:]]*$/) } pkg && /^version[[:space:]]*=/ { print $2; exit }' Cargo.toml
}

valid() {
    [[ "$1" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]
}

next() {
    local v major minor patch
    v=$(current)
    valid "$v" || { echo "Cargo.toml version '$v' is not X.Y.Z" >&2; exit 1; }
    IFS=. read -r major minor patch <<<"$v"
    case "$1" in
        major) echo "$((major + 1)).0.0" ;;
        minor) echo "$major.$((minor + 1)).0" ;;
        patch) echo "$major.$minor.$((patch + 1))" ;;
        *) echo "bump must be patch, minor or major, not '$1'" >&2; exit 1 ;;
    esac
}

# Replaces the quoted value only, so the rest of the line (a CR on a
# Windows checkout) stays as it was.
set_version() {
    local v=$1 tmp
    valid "$v" || { echo "'$v' is not a version X.Y.Z" >&2; exit 1; }
    tmp=$(mktemp)
    awk -v v="$v" '
        /^\[/ { pkg = ($0 ~ /^\[package\][[:space:]]*$/) }
        pkg && !done && /^version[[:space:]]*=/ { sub(/"[^"]*"/, "\"" v "\""); done = 1 }
        { print }
        END { if (!done) exit 1 }
    ' Cargo.toml >"$tmp" || { rm -f "$tmp"; echo "no [package] version in Cargo.toml" >&2; exit 1; }
    cat "$tmp" >Cargo.toml
    awk -v v="$v" '
        /^name = "audioboard"/ { app = 1; print; next }
        app && /^version = / { sub(/"[^"]*"/, "\"" v "\""); app = 0; done = 1 }
        { print }
        END { if (!done) exit 1 }
    ' Cargo.lock >"$tmp" || { rm -f "$tmp"; echo "no audioboard entry in Cargo.lock" >&2; exit 1; }
    cat "$tmp" >Cargo.lock
    rm -f "$tmp"
    [ "$(current)" = "$v" ] || { echo "Cargo.toml still says $(current)" >&2; exit 1; }
}

case "${1:-}" in
    "") current ;;
    next) next "${2:-}" ;;
    set) set_version "${2:-}" ;;
    *) sed -n '7,9p' "$0" | sed 's/^# //' >&2; exit 1 ;;
esac
