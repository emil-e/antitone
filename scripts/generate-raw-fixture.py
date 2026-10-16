#!/usr/bin/env python3
"""Maintain authored Bayer DNG fixtures; no dependencies, downloads or photographs.

Writes tests/fixtures/raw/*.dng, or --check fails on byte-for-byte recipe drift.
This encodes synthetic sensor data, not a camera/film calibration or photographic oracle.
"""

import argparse
from pathlib import Path
import struct

WIDTH, HEIGHT = 256, 192
BLACK, WHITE = 64, 4095
# Camera RGB values above black, in 64x64 tiles, left-to-right then top-to-bottom.
PATCHES = (
    (3000, 3000, 3000),
    (128, 128, 128),
    (1000, 1000, 1000),
    (2400, 1200, 600),
    (1200, 2400, 600),
    (600, 1200, 2400),
    (3200, 3200, 3200),
    (600, 1000, 1800),
    (256, 256, 256),
    (512, 512, 512),
    (1024, 1024, 1024),
    (2048, 2048, 2048),
)


def field(tag, kind, values):
    if kind == 2:  # ASCII, including its terminator.
        data = values.encode("ascii") + b"\0"
        count = len(data)
    elif kind in (5, 10):  # Unsigned/signed rational pairs.
        data = b"".join(struct.pack("<II" if kind == 5 else "<ii", *v) for v in values)
        count = len(values)
    else:
        code = {1: "B", 3: "H", 4: "I"}[kind]
        data = struct.pack("<" + code * len(values), *values)
        count = len(values)
    return tag, kind, count, data


def blend_axis(position, tiles):
    tile, offset = divmod(position, 64)
    if offset < 16 and tile > 0:
        return tile - 1, tile, offset + 16
    if offset >= 48 and tile < tiles - 1:
        return tile, tile + 1, offset - 48
    return tile, tile, 0


def patch_signal(x, y, width, height, channel):
    columns, rows = width // 64, height // 64
    x0, x1, wx = blend_axis(x, columns)
    y0, y1, wy = blend_axis(y, rows)

    def patch(xx, yy):
        return PATCHES[(yy * columns + xx) % len(PATCHES)][channel]

    value = (32 - wy) * ((32 - wx) * patch(x0, y0) + wx * patch(x1, y0)) + wy * (
        (32 - wx) * patch(x0, y1) + wx * patch(x1, y1)
    )
    return (value + 512) // 1024


def illumination(x, y, width, height, channel):
    # Channel-dependent affine light field at pixel centres, using exact rational arithmetic.
    denominator = 1000 * width * height
    numerator = (
        denominator
        + (80, -50, 30)[channel] * (2 * x + 1 - width) * height
        + (-40, 60, 80)[channel] * (2 * y + 1 - height) * width
    )
    return numerator, denominator


def roll_signal(x, y, frame):
    # Neutral negative ramp in a 320x320 picture, with a smooth join to broad film base.
    xx = min(319, max(0, x - 96))
    yy = min(319, max(0, y - 96))
    picture = 256 + (1792 * (xx + 2 * yy) + 478) // 957
    if frame == 1:
        picture = (3 * picture + 2) // 4
    edge = min(32, max(0, 96 - x, x - 415, 96 - y, y - 415))
    return ((32 - edge) * picture + edge * 3000 + 16) // 32


