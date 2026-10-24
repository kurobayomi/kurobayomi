#!/usr/bin/env bash
# Packs a built kurobayomi binary into an AppImage: GTK 4 (through linuxdeploy's GTK plugin), libmpv and the other shared
# libraries travel inside it.
#
# Usage: [CHANNEL=nightly] [FFMPEG_PREFIX=DIR] scripts/build-appimage.sh BINARY OUTPUT.AppImage
# CHANNEL=nightly packs the Nightly app: its own app id, name and gold icon, and it leaves the file types and links to the
# stable app. Without it the stable app is packed. FFMPEG_PREFIX names a decode-only FFmpeg built by
# scripts/build-ffmpeg.sh, bundled in place of the system's (whose encoder libraries are about 33 MiB of the AppImage),
# with its ffmpeg and ffprobe programs next to the app (anime downloads and video previews run them).
# The binary's debug info is left out of the AppImage and written next to it as OUTPUT without .AppImage, plus .debug.
# Needs the libraries the binary links (libgtk-4, libmpv, ...) installed on this machine: they are what gets bundled, so
# build on the oldest distro you want the AppImage to run on (CI uses Ubuntu 24.04, the first with GTK 4.14).
set -euo pipefail

bin="$(readlink -f "${1:?usage: build-appimage.sh BINARY OUTPUT.AppImage}")"
out="$(readlink -m "${2:?usage: build-appimage.sh BINARY OUTPUT.AppImage}")"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
data="$root/src/shell/data"
id=io.github.anas1412.Kurobayomi
icons="$data/icons"
# what an AppImage updater (Gear Lever, AppImageUpdate, ...) follows: the nightly release, or the newest stable one
update_info="gh-releases-zsync|kurobayomi|kurobayomi|latest|Kurobayomi-x86_64.AppImage.zsync"
if [ "${CHANNEL:-stable}" = nightly ]; then
    id=$id.Nightly
    icons="$data/icons-nightly"
    update_info="gh-releases-zsync|kurobayomi|kurobayomi|nightly|Kurobayomi-nightly-x86_64.AppImage.zsync"
fi
tools="${TOOLS_DIR:-$root/target/appimage-tools}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
appdir="$work/AppDir"

# the packaging tools, pinned by version and checked by hash
fetch() { # fetch NAME URL SHA256
    local file="$tools/$1"
    if ! { [ -f "$file" ] && echo "$3  $file" | sha256sum -c --status; }; then
        mkdir -p "$tools"
        curl -fsSL "$2" -o "$file"
        echo "$3  $file" | sha256sum -c --status || { echo "checksum mismatch: $1" >&2; exit 1; }
    fi
    chmod +x "$file"
}
fetch linuxdeploy https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d
fetch linuxdeploy-plugin-gtk.sh https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/7a3fbc31a9e5075073ff8790f26effbac5f84453/linuxdeploy-plugin-gtk.sh b0f4cbc684a0103a9651f0955b635eaea0096b3a66c0f5a2c2aa337960375171
fetch appimagetool https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0

# the app's files, laid out like an install into /usr
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications" "$appdir/usr/share/mime/packages"
install -m755 "$bin" "$appdir/usr/bin/kurobayomi"
# The debug info was about 51 of the AppImage's MiB. It moves to its own file, which the binary names (a debug link), so
# a debugger or addr2line given that file turns a crash report's addresses back into files and lines; the report itself
# still names the functions (the symbol table stays).
debug="${out%.AppImage}.debug"
mkdir -p "$(dirname "$debug")"
objcopy --only-keep-debug --compress-debug-sections=zstd "$appdir/usr/bin/kurobayomi" "$debug"
strip --strip-debug "$appdir/usr/bin/kurobayomi"
objcopy --add-gnu-debuglink="$debug" "$appdir/usr/bin/kurobayomi"
# every size and the SVG, into the one folder desktops read (the nightly's folder has another name in the source)
mkdir -p "$appdir/usr/share/icons"
cp -r "$icons/." "$appdir/usr/share/icons/"
# no D-Bus activation from inside an AppImage: there is no installed service file for the bus to start
desktop="$data/io.github.anas1412.Kurobayomi.desktop"
if [ "${CHANNEL:-stable}" = nightly ]; then
    sed -e "s/io\.github\.anas1412\.Kurobayomi/$id/g" -e 's/^Name=Kurobayomi$/Name=Kurobayomi Nightly/' \
        -e '/^DBusActivatable=/d' -e '/^MimeType=/d' "$desktop" > "$appdir/usr/share/applications/$id.desktop"
