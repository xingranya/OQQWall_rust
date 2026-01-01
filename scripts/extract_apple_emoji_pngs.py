#!/usr/bin/env python3
import argparse
import json
import pathlib
import shutil
import struct
import sys

PNG_SIG = b"\x89PNG\r\n\x1a\n"


def read_u16(data, offset):
    return struct.unpack_from(">H", data, offset)[0]


def read_u32(data, offset):
    return struct.unpack_from(">I", data, offset)[0]


def read_i8(value):
    return value - 256 if value > 127 else value


def parse_table_directory(font_bytes):
    num_tables = read_u16(font_bytes, 4)
    tables = {}
    offset = 12
    for _ in range(num_tables):
        tag = font_bytes[offset : offset + 4].decode("ascii")
        table_offset = read_u32(font_bytes, offset + 8)
        length = read_u32(font_bytes, offset + 12)
        tables[tag] = (table_offset, length)
        offset += 16
    return tables


def parse_cblc_sizes(cblc_bytes):
    if len(cblc_bytes) < 8:
        return []
    num_sizes = read_u32(cblc_bytes, 4)
    sizes = []
    for i in range(num_sizes):
        base = 8 + i * 48
        if base + 48 > len(cblc_bytes):
            break
        sizes.append(
            {
                "index_subtable_array_offset": read_u32(cblc_bytes, base),
                "index_subtable_array_size": read_u32(cblc_bytes, base + 4),
                "num_subtables": read_u32(cblc_bytes, base + 8),
                "start_glyph": read_u16(cblc_bytes, base + 40),
                "end_glyph": read_u16(cblc_bytes, base + 42),
                "ppem_x": cblc_bytes[base + 44],
                "ppem_y": cblc_bytes[base + 45],
                "bit_depth": cblc_bytes[base + 46],
                "flags": struct.unpack_from(">b", cblc_bytes, base + 47)[0],
            }
        )
    return sizes


def resolve_subtable_base(cblc_bytes, entries, array_start):
    for base in (array_start, 0):
        for entry in entries[: min(3, len(entries))]:
            sub_offset = base + entry["additional_offset"]
            if sub_offset + 8 > len(cblc_bytes):
                continue
            index_format = read_u16(cblc_bytes, sub_offset)
            image_format = read_u16(cblc_bytes, sub_offset + 2)
            if index_format in (1, 2, 3, 4, 5) and image_format in (17, 18):
                return base
    return array_start


def parse_png_payload(data, offset):
    if offset + 4 <= len(data):
        png_len = read_u32(data, offset)
        png_start = offset + 4
        png_end = png_start + png_len
        if png_end <= len(data):
            payload = data[png_start:png_end]
            if payload.startswith(PNG_SIG):
                return payload
    sig_index = data.find(PNG_SIG, offset)
    if sig_index != -1:
        return data[sig_index:]
    return None


def parse_format_17(data):
    if len(data) < 9:
        return None
    height = data[0]
    width = data[1]
    bearing_x = read_i8(data[2])
    bearing_y = read_i8(data[3])
    png = parse_png_payload(data, 5)
    if png is None:
        return None
    return width, height, bearing_x, bearing_y, png


def parse_format_18(data):
    if len(data) < 12:
        return None
    height = data[0]
    width = data[1]
    bearing_x = read_i8(data[2])
    bearing_y = read_i8(data[3])
    png = parse_png_payload(data, 8)
    if png is None:
        return None
    return width, height, bearing_x, bearing_y, png


