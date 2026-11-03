#!/usr/bin/env python3
"""Regenerates the committed manga download fixtures (T-23a-036).

Run once from this folder: `python3 make_fixtures.py` (needs Pillow). The outputs are committed, and no test writes a
binary fixture at run time. It is Python, not Rust, because nothing here is compiled.
"""
import os
import sqlite3
import zipfile

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))


def path(*parts):
    p = os.path.join(HERE, *parts)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    return p


def gradient(w, h):
    img = Image.new("RGB", (w, h))
    img.putdata([((x * 255) // max(w - 1, 1), (y * 255) // max(h - 1, 1), 128) for y in range(h) for x in range(w)])
    return img


# img/: 8x8 pages in each format, a 2-frame gif, a tall page, and a JPEG with a valid header and garbage after it.
tiny = gradient(8, 8)
tiny.save(path("img", "tiny.jpg"), "JPEG", quality=75)
tiny.save(path("img", "tiny.png"), "PNG")
tiny.save(path("img", "tiny.webp"), "WEBP")
tiny.convert("P").save(path("img", "tiny.gif"), "GIF")
frames = [Image.new("RGB", (16, 16), (255, 0, 0)), Image.new("RGB", (16, 16), (0, 0, 255))]
frames[0].save(path("img", "animated.gif"), "GIF", save_all=True, append_images=frames[1:], duration=100, loop=0)
gradient(800, 4000).save(path("img", "tall_800x4000.png"), "PNG")
with open(path("img", "tiny.jpg"), "rb") as f:
    head = f.read(32)
with open(path("img", "broken.jpg"), "wb") as f:
    f.write(head + bytes((i * 37 + 11) % 256 for i in range(64)))

# index/tree/: MangaDex (EN), mangadex (en), Unknown (XX). Chapter folders hold one page, `.cbz` files are real archives.
tree = path("index", "tree", "x")
tree = os.path.dirname(tree)
page = open(path("img", "tiny.jpg"), "rb").read()


def chapter_dir(*parts):
    with open(path("index", "tree", *parts, "001.jpg"), "wb") as f:
        f.write(page)


def chapter_cbz(*parts):
    with zipfile.ZipFile(path("index", "tree", *parts), "w", zipfile.ZIP_STORED) as z:
        z.writestr("001.jpg", page)


chapter_dir("MangaDex (EN)", "Berserk", "Chapter 1")
chapter_dir("MangaDex (EN)", "Berserk", "Chapter 2")
chapter_cbz("MangaDex (EN)", "Berserk", "Chapter 3.cbz")
chapter_dir("mangadex (en)", "Vagabond", "Chapter 9")
chapter_cbz("Unknown (XX)", "Other", "Chapter 1.cbz")

# index/corrupt.bin: 16 fixed bytes that are not a snapshot.
with open(path("index", "corrupt.bin"), "wb") as f:
    f.write(bytes([0x13, 0x37, 0x42, 0x99, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C]))

# db/queue_v1.sql: `entries` and `items` (the columns the queue tables reference), then the unified download tables.
queue_sql = open(os.path.join(HERE, "..", "..", "kuro-db", "sql", "unified", "downloads.sql")).read()
with open(path("db", "queue_v1.sql"), "w") as f:
    f.write(
        "-- Fixture for TST-42 to TST-46: the parent tables of the unified schema reduced to their keys, then the download DDL.\n"
        "CREATE TABLE entries(_id INTEGER PRIMARY KEY, kind TEXT NOT NULL, source INTEGER NOT NULL, url TEXT NOT NULL,"
        " title TEXT NOT NULL, UNIQUE (_id, kind));\n"
        "CREATE TABLE items(_id INTEGER PRIMARY KEY, entry_id INTEGER NOT NULL REFERENCES entries(_id) ON DELETE CASCADE,"
        " url TEXT NOT NULL, name TEXT NOT NULL);\n"
        "INSERT INTO entries VALUES (1, 'manga', 1, '/manga/1', 'Berserk');\n"
        "INSERT INTO items VALUES (10, 1, '/chapter/10', 'Chapter 10'), (11, 1, '/chapter/11', 'Chapter 11'),"
        " (12, 1, '/chapter/12', 'Chapter 12');\n" + queue_sql
    )
sqlite3.connect(":memory:").executescript(open(path("db", "queue_v1.sql")).read())  # parses
