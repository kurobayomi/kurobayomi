#!/usr/bin/env bash
# Packs a Windows build into a folder that runs on a PC without MSYS2, in GTK's prefix layout (GLib finds its data next to
# bin\): the exe and every DLL it loads from MSYS2's UCRT64 prefix in bin\, GDK's image loaders in lib\, and the GSettings
# schemas, icon themes and font settings in share\ and etc\, and the licences of all of it in share\licenses\kurobayomi\. Runs in
# the MSYS2 UCRT64 shell (ntldd, cygpath, pacman).
# Usage: scripts/bundle-windows.sh <path to kurobayomi.exe> <output folder>
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exe=$1
out=$2
prefix=${MINGW_PREFIX:-/ucrt64}

rm -rf "$out"
mkdir -p "$out/bin" "$out/lib" "$out/share/glib-2.0/schemas" "$out/share/icons"
cp "$exe" "$out/bin/"
# without its debug info: a release build carries it in the exe (over 400 MB), the AppImage keeps it apart
strip --strip-debug "$out/bin/$(basename "$exe")"
# GLib's helper that starts a session bus, which GApplication's single instance uses on Windows
cp "$prefix/bin/gdbus.exe" "$out/bin/"
# the programs anime downloads and local video previews run: a Windows PC seldom has them, and their libraries are
# bundled anyway (libmpv uses them). The app looks next to itself before PATH (kuro_platform::process::find_program)
cp "$prefix/bin/ffmpeg.exe" "$prefix/bin/ffprobe.exe" "$out/bin/"
# WebView2's loader, through which the hidden pages that pass a site's browser check start the system's Edge WebView2
# (the hidden pages of kuro-platform). It comes from the webview2-com-sys crate, not from MSYS2, so ntldd cannot find
# it, and the exe does not start without it
loader="$(ls -t "$(dirname "$exe")"/build/webview2-com-sys-*/out/x64/WebView2Loader.dll | head -1)"
cp "$loader" "$out/bin/"
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

# The licences that travel with the app, as in the AppImage (scripts/build-appimage.sh): ours (LICENSE, NOTICE) and, for
# every bundled file from MSYS2, the licence files of the package it came from with that package's version, so its
# source can be found (SOURCE-OFFER.txt). A bundle with no library traced to a package fails.
lic="$out/share/licenses/kurobayomi"
mkdir -p "$lic/libraries"
cp "$root/LICENSE" "$root/NOTICE" "$lic/"
{
    head -n 2 "$root/src/shell/data/SOURCE-OFFER.txt"
    cat <<'OFFER'

This Windows build also carries libraries (GTK, libmpv, ffmpeg and the libraries they use) from MSYS2's UCRT64
repository. Each one is the MSYS2 package listed in libraries/PACKAGES.txt (name and version), and its licence files are
in libraries/<package>/. Many of them are LGPL or GPL. The source code of every one of them is available from MSYS2, at
https://packages.msys2.org/ and https://github.com/msys2/MINGW-packages (the package's PKGBUILD names its source).
If you cannot get a source you are entitled to, write to the project at https://github.com/kurobayomi/kurobayomi/issues.
OFFER
} > "$lic/SOURCE-OFFER.txt"
{
    for file in "${dlls[@]}" "$prefix/bin/"{gdbus,ffmpeg,ffprobe}.exe "$prefix/lib/gdk-pixbuf-2.0/2.10.0/loaders/"*.dll "$prefix/share/icons/Adwaita/index.theme"; do
        pacman -Qqo "$file" 2>/dev/null || true
    done
} | sort -u | while read -r package; do
    name="${package#"${MINGW_PACKAGE_PREFIX:-mingw-w64-ucrt-x86_64}-"}"
    if [ -d "$prefix/share/licenses/$name" ]; then
        cp -r "$prefix/share/licenses/$name" "$lic/libraries/$name"
    fi
    pacman -Q "$package" >> "$lic/libraries/PACKAGES.txt"
done
if [ ! -s "$lic/libraries/PACKAGES.txt" ]; then
    echo "bundle-windows: no bundled library could be traced to an MSYS2 package; the licences are missing" >&2
    exit 1
fi
echo "bundle-windows: $(ls "$out/bin" | wc -l) files in bin, $(du -sh "$out" | cut -f1) in all"
