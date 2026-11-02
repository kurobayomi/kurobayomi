#!/usr/bin/env python3
"""Exports the Android drawables used by Kurobayomi as GTK symbolic and full-colour SVGs.

Python 3 standard library only. Run once from the repository root:

    python3 src/theme/gen/export_icons.py /path/to/aniyomi-src/app/src/main/res/drawable src/theme/icons

Output layout is the icon theme layout that `icons::register` expects:
`scalable/actions/kb-*-symbolic.svg`, `symbolic/apps/...-symbolic.svg`, `scalable/apps/...svg`.
Symbolic icons are filled with `#bebebe` so GTK recolours them to the widget foreground.
"""

import os
import re
import sys
import xml.etree.ElementTree as ET

ANDROID = "{http://schemas.android.com/apk/res/android}"
SYMBOLIC_FILL = "#bebebe"

# (output name, source drawable)
ACTION_ICONS = [
    ("kb-animelibrary-filled-symbolic", "ic_animelibrary_filled_24dp.xml"),
    ("kb-animelibrary-outline-symbolic", "ic_animelibrary_outline_24dp.xml"),
    ("kb-crop-symbolic", "ic_crop_24dp.xml"),
    ("kb-crop-off-symbolic", "ic_crop_off_24dp.xml"),
    ("kb-done-previous-symbolic", "ic_done_prev_24dp.xml"),
    ("kb-download-circle-symbolic", "ic_download_item_24dp.xml"),
    ("kb-forward-10-symbolic", "ic_forward_10_24dp.xml"),
    ("kb-overflow-vertical-symbolic", "ic_overflow_24dp.xml"),
    ("kb-seek-triangle-symbolic", "ic_play_seek_triangle.xml"),
    ("kb-reader-default-symbolic", "ic_reader_default_24dp.xml"),
    ("kb-reader-ltr-symbolic", "ic_reader_ltr_24dp.xml"),
    ("kb-reader-rtl-symbolic", "ic_reader_rtl_24dp.xml"),
    ("kb-reader-vertical-symbolic", "ic_reader_vertical_24dp.xml"),
    ("kb-reader-continuous-vertical-symbolic", "ic_reader_continuous_vertical_24dp.xml"),
    ("kb-reader-webtoon-symbolic", "ic_reader_webtoon_24dp.xml"),
    ("kb-updates-outline-symbolic", "ic_updates_outline_24dp.xml"),
    ("kb-font-family-symbolic", "outline_brand_family_24.xml"),
    ("kb-text-shadow-symbolic", "sharp_shadow_24.xml"),
]

# T-49ba-041: drawables the port drops (no reference in the app). The export refuses to read them.
DROPPED_DRAWABLES = [
    "ic_arrow_back_24dp.xml", "ic_baseline_collections_24.xml", "ic_baseline_input_24.xml",
    "ic_baseline_open_in_new_24.xml", "ic_brightness_negative_20dp.xml", "ic_chrome_player_mode_24dp.xml",
    "ic_circle_200dp.xml", "ic_circle_right_200dp.xml", "ic_collections_baseline_state.xml",
    "ic_launcher_background.xml", "ic_pause_circle_filled_24.xml", "ic_play_circle_filled_24.xml",
    "ic_video_chapter_20dp.xml", "ic_volume_off_24dp.xml",
]


def hex_rgba_to_svg(color):
    """Android #AARRGGBB, #RRGGBB or #RGB to an (SVG colour, opacity) pair."""
    value = color.lstrip("#")
    if len(value) == 8:
        alpha = int(value[:2], 16) / 255.0
        return "#" + value[2:], alpha
    return "#" + value, 1.0


def fmt_opacity(alpha):
    return "" if alpha >= 1.0 else ' fill-opacity="%.4f"' % alpha


def vector_root(path):
    root = ET.parse(path).getroot()
    vector = root if root.tag == "vector" else root.find(".//vector")
    if vector is None:
        raise SystemExit("no <vector> in %s" % path)
    return vector


def viewport(vector):
    return (float(vector.get(ANDROID + "viewportWidth")), float(vector.get(ANDROID + "viewportHeight")))


def iter_paths(vector):
    """Filled path elements in document order. Clip paths and stroke-only paths are skipped."""
    for node in vector.iter("path"):
        fill = node.get(ANDROID + "fillColor")
        if fill is None or fill == "@android:color/transparent":
            continue
        yield node


