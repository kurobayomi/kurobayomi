#!/usr/bin/env bash
# Writes the installed AppStream metadata for release VERSION to stdout, from
# src/shell/data/io.github.kurobayomi.Kurobayomi.metainfo.xml.in: the screenshots link to the tag vVERSION, and the
# releases are the newest ten versions of CHANGELOG.md, each with its notes (### headings become paragraphs, bullets list
# items). The changelog stays the one place release notes are written; the GitHub release reads it too
# (release-notes.sh). Usage: scripts/metainfo.sh 0.3.2 > io.github.kurobayomi.Kurobayomi.metainfo.xml
set -euo pipefail
version="${1:?usage: metainfo.sh VERSION}"
root="$(dirname "${BASH_SOURCE[0]}")/.."
template="$root/src/shell/data/io.github.kurobayomi.Kurobayomi.metainfo.xml.in"
grep -q "^## \[$version\] - " "$root/CHANGELOG.md" || { echo "CHANGELOG.md has no section for $version" >&2; exit 1; }

releases="$(awk '
    function esc(s) { gsub(/&/, "\\&amp;", s); gsub(/</, "\\&lt;", s); gsub(/>/, "\\&gt;", s); gsub(/`/, "", s); return s }
    function flush_item() { if (item != "") { print "          <li>" esc(item) "</li>"; item = "" } }
    function close_list() { flush_item(); if (in_list) { print "        </ul>"; in_list = 0 } }
    function close_release() { if (open) { close_list(); print "      </description>"; print "    </release>"; open = 0 } }
    /^## \[Unreleased\]/ { next }
    /^## \[[0-9]/ {
        close_release()
        if (++count > 10) exit
        match($0, /\[[^]]*\]/); v = substr($0, RSTART + 1, RLENGTH - 2)
        match($0, /[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]/); d = substr($0, RSTART, RLENGTH)
        print "    <release version=\"" v "\" date=\"" d "\">"; print "      <description>"; open = 1; next
    }
    !open { next }
    /^### / { close_list(); print "        <p>" esc(substr($0, 5)) "</p>"; next }
    /^- / { flush_item(); if (!in_list) { print "        <ul>"; in_list = 1 }; item = substr($0, 3); next }
    /^  +[^ ]/ && item != "" { sub(/^ +/, ""); item = item " " $0; next }
    /^[[:space:]]*$/ { flush_item(); next }
    END { close_release() }
' "$root/CHANGELOG.md")"

awk -v tag="v$version" -v releases="$releases" '
    { gsub(/@TAG@/, tag) }
    /^@RELEASES@$/ { print releases; next }
    { print }
' "$template"
