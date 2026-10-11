#!/usr/bin/env python3
"""Adds an `m` (U+006D) mapping to the codicon font's cmap.

GPUI's macOS text system skips every font without an `m` glyph (it measures
`m` for em widths), so Microsoft's `codicon.ttf` as published would never
load there. This maps `m` to the `add` icon's glyph; nothing else changes
(the glyphs, metrics and other tables are copied byte for byte).

Usage: patch-codicon-font.py <codicon.ttf from @vscode/codicons> <out.ttf>
"""

import struct
import sys

M = 0x6D
DONOR = 0xEA60  # `add`


def checksum(data):
    data = data + b"\0" * (-len(data) % 4)
    return sum(struct.unpack(">%dI" % (len(data) // 4), data)) & 0xFFFFFFFF


def read_mapping(cmap):
    """Every code point -> glyph mapping of the format 12 subtable."""
    _, count = struct.unpack(">HH", cmap[:4])
    for i in range(count):
        _, _, offset = struct.unpack(">HHI", cmap[4 + 8 * i : 12 + 8 * i])
        if struct.unpack(">H", cmap[offset : offset + 2])[0] == 12:
            groups = struct.unpack(">I", cmap[offset + 12 : offset + 16])[0]
            mapping = {}
            for g in range(groups):
                start, end, glyph = struct.unpack(
                    ">III", cmap[offset + 16 + 12 * g : offset + 28 + 12 * g]
                )
                for code in range(start, end + 1):
                    mapping[code] = glyph + code - start
            return mapping
    raise SystemExit("no format 12 cmap subtable")


def format4(mapping):
    codes = sorted(c for c in mapping if c <= 0xFFFF)
    segments = []  # (start, end, delta)
    for code in codes:
        delta = (mapping[code] - code) & 0xFFFF
        if segments and segments[-1][1] == code - 1 and segments[-1][2] == delta:
            segments[-1][1] = code
        else:
            segments.append([code, code, delta])
    segments.append([0xFFFF, 0xFFFF, 1])
    n = len(segments)
    search = 2 ** (n.bit_length() - 1) * 2
    body = struct.pack(
        ">HHHH", n * 2, search, (search // 2).bit_length() - 1, n * 2 - search
    )
    body += b"".join(struct.pack(">H", s[1]) for s in segments)
    body += b"\0\0"
    body += b"".join(struct.pack(">H", s[0]) for s in segments)
    body += b"".join(struct.pack(">H", s[2]) for s in segments)
    body += b"".join(struct.pack(">H", 0) for _ in segments)
    return struct.pack(">HHH", 4, 6 + len(body), 0) + body


def format12(mapping):
    groups = []
    for code in sorted(mapping):
        glyph = mapping[code]
        if groups and groups[-1][1] == code - 1 and groups[-1][2] + code - groups[-1][0] == glyph:
            groups[-1][1] = code
        else:
            groups.append([code, code, glyph])
    body = b"".join(struct.pack(">III", *g) for g in groups)
    return struct.pack(">HHIII", 12, 0, 16 + len(body), 0, len(groups)) + body


def format0(mapping):
    glyphs = bytes(mapping.get(c, 0) if mapping.get(c, 0) < 256 else 0 for c in range(256))
    return struct.pack(">HHH", 0, 262, 0) + glyphs


def build_cmap(mapping):
    subtables = [format4(mapping), format12(mapping), format0(mapping)]
    records = [(0, 3, 0), (0, 4, 1), (1, 0, 2), (3, 1, 0), (3, 10, 1)]
    header = 4 + 8 * len(records)
    offsets = []
    position = header
    for table in subtables:
        offsets.append(position)
        position += len(table)
    out = struct.pack(">HH", 0, len(records))
    out += b"".join(struct.pack(">HHI", p, e, offsets[i]) for p, e, i in records)
    return out + b"".join(subtables)


def main(source, target):
    data = open(source, "rb").read()
    sfnt, count = struct.unpack(">IH", data[:6])
    tables = {}
    for i in range(count):
        tag, _, offset, length = struct.unpack(">4sIII", data[12 + 16 * i : 28 + 16 * i])
        tables[tag] = data[offset : offset + length]
    mapping = read_mapping(tables[b"cmap"])
    if M in mapping:
        raise SystemExit("the font already maps m")
    mapping[M] = mapping[DONOR]
    tables[b"cmap"] = build_cmap(mapping)
    # head.checkSumAdjustment is recomputed below.
    head = bytearray(tables[b"head"])
    head[8:12] = b"\0\0\0\0"
    tables[b"head"] = bytes(head)

    tags = sorted(tables)
    search = 2 ** (len(tags).bit_length() - 1)
    out = struct.pack(
        ">IHHHH", sfnt, len(tags), search * 16, search.bit_length() - 1, len(tags) * 16 - search * 16
    )
    offset = 12 + 16 * len(tags)
    directory, body = b"", b""
    offsets = {}
    for tag in tags:
        table = tables[tag]
        offsets[tag] = offset + len(body)
        directory += struct.pack(">4sIII", tag, checksum(table), offsets[tag], len(table))
        body += table + b"\0" * (-len(table) % 4)
    font = bytearray(out + directory + body)
    head_offset = offsets[b"head"]
    adjustment = (0xB1B0AFBA - checksum(bytes(font))) & 0xFFFFFFFF
    font[head_offset + 8 : head_offset + 12] = struct.pack(">I", adjustment)
    open(target, "wb").write(bytes(font))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
