#!/usr/bin/env bash
# Packs a Windows build into a folder that runs on a PC without MSYS2, in GTK's prefix layout (GLib finds its data next to
# bin\): the exe and every DLL it loads from MSYS2's UCRT64 prefix in bin\, GDK's image loaders in lib\, and the GSettings
# schemas, icon themes and font settings in share\ and etc\. Runs in the MSYS2 UCRT64 shell (ntldd, cygpath).
# Usage: scripts/bundle-windows.sh <path to kurobayomi.exe> <output folder>
set -euo pipefail

exe=$1
out=$2
prefix=${MINGW_PREFIX:-/ucrt64}

rm -rf "$out"
mkdir -p "$out/bin" "$out/lib" "$out/share/glib-2.0/schemas" "$out/share/icons"
cp "$exe" "$out/bin/"
# GLib's helper that starts a session bus, which GApplication's single instance uses on Windows
cp "$prefix/bin/gdbus.exe" "$out/bin/"
# the image loaders (librsvg's draws the SVG icons) with their cache, whose paths are relative to the prefix
cp -r "$prefix/lib/gdk-pixbuf-2.0" "$out/lib/"
cp "$prefix/share/glib-2.0/schemas/gschemas.compiled" "$out/share/glib-2.0/schemas/"
cp -r "$prefix/share/icons/Adwaita" "$prefix/share/icons/hicolor" "$out/share/icons/"
if [ -d "$prefix/etc/fonts" ]; then
    mkdir -p "$out/etc"
    cp -r "$prefix/etc/fonts" "$out/etc/"
fi

# every DLL the exe, gdbus and the loaders load from the prefix (ntldd -R follows the whole tree); Windows' own DLLs
# are on every PC and stay out
mapfile -t dlls < <(
    ntldd -R "$out/bin/"*.exe "$out/lib/gdk-pixbuf-2.0/2.10.0/loaders/"*.dll |
        sed -n 's/^[[:space:]]*[^[:space:]]* => \(.*\) (0x[0-9a-fA-F]*)$/\1/p' |
        while read -r dll; do cygpath -u "$dll"; done |
        grep -i "^$prefix/bin/" | sort -u
)
if [ "${#dlls[@]}" -eq 0 ]; then
    echo "bundle-windows: ntldd found no DLL from $prefix; the output format may have changed" >&2
    exit 1
fi
cp "${dlls[@]}" "$out/bin/"
echo "bundle-windows: $(ls "$out/bin" | wc -l) files in bin, $(du -sh "$out" | cut -f1) in all"