def extract_glyphs(cblc_bytes, cbdt_bytes, size_table, limit=None):
    array_start = size_table["index_subtable_array_offset"]
    array_size = size_table["index_subtable_array_size"]
    num_subtables = size_table["num_subtables"]
    if array_start + array_size > len(cblc_bytes):
        raise ValueError("index subtable array out of range")
    entries = []
    for i in range(num_subtables):
        entry_offset = array_start + i * 8
        if entry_offset + 8 > len(cblc_bytes):
            break
        entries.append(
            {
                "first_glyph": read_u16(cblc_bytes, entry_offset),
                "last_glyph": read_u16(cblc_bytes, entry_offset + 2),
                "additional_offset": read_u32(cblc_bytes, entry_offset + 4),
            }
        )
    base = resolve_subtable_base(cblc_bytes, entries, array_start)
    glyphs = {}
    for entry in entries:
        sub_offset = base + entry["additional_offset"]
        if sub_offset + 8 > len(cblc_bytes):
            continue
        index_format = read_u16(cblc_bytes, sub_offset)
        image_format = read_u16(cblc_bytes, sub_offset + 2)
        image_data_offset = read_u32(cblc_bytes, sub_offset + 4)
        if index_format != 1 or image_format not in (17, 18):
            continue
        num_glyphs = entry["last_glyph"] - entry["first_glyph"] + 1
        offset_array_offset = sub_offset + 8
        offsets_end = offset_array_offset + 4 * (num_glyphs + 1)
        if offsets_end > len(cblc_bytes):
            continue
        offsets = struct.unpack_from(
            ">" + "I" * (num_glyphs + 1), cblc_bytes, offset_array_offset
        )
        for i in range(num_glyphs):
            if limit is not None and len(glyphs) >= limit:
                return glyphs
            start = image_data_offset + offsets[i]
            end = image_data_offset + offsets[i + 1]
            if start >= end or end > len(cbdt_bytes):
                continue
            data = cbdt_bytes[start:end]
            if image_format == 17:
                parsed = parse_format_17(data)
            else:
                parsed = parse_format_18(data)
            if parsed is None:
                continue
            width, height, bearing_x, bearing_y, png = parsed
            glyph_id = entry["first_glyph"] + i
            glyphs[glyph_id] = {
                "width": width,
                "height": height,
                "bearing_x": bearing_x,
                "bearing_y": bearing_y,
                "png": png,
            }
    return glyphs


def main():
    repo_root = pathlib.Path(__file__).resolve().parents[1]
    default_font = repo_root / "res/fonts/AppleColorEmoji.ttf"
    default_out = repo_root / "res/emoji_png/apple_color_emoji"

    parser = argparse.ArgumentParser(
        description="Extract AppleColorEmoji CBDT PNGs into res/emoji_png/apple_color_emoji."
    )
    parser.add_argument("--input", default=str(default_font), help="Path to AppleColorEmoji.ttf.")
    parser.add_argument(
        "--output-dir", default=str(default_out), help="Output directory for PNGs."
    )
    parser.add_argument(
        "--ppem",
        type=int,
        default=None,
        help="Override strike size (ppem) to extract.",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=None,
        help="Only extract the first N glyphs (debug).",
    )
    parser.add_argument(
        "--clean", action="store_true", help="Delete the output directory before writing."
    )
    args = parser.parse_args()

    font_path = pathlib.Path(args.input)
    if not font_path.exists():
        print(f"font not found: {font_path}", file=sys.stderr)
        return 1

    font_bytes = font_path.read_bytes()
    tables = parse_table_directory(font_bytes)
    if "CBLC" not in tables or "CBDT" not in tables:
        print("missing CBLC/CBDT tables in font", file=sys.stderr)
        return 1

    cblc_offset, cblc_length = tables["CBLC"]
    cbdt_offset, cbdt_length = tables["CBDT"]
    cblc_bytes = font_bytes[cblc_offset : cblc_offset + cblc_length]
    cbdt_bytes = font_bytes[cbdt_offset : cbdt_offset + cbdt_length]

    size_tables = parse_cblc_sizes(cblc_bytes)
    if not size_tables:
        print("no CBLC size tables found", file=sys.stderr)
        return 1

    if args.ppem is None:
        size_table = max(size_tables, key=lambda item: item["ppem_x"])
    else:
        matches = [item for item in size_tables if item["ppem_x"] == args.ppem]
        if not matches:
            available = ", ".join(str(item["ppem_x"]) for item in size_tables)
            print(f"ppem {args.ppem} not found; available: {available}", file=sys.stderr)
            return 1
        size_table = matches[0]

    output_dir = pathlib.Path(args.output_dir)
    if args.clean and output_dir.exists():
        shutil.rmtree(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    glyphs = extract_glyphs(
        cblc_bytes, cbdt_bytes, size_table, limit=args.limit
    )
    if not glyphs:
        print("no glyphs extracted", file=sys.stderr)
        return 1

    records = []
    for glyph_id in sorted(glyphs.keys()):
        info = glyphs[glyph_id]
        png_path = output_dir / f"gid_{glyph_id:04x}.png"
        png_path.write_bytes(info["png"])
        records.append(
            {
                "glyph_id": glyph_id,
                "width": info["width"],
                "height": info["height"],
                "bearing_x": info["bearing_x"],
                "bearing_y": info["bearing_y"],
            }
        )

    metadata = {
        "strike_ppem": size_table["ppem_x"],
        "glyphs": records,
    }
    metadata_path = output_dir / "metadata.json"
    metadata_path.write_text(json.dumps(metadata, ensure_ascii=True, separators=(",", ":")))

    print(
        f"extracted {len(records)} glyphs at ppem={size_table['ppem_x']} -> {output_dir}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
