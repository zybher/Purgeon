"""Rasterizes purgeon-icon.svg (plain shapes only) into the files the app embeds:
purgeon.ico (16-256 px, for the exe), purgeon-256.png (README) and icon-128.rgba (raw RGBA for the
window/taskbar icon, so the app needs no image decoder). Run after changing the SVG:

    python assets/make_icons.py
"""
import os
from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
TILE = (0x1F, 0x1F, 0x22, 255)
WHITE = (255, 255, 255, 255)
BLUE = (0x3B, 0x82, 0xF6, 255)
SS = 16  # supersampling factor


def draw(size):
    n = size * SS
    k = n / 100.0  # the SVG's viewBox is 0..100
    im = Image.new('RGBA', (n, n), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)

    def rr(x0, y0, x1, y1, r, col):
        d.rounded_rectangle([x0 * k, y0 * k, x1 * k - 1, y1 * k - 1], radius=r * k, fill=col)

    rr(0, 0, 100, 100, 22.5, TILE)
    # chip outline: rect 29..71, rx 11, stroke 7 (centred on the edge)
    rr(29 - 3.5, 29 - 3.5, 71 + 3.5, 71 + 3.5, 11 + 3.5, WHITE)
    rr(29 + 3.5, 29 + 3.5, 71 - 3.5, 71 - 3.5, 11 - 3.5, TILE)
    # pins: round-capped strokes 5.5 wide
    h = 5.5 / 2
    for x in (43, 57):
        rr(x - h, 20 - h, x + h, 29 + h, h, WHITE)
        rr(x - h, 71 - h, x + h, 80 + h, h, WHITE)
    for y in (43, 57):
        rr(20 - h, y - h, 29 + h, y + h, h, WHITE)
        rr(71 - h, y - h, 80 + h, y + h, h, WHITE)
    # re-cut the outline's inside where pin caps overlapped it, then the core
    rr(29 + 3.5, 29 + 3.5, 71 - 3.5, 71 - 3.5, 11 - 3.5, TILE)
    rr(41, 41, 59, 59, 5, BLUE)
    return im.resize((size, size), Image.LANCZOS)


big = draw(256)
big.save(os.path.join(HERE, 'purgeon-256.png'))
sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
imgs = {s: draw(s) for s in sizes}
imgs[256].save(os.path.join(HERE, 'purgeon.ico'), sizes=[(s, s) for s in sizes], append_images=[imgs[s] for s in sizes[:-1]])
with open(os.path.join(HERE, 'icon-128.rgba'), 'wb') as f:
    f.write(imgs[128].tobytes())
print('ok')
