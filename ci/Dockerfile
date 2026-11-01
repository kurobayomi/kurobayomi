# The build machine of CI, the nightly and the stable release: Ubuntu 24.04 with every system package they need, built by
# .github/workflows/build-image.yml and published to ghcr.io/<owner>/kurobayomi-build. A run then starts from it instead
# of installing the packages from Ubuntu's mirror (one release run waited 73 minutes there).
#
# Ubuntu 24.04 on purpose: the AppImage bundles these libraries, so its glibc (2.39) is the oldest the AppImage runs on,
# and it is the first Ubuntu with GTK 4.14, which the gtk4 v4_14 feature needs. scripts/build-appimage.sh expects
# Ubuntu's folder layout.
FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

# The container image leaves out most of /usr/share/doc; the AppImage carries the copyright file of every library it
# bundles (build-appimage.sh), so packages install in full, as on a normal Ubuntu.
RUN rm -f /etc/dpkg/dpkg.cfg.d/excludes

# One list for all three workflows, recommended packages included as on the runners (the AppImage bundles some of them):
# - the app: GTK 4, WebKitGTK, libmpv, dav1d (the image crate's avif-native feature)
# - the tests: ripgrep, desktop-file-utils (desktop-file-validate), libxml2-utils, nodejs (the feed widget's JS check,
#   skipped without it; the runners have it)
# - the AppImage: patchelf, glib-networking, dconf-gsettings-backend, file, zsync
# - the decode-only FFmpeg (scripts/build-ffmpeg.sh): nasm and the -dev packages after it
# - the jobs themselves: git (checkout and the nightly tag), gh (releases), curl, xz-utils, zstd (actions/cache)
RUN apt-get update \
 && apt-get install -y \
      build-essential pkg-config ca-certificates curl git gh xz-utils zstd \
      libgtk-4-dev libadwaita-1-dev libwebkitgtk-6.0-dev libdav1d-dev libmpv-dev \
      ripgrep desktop-file-utils libxml2-utils nodejs \
      patchelf glib-networking dconf-gsettings-backend file zsync \
      nasm libgnutls28-dev libxml2-dev libva-dev libvdpau-dev libdrm-dev zlib1g-dev libbz2-dev \
 && rm -rf /var/lib/apt/lists/*

# The jobs run as root on a checkout owned by the runner's user, which git refuses ("dubious ownership") once the checkout
# step is over: the nightly's tag push failed on it
RUN git config --system --add safe.directory '*'

# Rust stable, as rust-toolchain.toml asks; the jobs still run `rustup update stable` for a newer one
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable \
 && chmod -R a+w "$RUSTUP_HOME" "$CARGO_HOME"