else
    install -m644 "$data/$id.xml" "$appdir/usr/share/mime/packages/$id.xml"
    grep -v '^DBusActivatable=' "$desktop" > "$appdir/usr/share/applications/$id.desktop"
fi

# the plugin must be on PATH under the name linuxdeploy looks for; the AppImages run unpacked so no FUSE is needed
mkdir -p "$work/bin"
ln -s "$tools/linuxdeploy-plugin-gtk.sh" "$work/bin/linuxdeploy-plugin-gtk.sh"
export PATH="$work/bin:$PATH" APPIMAGE_EXTRACT_AND_RUN=1 DEPLOY_GTK_VERSION=4 NO_STRIP=1
# NO_STRIP: linuxdeploy would strip the symbol table too, and the crash reports name functions from it (the debug info
# is already split off above)
executables=(--executable "$appdir/usr/bin/kurobayomi")
if [ -n "${FFMPEG_PREFIX:-}" ]; then
    # linuxdeploy finds libraries through LD_LIBRARY_PATH first, so it bundles this FFmpeg rather than the system's
    [ -f "$FFMPEG_PREFIX/lib/libavcodec.so.60" ] || { echo "no libavcodec.so.60 in $FFMPEG_PREFIX/lib" >&2; exit 1; }
    export LD_LIBRARY_PATH="$FFMPEG_PREFIX/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    # the programs go next to the app, where it looks for them before PATH (kuro_platform::process::find_program)
    for program in ffmpeg ffprobe; do
        install -m755 "$FFMPEG_PREFIX/bin/$program" "$appdir/usr/bin/$program"
        executables+=(--executable "$appdir/usr/bin/$program")
    done
fi

"$tools/linuxdeploy" --appdir "$appdir" --plugin gtk \
    "${executables[@]}" \
    --desktop-file "$appdir/usr/share/applications/$id.desktop" \
    --icon-file "$icons/hicolor/256x256/apps/$id.png" --icon-filename "$id"
if [ -n "${FFMPEG_PREFIX:-}" ]; then
    # the bundled programs start with the bundled libraries alone (linuxdeploy set their rpath)
    for program in ffmpeg ffprobe; do
        LD_LIBRARY_PATH= "$appdir/usr/bin/$program" -hide_banner -version >/dev/null || { echo "the bundled $program does not start" >&2; exit 1; }
    done
fi

# The GTK plugin's launcher hook forces `GTK_THEME=Adwaita:<light|dark>` (so the app ignores the user's GTK theme) and
# `GDK_BACKEND=x11` (so it runs through XWayland on a Wayland desktop). Both are removed: the app follows the user's theme
# and uses the session's own display protocol. The check fails the build if a plugin update brings either line back.
hook="$appdir/apprun-hooks/linuxdeploy-plugin-gtk.sh"
sed -i -e '/^COLOR_SCHEME=/,/^APPIMAGE_GTK_THEME=/d' -e '/^export GTK_THEME=/d' -e '/^export GDK_BACKEND=/d' "$hook"
if grep -qE '^export (GTK_THEME|GDK_BACKEND)=|APPIMAGE_GTK_THEME' "$hook"; then
    echo "the AppRun hook still forces a theme or a display backend: $hook" >&2
    exit 1
fi

# On Wayland GTK reads the user's GTK theme, icon theme and fonts through GSettings, which sees the user's settings only
# through the dconf GIO module. The bundled GIO looks for modules in Ubuntu's folder alone (/usr/lib/x86_64-linux-gnu/gio/
# modules), so on another distro it found none, GSettings fell back to its defaults and every theme looked like Adwaita. The
# module travels inside the AppImage and GIO is told where (an extra folder, so a distro's own modules still load).
dconf_module=/usr/lib/x86_64-linux-gnu/gio/modules/libdconfsettings.so
[ -f "$dconf_module" ] || { echo "missing $dconf_module: install dconf-gsettings-backend" >&2; exit 1; }
mkdir -p "$appdir/usr/lib/gio/modules"
cp "$dconf_module" "$appdir/usr/lib/gio/modules/"
echo 'export GIO_EXTRA_MODULES="$APPDIR/usr/lib/gio/modules${GIO_EXTRA_MODULES:+:$GIO_EXTRA_MODULES}"' >> "$hook"
# WebKit (through libsoup) makes its HTTPS connections with GIO's TLS module, glib-networking's libgiognutls. The bundled GIO
# looks in Ubuntu's folder alone, so on another distro WebKit failed every HTTPS page with "TLS support is not available" (and
# the app then fell back to the system browser). The module travels in the same extra folder, with the libraries it needs.
tls_module=/usr/lib/x86_64-linux-gnu/gio/modules/libgiognutls.so
[ -f "$tls_module" ] || { echo "missing $tls_module: install glib-networking" >&2; exit 1; }
cp "$tls_module" "$appdir/usr/lib/gio/modules/"
"$tools/linuxdeploy" --appdir "$appdir" --deploy-deps-only "$appdir/usr/lib/gio/modules"