def symbolic_svg(vector, size, transforms=None):
    vw, vh = viewport(vector)
    lines = ['<svg xmlns="http://www.w3.org/2000/svg" width="%d" height="%d" viewBox="0 0 %g %g">' % (size, size, vw, vh)]
    for node in iter_paths(vector):
        d = node.get(ANDROID + "pathData")
        if not d:
            continue
        extra = ""
        if node.get(ANDROID + "fillType") == "evenOdd":
            extra += ' fill-rule="evenOdd"'
        name = node.get(ANDROID + "name")
        if transforms and name in transforms:
            extra += ' transform="%s"' % transforms[name]
        lines.append('  <path d="%s" fill="%s"%s/>' % (d, SYMBOLIC_FILL, extra))
    lines.append("</svg>")
    return "\n".join(lines) + "\n"


def colour_svg(vector, size=None):
    vw, vh = viewport(vector)
    dims = ' width="%d" height="%d"' % (size, size) if size else ""
    lines = ['<svg xmlns="http://www.w3.org/2000/svg"%s viewBox="0 0 %g %g">' % (dims, vw, vh)]
    for node in iter_paths(vector):
        d = node.get(ANDROID + "pathData")
        colour = node.get(ANDROID + "fillColor")
        if not d or colour is None:
            continue
        svg_colour, alpha = hex_rgba_to_svg(colour)
        extra = ""
        if node.get(ANDROID + "fillType") == "evenOdd":
            extra += ' fill-rule="evenOdd"'
        lines.append('  <path d="%s" fill="%s"%s%s/>' % (d, svg_colour, fmt_opacity(alpha), extra))
    lines.append("</svg>")
    return "\n".join(lines) + "\n"


def named_paths_svg(vector, names, size, transforms=None):
    """A symbolic icon from the named paths of a base vector (animated drawables)."""
    vw, vh = viewport(vector)
    lines = ['<svg xmlns="http://www.w3.org/2000/svg" width="%d" height="%d" viewBox="0 0 %g %g">' % (size, size, vw, vh)]
    for name in names:
        node = next(n for n in vector.iter("path") if n.get(ANDROID + "name") == name)
        extra = ""
        if transforms and name in transforms:
            extra = ' transform="%s"' % transforms[name]
        lines.append('  <path d="%s" fill="%s"%s/>' % (node.get(ANDROID + "pathData"), SYMBOLIC_FILL, extra))
    lines.append("</svg>")
    return "\n".join(lines) + "\n"


COMPASS_RING = "translate(12 12) scale(0.833) translate(-12 -12)"


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(text)


def main(source_dir, out_dir):
    clash = sorted({source for _, source in ACTION_ICONS} & set(DROPPED_DRAWABLES))
    if clash:
        sys.exit("dropped drawables must not be exported: " + ", ".join(clash))
    actions = os.path.join(out_dir, "scalable", "actions")
    for name, source in ACTION_ICONS:
        vector = vector_root(os.path.join(source_dir, source))
        # The seek triangle is 16dp x 20dp on a 24 viewport; its square viewBox keeps it centred at 16 px.
        write(os.path.join(actions, name + ".svg"), symbolic_svg(vector, 16))
    library = vector_root(os.path.join(source_dir, "anim_library_enter.xml"))
    write(os.path.join(actions, "kb-library-symbolic.svg"), named_paths_svg(library, ["bookmark", "angle"], 16))
    compass = vector_root(os.path.join(source_dir, "anim_browse_enter.xml"))
    write(
        os.path.join(actions, "kb-compass-symbolic.svg"),
        symbolic_svg(compass, 16, transforms={"ring": COMPASS_RING}),
    )
    logo = vector_root(os.path.join(source_dir, "ic_ani.xml"))
    write(os.path.join(out_dir, "symbolic", "apps", "io.github.anas1412.Kurobayomi-symbolic.svg"), symbolic_svg(logo, 16))
    launcher = vector_root(os.path.join(source_dir, "ic_launcher_foreground.xml"))
    write(os.path.join(out_dir, "scalable", "apps", "io.github.anas1412.Kurobayomi.svg"), colour_svg(launcher))
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("usage: export_icons.py <android drawable dir> <output theme dir>")
    sys.exit(main(sys.argv[1], sys.argv[2]))
