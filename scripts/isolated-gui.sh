#!/usr/bin/env bash
# Runs a command (a test copy of the app, a GUI test run) on its own session bus with the desktop openers stubbed, and
# stops what that bus started once the command ends.
# - The own bus keeps the run off the real desktop: file choosers, notifications and the password store.
# - The stubs keep a test from opening the real browser or file manager.
# - dbus-run-session leaves the portal and the password store it started running after it ends; 530 of them once held
#   4.7 GB of swap, so every process on the run's bus is stopped at the end.
# Usage: scripts/isolated-gui.sh cargo test -p kurobayomi --no-default-features -- browse::
set -euo pipefail

if [ "$#" -eq 0 ]; then
    echo "usage: $0 command [args...]" >&2
    exit 2
fi

stubs=$(mktemp -d)
trap 'rm -rf "$stubs"' EXIT
for name in xdg-open kde-open gio brave firefox sensible-browser; do
    printf '#!/bin/sh\nexit 0\n' > "$stubs/$name"
    chmod +x "$stubs/$name"
done

status=0
PATH="$stubs:$PATH" BROWSER=/bin/true dbus-run-session -- bash -c '
    status=0
    "$@" || status=$?
    for pid in $(pgrep -u "$(id -u)"); do
        [ "$pid" = "$$" ] && continue
        if cat "/proc/$pid/environ" 2>/dev/null | tr "\0" "\n" | grep -qxF "DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS"; then
            kill "$pid" 2>/dev/null || true
        fi
    done
    exit "$status"
' bash "$@" || status=$?
exit "$status"
