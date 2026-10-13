#!/usr/bin/env python3
"""Verify probe PNG codes against uncompressed RGB16 TIFFs; restore exact ICC."""
import pathlib
import struct
import sys
import zlib


def tiff(path):
    data = path.read_bytes()
    assert data[:4] == b"II*\0", "expected probe little-endian classic TIFF"
    ifd = struct.unpack_from("<I", data, 4)[0]
    tags = {}
    for i in range(struct.unpack_from("<H", data, ifd)[0]):
        entry = ifd + 2 + i * 12
        tag, kind, count = struct.unpack_from("<HHI", data, entry)
        size = {1: 1, 2: 1, 3: 2, 4: 4, 7: 1}.get(kind)
        if size is None:
            continue
        start = entry + 8 if count * size <= 4 else struct.unpack_from("<I", data, entry + 8)[0]
        value = data[start:start + count * size]
        tags[tag] = list(struct.unpack("<" + {3: "H", 4: "I"}[kind] * count, value)) if kind in (3, 4) else value
    assert tags[258] == [16, 16, 16] and tags[259] == [1] and tags[262] == [2]
    assert tags[277] == [3] and tags.get(284, [1]) == [1] and tags.get(317, [1]) == [1]
    codes = b"".join(data[offset:offset + size] for offset, size in zip(tags[273], tags[279]))
    width, height = tags[256][0], tags[257][0]
    assert len(codes) == width * height * 6
    # PNG 16-bit samples are big-endian, TIFF samples here are little-endian.
    swapped = bytearray(len(codes))
    swapped[0::2], swapped[1::2] = codes[1::2], codes[0::2]
    return width, height, bytes(swapped), tags[34675]


def chunks(data):
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    p = 8
    while p < len(data):
        size = struct.unpack_from(">I", data, p)[0]
        kind, value = data[p + 4:p + 8], data[p + 8:p + 8 + size]
        assert zlib.crc32(kind + value) == struct.unpack_from(">I", data, p + 8 + size)[0]
        yield kind, value
        p += size + 12
    assert p == len(data)


def chunk(kind, value):
    return struct.pack(">I", len(value)) + kind + value + struct.pack(">I", zlib.crc32(kind + value))


def verify(path):
    width, height, expected, profile = tiff(path.with_suffix(".tif"))
    parts = list(chunks(path.read_bytes()))
    ihdr = parts[0][1]
    assert struct.unpack(">IIBBBBB", ihdr) == (width, height, 16, 2, 0, 0, 0)
    packed = zlib.decompress(b"".join(v for k, v in parts if k == b"IDAT"))
    stride, previous, actual = width * 6, bytearray(width * 6), bytearray()
    assert len(packed) == height * (stride + 1)
    for row in range(height):
        method = packed[row * (stride + 1)]
        scan = bytearray(packed[row * (stride + 1) + 1:(row + 1) * (stride + 1)])
        assert method in range(5)
        for i in range(stride):
            left, up, corner = scan[i - 6] if i >= 6 else 0, previous[i], previous[i - 6] if i >= 6 else 0
            p = left + up - corner
            distances = [abs(p - left), abs(p - up), abs(p - corner)]
            paeth = [left, up, corner][distances.index(min(distances))]
            scan[i] = (scan[i] + [0, left, up, (left + up) // 2, paeth][method]) & 255
        actual.extend(scan)
        previous = scan
    assert actual == expected, f"PNG changed RGB16 code values: {path}"
    # ImageIO recognizes the custom profiles as named sRGB/P3 and can substitute
    # system metadata. Only after proving identical codes, restore the TIFF ICC.
    updated = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr)
    updated += chunk(b"iCCP", b"Antitone\0\0" + zlib.compress(profile))
    updated += b"".join(chunk(k, v) for k, v in parts[1:] if k not in (b"iCCP", b"sRGB", b"gAMA", b"cHRM"))
    if updated != path.read_bytes():
        path.write_bytes(updated)
    assert zlib.decompress(next(v for k, v in chunks(updated) if k == b"iCCP").split(b"\0", 1)[1][1:]) == profile


if __name__ == "__main__":
    directory = pathlib.Path(sys.argv[1])
    paths = sorted(directory.glob("*.png"))
    assert len(paths) == len(list(directory.glob("*.tif"))) > 0
    for path in paths:
        verify(path)
    print(f"Verified {len(paths)} RGB16 PNGs: exact TIFF codes and exact embedded ICC.")
