"""Génère le logo de Vigie (PNG + ICO multi-tailles) dans la DA de l'Explorateur.

Carré crème arrondi, courbe d'activité bleue (un moniteur), point orange « live » au bout.
Usage : py -3 assets/make_icon.py
"""
from pathlib import Path
from PIL import Image, ImageDraw

CREAM = (242, 230, 204, 255)
NAVY = (19, 34, 58, 255)
BLUE = (46, 94, 158, 255)
LIVE = (242, 140, 50, 255)
S = 1024
SS = 4


def draw(px: int) -> Image.Image:
    k = SS * px / S
    W = int(px * SS)
    img = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    s = lambda v: int(round(v * k))
    d.rounded_rectangle([s(24), s(24), s(1000), s(1000)], radius=s(200), fill=CREAM)
    # barres de fond (comme un histogramme d'activité)
    for i, h in enumerate([220, 360, 260, 470, 330]):
        x = 150 + i * 150
        d.rounded_rectangle([s(x), s(860 - h), s(x + 90), s(860)], radius=s(26), fill=(214, 200, 170, 255))
    pts = [(110, 640), (300, 640), (390, 380), (500, 790), (610, 300), (700, 560), (790, 560)]
    d.line([(s(x), s(y)) for x, y in pts], fill=BLUE, width=max(1, s(78 if px > 24 else 110)), joint="curve")
    r = 92 if px > 24 else 130
    d.ellipse([s(820 - r), s(560 - r), s(820 + r), s(560 + r)], fill=LIVE)
    return img.resize((px, px), Image.LANCZOS)


here = Path(__file__).parent
draw(256).save(here / "icon.png")
draw(64).save(here / "icon64.png")
big = draw(256)
big.save(here / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
         append_images=[draw(n) for n in (16, 24, 32, 48, 64, 128)])
