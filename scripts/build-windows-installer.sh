#!/usr/bin/env bash
# Builds the Windows installer (dist/windows/kurobayomi.iss) from a bundle of scripts/bundle-windows.sh with Inno Setup's
# compiler. Runs in the MSYS2 shell after a release build (it takes the app icon build.rs wrote).
# Usage: scripts/build-windows-installer.sh <stable|nightly> <bundle folder> <output folder>
set -euo pipefail

channel=$1
bundle=$2
out=$3
version="$(grep -m1 '^version = ' Cargo.toml | sed -E 's/^version = "(.*)"$/\1/')"
icon="$(ls -t target/release/build/kurobayomi-*/out/kurobayomi.ico | head -1)"
iscc="${ISCC:-/c/Program Files (x86)/Inno Setup 6/ISCC.exe}"

mkdir -p "$out"
# MSYS2 rewrites arguments that start with / as paths; ISCC's /D options must reach it as written. The paths are absolute:
# ISCC reads a relative one from the script's folder (dist\windows)
MSYS2_ARG_CONV_EXCL='*' "$iscc" /Q \
    "/DChannel=$channel" "/DVersion=$version" \
    "/DBundle=$(cygpath -wa "$bundle")" "/DIcon=$(cygpath -wa "$icon")" "/DOutput=$(cygpath -wa "$out")" \
    "$(cygpath -wa dist/windows/kurobayomi.iss)"
ls -l "$out"
