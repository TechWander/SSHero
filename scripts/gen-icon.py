#!/usr/bin/env python3
"""Generate a transparent application icon (terminal motif) without ImageMagick/PIL.

Writes:
  apps/electron/build/icon.png      512x512 RGBA  (Linux/Windows/macOS source)
  apps/electron/build/icon512.png   512x512 RGBA  (kept for reference)
  apps/electron/build/icon.ico      multi-size, PNG-compressed entries (transparency preserved)

The corners/area outside the rounded square are fully transparent (alpha 0),
which is what removes the "white tile background" around the task-bar icon.
Pure stdlib: zlib for PNG/ICO image data, struct for the containers.
"""
import struct, zlib, os

OUT = os.path.join(os.path.dirname(__file__), "..", "apps", "electron", "build")
OUT = os.path.abspath(OUT)
os.makedirs(OUT, exist_ok=True)

# ---------- tiny 2D drawing helpers on an RGBA float buffer ----------
SZ = 512
# image as list of [r,g,b,a] floats in 0..1, row-major
px = [[(0.0, 0.0, 0.0, 0.0) for _ in range(SZ)] for _ in range(SZ)]


def blend(x, y, color, alpha):
    """over-composite rgba tuple (0..255,0..255,0..255,0..255) at pixel."""
    if x < 0 or y < 0 or x >= SZ or y >= SZ:
        return
    sa = (color[3] / 255.0) * alpha
    if sa <= 0:
        return
    dr, dg, db, da = px[y][x]
    sr, sg, sb = color[0] / 255.0, color[1] / 255.0, color[2] / 255.0
    a = sa + da * (1 - sa)
    if a == 0:
        px[y][x] = (0.0, 0.0, 0.0, 0.0)
        return
    r = (sr * sa + dr * da * (1 - sa)) / a
    g = (sg * sa + dg * da * (1 - sa)) / a
    b = (sb * sa + db * da * (1 - sa)) / a
    px[y][x] = (r, g, b, a)


def rounded_rect(x0, y0, x1, y1, rad, color):
    """filled rounded rectangle with 1px-ish antialiased edge (supersample via alpha)."""
    for y in range(int(y0), int(y1)):
        for x in range(int(x0), int(x1)):
            # distance to nearest corner for radius
            dx = rad - (x - x0) if x < x0 + rad else (x - (x1 - 1)) - rad if x > x1 - 1 - rad else -1
            dy = rad - (y - y0) if y < y0 + rad else (y - (y1 - 1)) - rad if y > y1 - 1 - rad else -1
            if dx >= 0 and dy >= 0:
                d = ((dx) ** 2 + (dy) ** 2) ** 0.5
                if d > rad:
                    cov = max(0.0, 1.0 - (d - rad))
                    if cov <= 0:
                        continue
                    blend(x, y, color, cov)
                    continue
            blend(x, y, color, 1.0)


def line(x0, y0, x1, y1, width, color):
    """thick line via distance to segment."""
    x0f, y0f, x1f, y1f = float(x0), float(y0), float(x1), float(y1)
    vx, vy = x1f - x0f, y1f - y0f
    L2 = vx * vx + vy * vy or 1.0
    half = width / 2.0
    lo = int(min(x0, x1) - half - 1)
    hi = int(max(x0, x1) + half + 2)
    tlo = int(min(y0, y1) - half - 1)
    thi = int(max(y0, y1) + half + 2)
    for y in range(tlo, thi):
        for x in range(lo, hi):
            t = ((x - x0f) * vx + (y - y0f) * vy) / L2
            t = 0.0 if t < 0 else 1.0 if t > 1 else t
            cx, cy = x0f + t * vx, y0f + t * vy
            d = ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5
            cov = max(0.0, min(1.0, half + 0.5 - d))
            if cov > 0:
                blend(x, y, color, cov)


def fill_rect(x0, y0, x1, y1, color):
    for y in range(int(y0), int(y1)):
        for x in range(int(x0), int(x1)):
            blend(x, y, color, 1.0)


# ---------- compose the icon ----------
# background rounded square: dark slate vertical gradient
G0 = (27, 36, 48, 255)   # top  #1b2430
G1 = (13, 17, 23, 255)   # bottom #0d1117
RAD = int(SZ * 0.20)
M = int(SZ * 0.06)       # outer margin -> transparent around the tile
for y in range(M, SZ - M):
    t = (y - M) / float(SZ - 2 * M)
    r = G0[0] + (G1[0] - G0[0]) * t
    g = G0[1] + (G1[1] - G0[1]) * t
    b = G0[2] + (G1[2] - G0[2]) * t
    for x in range(M, SZ - M):
        dx = RAD - (x - M) if x < M + RAD else (x - (SZ - M - 1)) - RAD if x > SZ - M - 1 - RAD else -1
        dy = RAD - (y - M) if y < M + RAD else (y - (SZ - M - 1)) - RAD if y > SZ - M - 1 - RAD else -1
        if dx >= 0 and dy >= 0 and (dx * dx + dy * dy) ** 0.5 > RAD:
            continue
        px[y][x] = (r / 255.0, g / 255.0, b / 255.0, 1.0)
