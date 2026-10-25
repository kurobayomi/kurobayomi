#!/usr/bin/env python3
"""Writes the app's own icon set: every standard icon name the app uses, drawn with a Material icon.

The app shows its own icons on every desktop (user, 2026-10-14): an icon theme whose "-symbolic" icons are colour drawings
turned them into blobs. `icons::register` points GTK at the app's own theme, so these files win over the user's theme.

Python 3 standard library only. Run from the repository root after changing the table:

    python3 src/theme/gen/material_icons.py

The Material icons come from the `@material-design-icons/svg` package (Apache License 2.0, see NOTICE) at a fixed version.
Each file is filled with `#bebebe`, so GTK recolours it to the widget's text colour.
"""

import os
import re
import sys
import urllib.request

VERSION = "0.14.13"
URL = "https://cdn.jsdelivr.net/npm/@material-design-icons/svg@{version}/{style}/{name}.svg"
OUT = "src/theme/icons/scalable/actions"
SYMBOLIC_FILL = "#bebebe"

# standard name (without "-symbolic") -> Material icon; "filled/" picks the filled style, a trailing "!flip" mirrors it
# top to bottom
ICONS = {
    "accessories-dictionary": "menu_book",
    "action-unavailable": "block",
    "applications-system": "settings",
    "application-x-addon": "extension",
    "audio-x-generic": "audiotrack",
    "avatar-default": "person",
    "bookmark-filled": "filled/bookmark",
    "bookmark-new": "bookmark_add",
    "checkbox-checked": "filled/check_box",
    "checkbox-mixed": "filled/indeterminate_check_box",
    "checkbox": "check_box_outline_blank",
    "compass": "explore",
    "content-loading": "hourglass_empty",
    "dialog-error": "error",
    "dialog-information": "info",
    "dialog-warning": "warning",
    "document-edit": "edit",
    "document-open-recent": "history",
    "document-open": "folder_open",
    "document-send": "send",
    "drive-harddisk": "storage",
    "edit-clear-all": "clear_all",
    "edit-clear": "clear",
    "edit-copy": "content_copy",
    "edit-find": "search",
    "edit-select-all": "select_all",
    "edit-undo": "undo",
    "emblem-important": "priority_high",
    "emblem-ok": "check_circle",
    "emblem-synchronizing": "sync",
    "emblem-system": "settings",
    "external-link": "open_in_new",
    "folder-download": "download",
    "folder-new": "create_new_folder",
    "folder": "folder",
    "go-down": "arrow_downward",
    "go-next": "arrow_forward",
    "go-previous": "arrow_back",
    "go-up": "arrow_upward",
    "help-about": "info",
    "help-browser": "help_outline",
    "image-crop": "crop",
    "image-x-generic": "image",
    "kb-share": "share",
    "kb-swap": "swap_horiz",
    "input-keyboard": "keyboard",
    "list-add": "add",
    "list-drag-handle": "drag_handle",
    "list-remove": "remove",
    "media-playback-pause": "pause",
    "media-playback-start": "play_arrow",
    "media-record": "filled/fiber_manual_record",
    "media-skip-backward": "skip_previous",
    "media-skip-forward": "skip_next",
    "media-view-subtitles": "subtitles",
    "money": "payments",
    "network-server": "dns",
    "non-starred": "star_border",
    "object-flip-horizontal": "flip",
    "object-select": "check",
    "open-menu": "menu",
    "package-x-generic": "inventory_2",
    "pan-down": "expand_more",
    "pan-end": "chevron_right",
    "pan-up": "expand_less",
    "preferences-desktop-appearance": "palette",
    "security-low": "gpp_maybe",
    "software-update-available": "system_update",
    "starred": "filled/star",
    "system-search": "search",
    "tag-outline": "label",
    "tag": "filled/label",
    "user-available": "filled/circle",
    "user-trash": "delete",
    "video-display": "tv",
    "video-x-generic": "movie",
    "view-conceal": "visibility_off",
    "view-filter": "filter_list",
    "view-fullscreen": "fullscreen",
    "view-grid": "grid_view",
    "view-list": "view_list",
    "view-more-horizontal": "more_horiz",
    "view-more": "more_vert",
    "view-pin": "push_pin",
    "view-refresh": "refresh",
    "view-reveal": "visibility",
    "view-sort-ascending": "sort!flip",
    "view-sort-descending": "sort",
    "web-browser": "language",
    "window-close": "close",
    "x-office-calendar": "calendar_today",
    "x-office-document": "description",
}


def fetch(source):
    flip = source.endswith("!flip")
    source = source.removesuffix("!flip")
    style, name = source.split("/") if "/" in source else ("outlined", source)
    url = URL.format(version=VERSION, style=style, name=name)
    for attempt in range(3):
        try:
            with urllib.request.urlopen(url, timeout=30) as reply:
                text = reply.read().decode()
            break
        except TimeoutError:
            if attempt == 2:
                raise
    # the shapes, as paths and circles, without their own fill
    shapes = [re.sub(r'\s*fill="[^"]*"', "", attrs) for attrs in re.findall(r"<(?:path|circle) ([^>]*?)/?>", text)]
    if not shapes:
        sys.exit(f"no shape in {source}")
    transform = ' transform="matrix(1 0 0 -1 0 24)"' if flip else ""
    tags = re.findall(r"<(path|circle) ", text)
    body = "".join(f'\n  <{tag} {attrs.strip()} fill="{SYMBOLIC_FILL}"{transform}/>' for tag, attrs in zip(tags, shapes))
    return f'<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24">{body}\n</svg>\n'


def main():
    os.makedirs(OUT, exist_ok=True)
    for name, source in ICONS.items():
        with open(os.path.join(OUT, f"{name}-symbolic.svg"), "w") as out:
            out.write(fetch(source))
    print(f"wrote {len(ICONS)} icons to {OUT}")


if __name__ == "__main__":
    main()