def preview_jpeg():
    # Baseline 16x8 YCbCr JPEG: two neutral 8x8 blocks, values 64 and 192.
    # Constant blocks have only a DC coefficient; no general-purpose encoder is needed.
    def segment(marker, payload):
        return bytes((255, marker)) + struct.pack(">H", len(payload) + 2) + payload

    dqt = segment(0xDB, bytes((0,)) + bytes((8,)) * 64)
    dc = bytes((0,)) + bytes((0, 0, 0, 12)) + bytes(12) + bytes(range(12))
    ac = bytes((16, 1)) + bytes(15) + bytes((0,))
    dht = segment(0xC4, dc + ac)
    sof = segment(0xC0, struct.pack(">BHHB", 8, 8, 16, 3) + bytes((1, 17, 0, 2, 17, 0, 3, 17, 0)))
    sos = segment(0xDA, bytes((3, 1, 0, 2, 0, 3, 0, 0, 63, 0)))
    bits = ""
    previous = [0, 0, 0]
    for value in (64, 192):
        for channel, coefficient in enumerate((value - 128, 0, 0)):
            difference = coefficient - previous[channel]
            previous[channel] = coefficient
            size = abs(difference).bit_length()
            bits += f"{size:04b}"
            if size:
                amplitude = difference if difference >= 0 else difference + (1 << size) - 1
                bits += f"{amplitude:0{size}b}"
            bits += "0"  # AC end-of-block.
    bits += "1" * (-len(bits) % 8)
    entropy = bytes(int(bits[i : i + 8], 2) for i in range(0, len(bits), 8)).replace(
        b"\xff", b"\xff\x00"
    )
    jfif = segment(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0")
    return b"\xff\xd8" + jfif + dqt + dht + sof + sos + entropy + b"\xff\xd9"


def fixture(width=WIDTH, height=HEIGHT, mode="patches"):
    # ColorMatrix1 is XYZ -> camera RGB at D65, not camera -> XYZ.
    xyz_to_camera = (
        32404542,
        -15371385,
        -4985314,
        -9692660,
        18760108,
        415560,
        556434,
        -2040259,
        10572252,
    )
    fields = sorted(
        [
            field(254, 4, (0,)),  # NewSubfileType: full-resolution raw
            field(256, 4, (width,)),
            field(257, 4, (height,)),
            field(258, 3, (16,)),
            field(259, 3, (1,)),  # Uncompressed unsigned RGB CFA
            field(262, 3, (32803,)),
            field(271, 2, "Antitone"),
            field(272, 2, "Authored Bayer patches"),
            field(273, 4, (0,)),  # Strip offset filled after metadata
            field(274, 3, (1,)),
            field(277, 3, (1,)),
            field(278, 4, (height,)),
            field(279, 4, (width * height * 2,)),
            field(284, 3, (1,)),
            field(339, 3, (1,)),
            field(305, 2, "Antitone authored RAW fixture"),
            field(33421, 3, (2, 2)),
            field(33422, 1, (0, 1, 1, 2)),
            field(50706, 1, (1, 1, 0, 0)),
            field(50707, 1, (1, 1, 0, 0)),
            field(50708, 2, "Antitone Authored Bayer patches"),
            field(50710, 1, (0, 1, 2)),
            field(50711, 3, (1,)),
            # Omit optional BlackLevelRepeatDim (default 1x1). LibRaw retains an explicit
            # repeat dimension in cblack even for scalar black; the bridge rejects that pattern.
            field(50714, 5, ((BLACK, 1),)),
            field(50717, 4, (WHITE,)),
            field(50718, 5, ((1, 1), (1, 1))),  # DefaultScale
            field(50719, 4, (8, 8)),
            field(50720, 4, (width - 16, height - 16)),
            field(50721, 10, tuple((v, 10000000) for v in xyz_to_camera)),
            field(50728, 5, ((1, 1), (1, 1), (1, 1))),
            field(50778, 3, (21,)),  # CalibrationIlluminant1: D65
            field(50829, 4, (0, 0, height, width)),  # ActiveArea: top,left,bottom,right
        ]
    )
    if mode == "preview":
        fields.append(field(34665, 4, (0,)))  # Exif IFD offset filled below
        fields.sort()
    metadata_end = 8 + 2 + 12 * len(fields) + 4
    entries, extra = bytearray(), bytearray()
    for tag, kind, count, data in fields:
        if len(data) <= 4:
            value = data.ljust(4, b"\0")
        else:
            value = struct.pack("<I", metadata_end + len(extra))
            extra.extend(data)
            if len(extra) % 2:
                extra.append(0)
        entries.extend(struct.pack("<HHI", tag, kind, count) + value)
    output = bytearray(b"II" + struct.pack("<HIH", 42, 8, len(fields)))
    output.extend(entries + struct.pack("<I", 0) + extra)
    strip_index = next(i for i, entry in enumerate(fields) if entry[0] == 273)
    struct.pack_into("<I", output, 8 + 2 + strip_index * 12 + 8, len(output))
    for y in range(height):
        for x in range(width):
            # Smooth 32-pixel joins avoid artificial AHD undershoot at hard colour steps.
            channel = ((0, 1), (1, 2))[y % 2][x % 2]
            if mode.startswith("roll-"):
                value = roll_signal(x, y, int(mode[-1]))
            else:
                value = 2400 if mode == "bare-light" else patch_signal(x, y, width, height, channel)
            if mode in ("bare-light", "illuminated-patches"):
                numerator, denominator = illumination(x, y, width, height, channel)
                value = (value * numerator + denominator // 2) // denominator
            output.extend(struct.pack("<H", BLACK + value))
    if mode == "preview":
        exif_offset = len(output)
        output.extend(
            struct.pack("<HHHI", 1, 40961, 3, 1) + struct.pack("<I", 1) + struct.pack("<I", 0)
        )
        exif_index = next(i for i, entry in enumerate(fields) if entry[0] == 34665)
        struct.pack_into("<I", output, 8 + 2 + exif_index * 12 + 8, exif_offset)
        thumbnail_offset = len(output)
        struct.pack_into("<I", output, 8 + 2 + 12 * len(fields), thumbnail_offset)
        jpeg = preview_jpeg()
        thumbnail_fields = sorted(
            [
                field(254, 4, (1,)),
                field(256, 4, (16,)),
                field(257, 4, (8,)),
                field(258, 3, (8, 8, 8)),
                field(259, 3, (7,)),
                field(262, 3, (6,)),
                field(274, 3, (6,)),
                field(277, 3, (3,)),
                field(513, 4, (0,)),  # JPEG offset filled after this IFD
                field(514, 4, (len(jpeg),)),
                field(530, 3, (1, 1)),
            ]
        )
        output.extend(struct.pack("<H", len(thumbnail_fields)))
        extra = bytearray()
        extra_offset = thumbnail_offset + 2 + 12 * len(thumbnail_fields) + 4
        for tag, kind, count, data in thumbnail_fields:
            if len(data) > 4:
                value = struct.pack("<I", extra_offset + len(extra))
                extra.extend(data)
            else:
                value = data.ljust(4, b"\0")
            output.extend(struct.pack("<HHI", tag, kind, count) + value)
        output.extend(struct.pack("<I", 0) + extra)
        jpeg_index = next(i for i, entry in enumerate(thumbnail_fields) if entry[0] == 513)
        struct.pack_into("<I", output, thumbnail_offset + 2 + jpeg_index * 12 + 8, len(output))
        output.extend(jpeg)
    return bytes(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Compare without writing")
    args = parser.parse_args()
    directory = Path(__file__).resolve().parent.parent / "tests/fixtures/raw"
    fixtures = {
        "patches.dng": fixture(),
        "bare-light.dng": fixture(384, 320, "bare-light"),
        "illuminated-patches.dng": fixture(384, 320, "illuminated-patches"),
        "roll-0.dng": fixture(512, 512, "roll-0"),
        "roll-1.dng": fixture(512, 512, "roll-1"),
        "preview.dng": fixture(mode="preview"),
    }
    for name, data in fixtures.items():
        path = directory / name
        if args.check:
            if not path.exists() or path.read_bytes() != data:
                parser.exit(1, f"{path}: missing or differs from the authored recipe\n")
        else:
            directory.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        print(f"{path.name}: {len(data)} bytes; {'matches' if args.check else 'written'}")


if __name__ == "__main__":
    main()