# rounded-corner AA pass (erase overhangs softly)
GREEN = (63, 191, 111, 255)
CYAN = (90, 170, 255, 255)
SCREEN = (8, 12, 18, 255)
# monitor screen bezel
sx0, sy0, sx1, sy1 = int(SZ * 0.16), int(SZ * 0.22), int(SZ * 0.84), int(SZ * 0.66)
rounded_rect(sx0, sy0, sx1, sy1, int(SZ * 0.05), (63, 191, 111, 210))
fill_rect(sx0 + 8, sy0 + 8, sx1 - 8, sy1 - 8, SCREEN)
# window dots (title bar)
for i, col in enumerate(((226, 109, 109, 255), (230, 190, 90, 255), GREEN)):
    fill_rect(sx0 + 22 + i * 22, sy0 + 22, sx0 + 34 + i * 22, sy0 + 34, col)
# terminal prompt ">" chevron in green
py0 = int((sy0 + sy1) / 2) + 18
line(sx0 + 30, py0 - 26, sx0 + 58, py0, 9, GREEN)
line(sx0 + 58, py0, sx0 + 30, py0 + 26, 9, GREEN)
# cursor block
fill_rect(sx0 + 74, py0 - 22, sx0 + 104, py0 + 12, (90, 170, 255, 235))
# a couple of "output" lines
fill_rect(sx0 + 120, py0 - 20, sx1 - 34, py0 - 12, (120, 132, 148, 150))
fill_rect(sx0 + 74, py0 + 40, sx1 - 70, py0 + 48, (120, 132, 148, 110))
# folder/transfer arrow into the dock below the monitor (nod to file manager)
ax0, ax1 = int(SZ * 0.5), int(SZ * 0.5)
line(ax0, int(SZ * 0.70), ax0, int(SZ * 0.86), 10, CYAN)         # shaft
line(ax0 - 22, int(SZ * 0.80), ax0, int(SZ * 0.88), 10, CYAN)    # head-left
line(ax0 + 22, int(SZ * 0.80), ax0, int(SZ * 0.88), 10, CYAN)    # head-right
# two file chips flanking the arrow
fill_rect(int(SZ * 0.28), int(SZ * 0.78), int(SZ * 0.40), int(SZ * 0.88), (140, 152, 170, 120))
fill_rect(int(SZ * 0.60), int(SZ * 0.78), int(SZ * 0.72), int(SZ * 0.88), (140, 152, 170, 120))


# ---------- PNG encoder (RGBA, 8-bit, filter 0) ----------
def downscale(target):
    """area-average box downscale from the 512 buffer to target x target."""
    if target == SZ:
        return [[px[y][x] for x in range(SZ)] for y in range(SZ)]
    fac = SZ / float(target)
    out = []
    for ty in range(target):
        row = []
        y0 = int(ty * fac); y1 = min(SZ, int((ty + 1) * fac)) or (y0 + 1)
        for tx in range(target):
            x0 = int(tx * fac); x1 = min(SZ, int((tx + 1) * fac)) or (x0 + 1)
            ar = ag = ab = aa = n = 0.0
            for yy in range(y0, y1):
                for xx in range(x0, x1):
                    r, g, b, a = px[yy][xx]
                    ar += r; ag += g; ab += b; aa += a; n += 1
            # premultiply-aware: divide color by alpha so transparent pixels don't darken
            if aa > 0:
                r, g, b = ar / aa, ag / aa, ab / aa
            else:
                r = g = b = 0
            a = aa / n
            row.append((r, g, b, a))
        out.append(row)
    return out


def png_bytes(img, size):
    raw = bytearray()
    for y in range(size):
        raw.append(0)  # filter: none
        for x in range(size):
            r, g, b, a = img[y][x]
            raw.append(max(0, min(255, int(round(r * 255)))))
            raw.append(max(0, min(255, int(round(g * 255)))))
            raw.append(max(0, min(255, int(round(b * 255)))))
            raw.append(max(0, min(255, int(round(a * 255)))))
    comp = zlib.compress(bytes(raw), 9)

    def chunk(tag, data):
        c = tag + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)  # 8-bit RGBA
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", comp) + chunk(b"IEND", b""))


# write 512 png(s)
big = downscale(SZ)
with open(os.path.join(OUT, "icon.png"), "wb") as f:
    f.write(png_bytes(big, SZ))
with open(os.path.join(OUT, "icon512.png"), "wb") as f:
    f.write(png_bytes(big, SZ))

# build ICO with PNG-compressed entries (Vista+ / modern Linux both honor PNG-in-ICO)
sizes = [16, 32, 48, 64, 128, 256]
pngs = [(s, png_bytes(downscale(s), s)) for s in sizes]
# ICONDIR
ico = struct.pack("<HHH", 0, 1, len(pngs))
offset = 6 + 16 * len(pngs)
images = b""
for s, data in pngs:
    w = 0 if s >= 256 else s
    h = 0 if s >= 256 else s
    ico += struct.pack("<BBBBHHII", w, h, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
    images += data
with open(os.path.join(OUT, "icon.ico"), "wb") as f:
    f.write(ico + images)

print("wrote", os.path.join(OUT, "icon.png"), os.path.getsize(os.path.join(OUT, "icon.png")), "bytes")
print("wrote", os.path.join(OUT, "icon.ico"), os.path.getsize(os.path.join(OUT, "icon.ico")), "bytes")
