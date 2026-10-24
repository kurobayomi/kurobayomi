#!/usr/bin/env bash
# Builds a decode-only FFmpeg into <prefix> for the AppImage. Ubuntu's FFmpeg links about thirty outside libraries
# (x264, x265, SVT-AV1, rav1e, aom, codec2, flite voices, pocketsphinx with LAPACK, …), mostly encoders the app never
# uses, and they made up about 33 MiB of the AppImage. This is the same version Ubuntu 24.04 ships (6.1.1, the sonames
# libavcodec.so.60 and so on, with the same symbol versions), so Ubuntu's libmpv loads it in place of Ubuntu's FFmpeg.
# Only what playback needs is enabled: the built-in decoders and demuxers, dav1d for AV1, TLS for https streams, libxml2
# for DASH, and VA-API and VDPAU for hardware decoding. LGPL: no --enable-gpl, so no libpostproc (libmpv does not need it).
# The ffmpeg and ffprobe programs are built too: anime downloads copy streams into Matroska (no encoding) and local
# videos get a preview frame, and the AppImage carries them so this works without a system ffmpeg.
# Usage: scripts/build-ffmpeg.sh <prefix> [extra configure flags]
set -euo pipefail

prefix="$(realpath -m "$1")"
shift
version=6.1.1
sha256=8684f4b00f94b85461884c3719382f1261f0d9eb3d59640a1f4ac0873616f968

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl -fsSL "https://ffmpeg.org/releases/ffmpeg-$version.tar.xz" -o "$work/ffmpeg.tar.xz"
echo "$sha256  $work/ffmpeg.tar.xz" | sha256sum -c --quiet
tar -xf "$work/ffmpeg.tar.xz" -C "$work"

cd "$work/ffmpeg-$version"
./configure --prefix="$prefix" --enable-shared --disable-static --disable-ffplay --disable-doc \
    --disable-autodetect --enable-pthreads --enable-zlib --enable-bzlib --enable-iconv \
    --enable-gnutls --enable-libdav1d --enable-libxml2 --enable-libdrm --enable-vaapi --enable-vdpau \
    --disable-indevs --disable-outdevs "$@"
make -j"$(nproc)"
make install
# the licence and the source, for the AppImage's licence folder (scripts/build-appimage.sh copies it)
mkdir -p "$prefix/share/licenses/ffmpeg"
cp COPYING.LGPLv2.1 "$prefix/share/licenses/ffmpeg/"
{
    echo "FFmpeg $version, unmodified, from https://ffmpeg.org/releases/ffmpeg-$version.tar.xz (sha256 $sha256),"
    echo "built by scripts/build-ffmpeg.sh in https://github.com/kurobayomi/kurobayomi with:"
    sed -n 's/^#define FFMPEG_CONFIGURATION "\(.*\)"$/\1/p' config.h
} > "$prefix/share/licenses/ffmpeg/SOURCE.txt"
