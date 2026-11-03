"""Generates the image decoder fixtures.

Run from anywhere: python3 generate.py. Needs Pillow. The AVIF and JPEG XL stills are not generated here: they were
were produced once and are checked in. F-10 (HEIF) has no encoder in this build
and is not generated.
"""

import io
import os
import struct
import zlib

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))


def write(name, data):
    with open(os.path.join(HERE, name), "wb") as f:
        f.write(data)


def fill_rgb(w, h):
    img = Image.new("RGB", (w, h))
    px = img.load()
    for y in range(h):
        for x in range(w):
            px[x, y] = (x * 255 // max(w - 1, 1), y * 255 // max(h - 1, 1), 128)
    return img


def encode(img, fmt, **kw):
    buf = io.BytesIO()
    img.save(buf, format=fmt, **kw)
    return buf.getvalue()


def png_with_ihdr(width, height, rounds=4):
    """A small RGB PNG whose IHDR size is replaced, with the chunk CRC recomputed so the header stays valid."""
    base = encode(Image.new("RGB", (rounds, rounds), (10, 20, 30)), "PNG")
    ihdr_len = struct.unpack(">I", base[8:12])[0]
    assert base[12:16] == b"IHDR" and ihdr_len == 13
    body = bytearray(base[16:16 + 13])
    body[0:4] = struct.pack(">I", width)
    body[4:8] = struct.pack(">I", height)
    crc = zlib.crc32(b"IHDR" + bytes(body)) & 0xFFFFFFFF
    return base[:8] + struct.pack(">I", 13) + b"IHDR" + bytes(body) + struct.pack(">I", crc) + base[8 + 8 + 13 + 4:]


def exif_orientation_jpeg(orientation, w, h):
    exif = Image.Exif()
    exif[0x0112] = orientation
    return encode(fill_rgb(w, h), "JPEG", quality=90, exif=exif.tobytes())


def main():
    jpeg = encode(fill_rgb(320, 480), "JPEG", quality=85)
    write("jpeg_baseline_320x480.jpg", jpeg)  # F-01
    # F-02: stored 480 x 320, orientation 6 (rotate 90 clockwise to display), so it displays 320 x 480.
    write("jpeg_exif_rot90.jpg", exif_orientation_jpeg(6, 480, 320))

    rgba = Image.new("RGBA", (100, 100))
    px = rgba.load()
    for y in range(100):
        for x in range(100):
            px[x, y] = (200, 60, 60, x * 255 // 99)
    write("png_rgba_100x100.png", encode(rgba, "PNG"))  # F-03

    static = Image.new("P", (64, 64), 0)
    static.putpalette([0, 0, 0, 255, 255, 255, 200, 40, 40] + [0] * (768 - 9))
    static.paste(2, (0, 0, 32, 64))
    static.paste(1, (32, 0, 64, 64))
    write("gif_static_64.gif", encode(static, "GIF"))  # F-04

    frames = []
    for color in [(255, 0, 0), (0, 255, 0), (0, 0, 255)]:
        frames.append(Image.new("RGB", (64, 64), color))
    buf = io.BytesIO()
    frames[0].save(buf, format="GIF", save_all=True, append_images=frames[1:], duration=100, loop=0)
    write("gif_animated_3f.gif", buf.getvalue())  # F-05

    webp_buf = io.BytesIO()
    frames[0].save(webp_buf, format="WEBP", save_all=True, append_images=frames[1:], duration=100, loop=0, lossless=True)
    write("webp_animated_3f.webp", webp_buf.getvalue())  # animated WebP, same three frames as F-05

    write("webp_lossy_50.webp", encode(fill_rgb(120, 90), "WEBP", quality=50))  # F-06
    write("bmp_24bit.bmp", encode(fill_rgb(96, 64), "BMP"))  # F-07

    write("truncated_jpeg.jpg", jpeg[:1024])  # F-11
    write("zero_bytes.bin", b"")  # F-12
    write("text_named.jpg", b"This is plain text, not a picture.\n" * 4)  # F-13
    write("zero_dimension.png", png_with_ihdr(0, 4))  # F-14
    write("huge_header.png", png_with_ihdr(30000, 30000))  # F-15
    write("uppercase_ext.JPG", jpeg)  # F-16
    write("jpeg_as_jpeg_ext.jpeg", jpeg)  # F-17


if __name__ == "__main__":
    main()
