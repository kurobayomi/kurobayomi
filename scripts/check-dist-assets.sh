#!/usr/bin/env bash
# Checks the icons and the dev desktop entry (T-54a-012, T-54a-028).
# Usage: scripts/check-dist-assets.sh [icon root]   (default: dist/icons/hicolor)
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
root="${1:-dist/icons/hicolor}"
id=io.github.kurobayomi.Kurobayomi
fail() { echo "FAIL: $*" >&2; exit 1; }

for f in "$root/scalable/apps/$id.svg" "$root/512x512/apps/$id.png" "$root/scalable/apps/$id.Devel.svg"; do
    [ -f "$f" ] && [ ! -L "$f" ] || fail "$f is not a regular file"
done
[ "$(file -b "$root/512x512/apps/$id.png" | grep -c '512 x 512')" = 1 ] || fail "PNG is not 512x512"

dev="$root/scalable/apps/$id.Devel.svg"
for c in '#C92824' '#455A64' '#607D8B'; do
    grep -qi "$c" "$dev" || fail "dev icon lacks $c"
done
if cmp -s "$dev" "$root/scalable/apps/$id.svg"; then fail "dev icon equals release icon"; fi

desktop=dist/$id.Devel.desktop
grep -qx 'Name=Kurobayomi (dev)' "$desktop" || fail "dev desktop entry name"
grep -qx "Icon=$id.Devel" "$desktop" || fail "dev desktop entry icon"
if command -v desktop-file-validate >/dev/null; then desktop-file-validate "$desktop" | grep -v hint || true; fi
echo "OK: dist assets"
