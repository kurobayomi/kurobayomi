#!/usr/bin/env bash
# Fails when OS-specific code appears outside crates/kuro-platform.
# Patterns: cfg(target_os) and cfg!(target_os), xdg-open, libsecret, zbus, XDG_, raw C calls (libc::), and any reach into
# a per-OS module (kuro_platform::linux, kuro_platform::windows): other crates use the shared modules, which pick the
# engine of the OS they are built for.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

pattern='cfg!?\(target_os|xdg-open|libsecret|zbus|XDG_|libc::|kuro_platform::(linux|windows)::'
status=0
matches=$(rg --line-number --no-heading --color never --hidden \
    --glob '!.git/**' \
    --glob '!crates/kuro-platform/**' \
    --glob '!target/**' \
    --glob '!plans/**' \
    --glob '!docs/audit/**' \
    --glob '!shots/**' \
    --glob '!Cargo.lock' \
    --glob '!scripts/check-platform-boundary.sh' \
    --glob '!scripts/build-appimage.sh' \
    --glob '!scripts/isolated-gui.sh' \
    --glob '!CONTRIBUTING.md' \
    -e "$pattern" .) || status=$?

case "$status" in
    0)
        echo "$matches"
        echo "FAIL: OS-specific code found outside crates/kuro-platform (see lines above)" >&2
        exit 1
        ;;
    1)
        echo "OK: no OS-specific code outside crates/kuro-platform"
        ;;
    *)
        echo "ERROR: rg failed with exit status $status" >&2
        exit "$status"
        ;;
esac