# WebKitGTK (the hidden pages that pass Cloudflare's browser check, `src/page_fetch.rs`) starts helper programs
# (WebKitWebProcess, WebKitNetworkProcess) from a folder whose absolute path is compiled into the library. Inside an
# AppImage that folder is elsewhere, so, as Tauri does, the helpers travel in the AppImage, the path in the library is
# rewritten to the same-length relative path `././/lib/...`, and the launcher starts the app from `$APPDIR/usr`, where that
# relative path resolves. The helpers find the bundled libraries through their rpath.
wk_dir=/usr/lib/x86_64-linux-gnu/webkitgtk-6.0
[ -d "$wk_dir" ] || { echo "missing $wk_dir: install libwebkitgtk-6.0-dev" >&2; exit 1; }
mkdir -p "$appdir/usr/lib/x86_64-linux-gnu"
cp -a "$wk_dir" "$appdir/usr/lib/x86_64-linux-gnu/"
"$tools/linuxdeploy" --appdir "$appdir" --deploy-deps-only "$appdir/usr/lib/x86_64-linux-gnu/webkitgtk-6.0"
for helper in "$appdir"/usr/lib/x86_64-linux-gnu/webkitgtk-6.0/WebKit*Process; do
    patchelf --set-rpath '$ORIGIN/../..' "$helper"
done
patched=0
for lib in "$appdir"/usr/lib/libwebkitgtk-6.0.so*; do
    [ -f "$lib" ] && [ ! -L "$lib" ] || continue
    sed -i "s|$wk_dir|././/lib/x86_64-linux-gnu/webkitgtk-6.0|g" "$lib"
    patched=$((patched + 1))
done
[ "$patched" -gt 0 ] || { echo "libwebkitgtk-6.0 was not bundled" >&2; exit 1; }
if grep -qa "$wk_dir" "$appdir"/usr/lib/libwebkitgtk-6.0.so*; then
    echo "libwebkitgtk-6.0 still names $wk_dir" >&2
    exit 1
fi
echo 'cd "$APPDIR/usr"' >> "$hook"
# WebKit starts its helpers inside a bubblewrap sandbox that needs bwrap and xdg-dbus-proxy from the host and the helper
# folder at its own path: neither holds here (the folder is rewritten to a relative path, and a host may lack the tools), and
# a failed launch aborts the whole app ("bwrap: Can't mkdir parents ...", "Failed to fully launch dbus-proxy"). So inside the
# AppImage the helpers start directly. The pages only pass the browser check of sites the user installed sources for.
echo 'export WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1' >> "$hook"

# libmpv links the client libraries of two sound servers, PipeWire and JACK. The AppImage exclude list keeps both out
# (each must match the server installed on the host), so a system without them could not even start the app ("error while
# loading shared libraries: libpipewire-0.3.so.0", the AppImage catalog's test). They travel as fallbacks, one folder
# each, put on the library path only when the system has no copy: a host with the server keeps its own library, and on a
# host without, the bundled one loads, finds no server, and mpv plays its sound through PulseAudio or ALSA instead.
fallbacks=(libpipewire-0.3.so.0 libjack.so.0)
for soname in "${fallbacks[@]}"; do
    found="$(ldconfig -p | awk -v n="$soname" '$1 == n && /x86-64/ {print $NF; exit}')"
    [ -n "$found" ] || { echo "missing $soname: install the package that has it, so it can travel as a fallback" >&2; exit 1; }
    mkdir -p "$appdir/usr/lib/fallback/$soname"
    cp -L "$found" "$appdir/usr/lib/fallback/$soname/$soname"
    "$tools/linuxdeploy" --appdir "$appdir" --deploy-deps-only "$appdir/usr/lib/fallback/$soname"
