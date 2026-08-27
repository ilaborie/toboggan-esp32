#!/usr/bin/env python3
"""Compare the frames `sim-test` captured against the committed goldens.

wokwi-cli's own `compare-with` demands byte equality, which the simulated panel
does not deliver: a dependency bump that cannot touch rasterisation at all (the
docs-only embedded-graphics 0.8.2) and a simple change of screenshot delay each
moved a single isolated pixel inside a glyph. That is an artefact of Wokwi's
ILI9341 model, not of what the firmware drew, and it made the check cry wolf on
every upgrade.

So compare here instead, with a budget. A real regression is never subtle: one
wrong character in FONT_9X18 is up to 162 pixels, a moved line or a wrong colour
is thousands. Anything under the threshold is panel noise.
"""

import struct
import sys
import zlib
from pathlib import Path

# A single FONT_9X18 glyph is 9x18 = 162 pixels, so this stays well under the
# smallest change anyone would call a regression.
THRESHOLD = 50


def read_png(path):
    """Decode a PNG to (width, height, bytes_per_pixel, raw pixels)."""
    data = path.read_bytes()
    pos, idat, width, height, colour_type = 8, b"", None, None, None
    while pos < len(data):
        length = struct.unpack(">I", data[pos : pos + 4])[0]
        kind = data[pos + 4 : pos + 8]
        chunk = data[pos + 8 : pos + 8 + length]
        if kind == b"IHDR":
            width, height, _bit_depth, colour_type = struct.unpack(">IIBB", chunk[:10])
        elif kind == b"IDAT":
            idat += chunk
        pos += 12 + length

    raw = zlib.decompress(idat)
    bpp = 4 if colour_type == 6 else 3
    stride = width * bpp
    out, previous, offset = bytearray(), bytearray(stride), 0
    for _ in range(height):
        filter_type = raw[offset]
        offset += 1
        line = bytearray(raw[offset : offset + stride])
        offset += stride
        for x in range(stride):
            left = line[x - bpp] if x >= bpp else 0
            up = previous[x]
            up_left = previous[x - bpp] if x >= bpp else 0
            if filter_type == 1:
                line[x] = (line[x] + left) & 0xFF
            elif filter_type == 2:
                line[x] = (line[x] + up) & 0xFF
            elif filter_type == 3:
                line[x] = (line[x] + (left + up) // 2) & 0xFF
            elif filter_type == 4:
                pa, pb, pc = abs(up - up_left), abs(left - up_left), abs(left + up - 2 * up_left)
                nearest = left if pa <= pb and pa <= pc else (up if pb <= pc else up_left)
                line[x] = (line[x] + nearest) & 0xFF
        out += line
        previous = line
    return width, height, bpp, bytes(out)


def compare(actual_path, golden_path):
    """Return the number of differing pixels, or None if the frames are incomparable."""
    width, height, bpp, actual = read_png(actual_path)
    golden_width, golden_height, _, golden = read_png(golden_path)
    if (width, height) != (golden_width, golden_height):
        print(f"  {actual_path.name}: size {width}x{height} != golden {golden_width}x{golden_height}")
        return None

    differing = 0
    first = None
    for y in range(height):
        for x in range(width):
            at = (y * width + x) * bpp
            if actual[at : at + 3] != golden[at : at + 3]:
                differing += 1
                if first is None:
                    first = (x, y)

    total = width * height
    verdict = "ok" if differing <= THRESHOLD else "FAILED"
    detail = f" first at {first}" if first else ""
    print(f"  {actual_path.name}: {differing}/{total} pixels differ ({verdict}){detail}")
    return differing


def main():
    here = Path(__file__).parent
    goldens = sorted((here / "golden").glob("*.png"))
    if not goldens:
        print("no goldens to compare against", file=sys.stderr)
        return 1

    print(f"Comparing frames against goldens (threshold {THRESHOLD} pixels):")
    failed = False
    for golden in goldens:
        actual = here / "out" / golden.name
        if not actual.exists():
            print(f"  {golden.name}: MISSING from wokwi/out/ — the scenario never captured it")
            failed = True
            continue
        differing = compare(actual, golden)
        if differing is None or differing > THRESHOLD:
            failed = True

    if failed:
        print("\nA frame differs by more than panel noise. Inspect wokwi/out/ against")
        print("wokwi/golden/ before deciding whether the golden should be refreshed.")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
