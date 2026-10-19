"""Regenerates the Tauri icon set (GS monogram) in this directory.

Style replicates the original icon: dark blue-gray vertical gradient with
rounded corners, a centered semi-transparent orange grid panel, and gold
letters on top.

Usage:  python make_icons.py
"""

from PIL import Image, ImageDraw, ImageFont
import os

MASTER = 1024  # render size before downscaling

# Style constants, measured from the original 256px icon (fractions of size).
RADIUS = 40 / 256          # rounded-corner radius of the background
BG_TOP = (30, 34, 46)
BG_BOTTOM = (48, 50, 70)
PANEL = 174 / 256          # side length of the centered grid panel
GRID_STEP = 64 / 256       # grid lines at 64, 128, 192 (both axes)
GRID_WIDTH = 4 / 256
GRID_COLOR = (255, 150, 60, 70)
LETTER_GOLD = (255, 200, 120, 255)
CAP_HEIGHT = 91 / 256
LETTER_GAP = 19 / 256
TEXT = "GS"

FONT_CANDIDATES = [
    r"C:\Windows\Fonts\segoeui.ttf",
    r"C:\Windows\Fonts\arial.ttf",
]

S = MASTER
PX = lambda f: round(f * S)


def find_font():
    for p in FONT_CANDIDATES:
        if os.path.exists(p):
            return p
    raise SystemExit("no usable TrueType font found")


def glyph_tile(font_path, ch, cap_px):
    """Render one glyph, tightly cropped, scaled so its bbox height == cap_px."""
    size = int(cap_px / 0.7)
    for _ in range(10):
        font = ImageFont.truetype(font_path, size)
        x0, y0, x1, y1 = font.getbbox(ch)
        h = y1 - y0
        if abs(h - cap_px) <= 2:
            break
        size = max(8, round(size * cap_px / h))
    img = Image.new("RGBA", (x1 - x0 + 8, y1 - y0 + 8), (0, 0, 0, 0))
    ImageDraw.Draw(img).text((4 - x0, 4 - y0), ch, font=font, fill=LETTER_GOLD)
    bbox = img.getbbox()
    return img.crop(bbox)


def build_layers():
    # Background: vertical gradient inside a rounded rectangle (mask rendered
    # 4x oversized for smooth corners).
    bg = Image.new("RGBA", (S, S))
    d = ImageDraw.Draw(bg)
    for y in range(S):
        t = y / (S - 1)
        color = tuple(round(a + (b - a) * t) for a, b in zip(BG_TOP, BG_BOTTOM))
        d.line([(0, y), (S, y)], fill=color + (255,))

    ss = 4
    mask = Image.new("L", (S * ss, S * ss), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [0, 0, S * ss - 1, S * ss - 1], radius=PX(RADIUS) * ss, fill=255
    )
    bg.putalpha(mask.resize((S, S), Image.LANCZOS))

    # Letters: gold glyphs, vertically centered, fixed gap between them,
    # kept on their own layer so grids are drawn beneath them at every size.
    font_path = find_font()
    tiles = [glyph_tile(font_path, ch, PX(CAP_HEIGHT)) for ch in TEXT]
    gap = PX(LETTER_GAP)
    total = sum(t.width for t in tiles) + gap * (len(tiles) - 1)
    letters = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    x = (S - total) // 2
    for t in tiles:
        y = (S - t.height) // 2
        letters.alpha_composite(t, (x, y))
        x += t.width + gap
    return bg, letters


def draw_grid(img):
    # Semi-transparent orange lines inside a centered square. ImageDraw
    # writes raw RGBA values (no blending), so saved pixels keep alpha=70
    # exactly like the original file. Drawn per output size (never scaled)
    # to avoid resampling artifacts on the semi-transparent color.
    size = img.width
    inset = round((1 - PANEL) / 2 * size)
    end = size - inset
    w = max(1, round(GRID_WIDTH * size))
    d = ImageDraw.Draw(img)
    k = GRID_STEP
    while k < 1:
        c = round(k * size)
        lo, hi = c - w // 2, c + w - w // 2
        if inset < c < end:
            d.rectangle([lo, inset, hi, end], fill=GRID_COLOR)
            d.rectangle([inset, lo, end, hi], fill=GRID_COLOR)
        k += GRID_STEP


def render(bg, letters, size):
    img = bg.resize((size, size), Image.LANCZOS)
    draw_grid(img)
    img.alpha_composite(letters.resize((size, size), Image.LANCZOS))
    return img


def main():
    bg, letters = build_layers()
    master = render(bg, letters, 256)
    master.save("icon.png")
    render(bg, letters, 128).save("128x128.png")
    render(bg, letters, 32).save("32x32.png")
    ico_sizes = [(s, s) for s in (16, 24, 32, 48, 64, 128, 256)]
    master.save("icon.ico", format="ICO", sizes=ico_sizes)
    print("generated icon.png, 128x128.png, 32x32.png, icon.ico")


if __name__ == "__main__":
    main()