done
# the launcher's check: the system's library when the linker or one of the usual folders has it, else the bundled one
cat >> "$hook" <<'EOF'
kb_system_has() {
    if { /sbin/ldconfig -p 2>/dev/null || ldconfig -p 2>/dev/null; } | grep -q "^[[:space:]]*$1 "; then return 0; fi
    for kb_dir in /usr/lib/x86_64-linux-gnu /lib/x86_64-linux-gnu /usr/lib64 /usr/lib; do
        if [ -e "$kb_dir/$1" ]; then return 0; fi
    done
    return 1
}
EOF
echo "for kb_lib in ${fallbacks[*]}; do" >> "$hook"
cat >> "$hook" <<'EOF'
    kb_system_has "$kb_lib" || export LD_LIBRARY_PATH="$APPDIR/usr/lib/fallback/$kb_lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
done
EOF

# GTK's gstreamer media module links the host's gstreamer, which fails against the bundled glib on newer distros (an
# "undefined symbol" warning at start); the app plays video through libmpv, so the module is not needed
rm -rf "$appdir"/usr/lib/gtk-4.0/*/media

# The licences that travel with the AppImage: ours (LICENSE, NOTICE) and, for every bundled system library, the copyright file
# of the Debian/Ubuntu package it came from, with that package's version, so the source of each library can be found
# (SOURCE-OFFER.txt). Without dpkg (a local build on another distro) the library part is skipped with a warning; on the CI
# runner it must find something, or the build fails.
lic="$appdir/usr/share/licenses/kurobayomi"
mkdir -p "$lic/libraries"
install -m644 "$root/LICENSE" "$root/NOTICE" "$lic/"
cp "$data/SOURCE-OFFER.txt" "$lic/SOURCE-OFFER.txt"
if command -v dpkg >/dev/null; then
    : > "$lic/libraries/PACKAGES.txt"
    {
        for lib in "$appdir"/usr/lib/*.so* "$appdir"/usr/lib/fallback/*/*.so*; do
            found="$(ldconfig -p | awk -v n="$(basename "$lib")" '$1 == n {print $NF; exit}')"
            if [ -n "$found" ]; then
                dpkg -S "$(readlink -f "$found")" 2>/dev/null | head -1 | cut -d: -f1 || true
            fi
        done
        dpkg -S "$dconf_module" 2>/dev/null | head -1 | cut -d: -f1 || true
    } | sort -u | while read -r pkg; do
        [ -f "/usr/share/doc/$pkg/copyright" ] && cp "/usr/share/doc/$pkg/copyright" "$lic/libraries/$pkg.copyright"
        dpkg-query -W -f='${Package} ${Version}\n' "$pkg" >> "$lic/libraries/PACKAGES.txt"
    done
    [ -s "$lic/libraries/PACKAGES.txt" ] || { echo "no bundled library could be traced to a package: the licences are missing" >&2; exit 1; }
else
    echo "warning: dpkg not found, the licence texts of the bundled libraries are not included" >&2
fi
if [ -n "${FFMPEG_PREFIX:-}" ]; then
    # the bundled FFmpeg is ours, not a package's: its licence and where its source and build flags are
    cp -r "$FFMPEG_PREFIX/share/licenses/ffmpeg" "$lic/libraries/ffmpeg"
    # and it really is the one bundled: none of the system FFmpeg's encoder libraries came along
    if ls "$appdir"/usr/lib/libx265.so* "$appdir"/usr/lib/libSvtAv1Enc.so* >/dev/null 2>&1; then
        echo "the system FFmpeg was bundled instead of $FFMPEG_PREFIX" >&2
        exit 1
    fi
fi

mkdir -p "$(dirname "$out")"
# update information for AppImage updaters (Gear Lever, AppImageUpdate, ...): they fetch only the changed blocks of the
# newest file of the channel. appimagetool runs zsyncmake, which writes the .zsync file into the current folder, so run it
# in the output folder to get it next to the AppImage.
# zstd in 1 MiB blocks at level 19 instead of 128 KiB at the default level: about 8% smaller (level 22 gave nothing more)
squash=(--mksquashfs-opt -b --mksquashfs-opt 1M --mksquashfs-opt -Xcompression-level --mksquashfs-opt 19)
(cd "$(dirname "$out")" && ARCH=x86_64 "$tools/appimagetool" --no-appstream "${squash[@]}" -u "$update_info" "$appdir" "$(basename "$out")")
[ -f "$out.zsync" ] || { echo "no $out.zsync was written (is zsyncmake installed?)" >&2; exit 1; }
echo "built $out ($(du -h "$out" | cut -f1)) and $out.zsync"
