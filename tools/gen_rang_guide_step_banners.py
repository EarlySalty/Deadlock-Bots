#!/usr/bin/env python3
"""Erzeugt die Schritt-Banner des Rang-Guides im Stil der divider-*-Familie.

Verfahren: divider-anleitung.png dient als Template (Rahmen, Gradient, Grid,
D-Watermark bleiben erhalten). Der Textbereich links wird durch Tiling einer
textfreien Spalte rekonstruiert, danach Kicker + Titel + Unterstrich im
Hero-Layout (kleine Zeile ueber grosser Zeile) neu gesetzt.

Aufruf: python3 tools/gen_rang_guide_step_banners.py
Schreibt nach assets/welcome-banners/rang-guide-*.png
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

REPO = Path(__file__).resolve().parent.parent
BANNERS = REPO / "assets" / "welcome-banners"
TEMPLATE = BANNERS / "divider-anleitung.png"
FONT = "/usr/share/fonts/truetype/noto/NotoSans-Light.ttf"

# Patch-Bereich: alter Text inkl. Unterstrich, innerhalb des Gold-Rahmens
PATCH_X0, PATCH_X1 = 55, 700
PATCH_Y0, PATCH_Y1 = 32, 268
FEATHER = 40  # weicher Uebergang am rechten Patch-Rand

TEXT_X = 72
KICKER_COLOR = (198, 169, 112)
TITLE_COLOR = (239, 212, 157)
UNDERLINE_COLOR = (200, 172, 110)
UNDERLINE_SEGMENTS = [(72, 412), (420, 492)]  # wie im Template vermessen

BANNER_TEXTS = [
    ("rang-guide-schritt-1.png", "SCHRITT 1", "STEAM ANMELDEN"),
    ("rang-guide-schritt-2.png", "SCHRITT 2", "FREUNDE WERDEN"),
    ("rang-guide-schritt-3.png", "SCHRITT 3", "RANG AKTIV"),
    ("rang-guide-schritt-4.png", "SCHRITT 4", "DISCORD-SIEGEL"),
    ("rang-guide-hilfe.png", "HILFE", "WENN'S MAL HAKT"),
]


def cleared_template() -> Image.Image:
    im = Image.open(TEMPLATE).convert("RGB")
    px = im.load()
    # Vertikalprofil aus textfreier Spalte (behaelt horizontale Grid-Linien)
    profile = []
    for y in range(im.height):
        r = g = b = 0
        for x in range(645, 656):
            pr, pg, pb = px[x, y]
            r += pr; g += pg; b += pb
        profile.append((r // 11, g // 11, b // 11))
    for y in range(PATCH_Y0, PATCH_Y1):
        pr = profile[y]
        for x in range(PATCH_X0, PATCH_X1):
            if x >= PATCH_X1 - FEATHER:
                a = (PATCH_X1 - x) / FEATHER
                orig = px[x, y]
                px[x, y] = tuple(int(p * a + o * (1 - a)) for p, o in zip(pr, orig))
            else:
                px[x, y] = pr
    return im


def draw_tracked(draw: ImageDraw.ImageDraw, pos, text, font, tracking, fill):
    x, y = pos
    for ch in text:
        draw.text((x, y), ch, font=font, fill=fill)
        x += draw.textlength(ch, font=font) + tracking


def tracked_width(draw, text, font, tracking):
    return sum(draw.textlength(ch, font=font) for ch in text) + tracking * (len(text) - 1)


def render(filename: str, kicker: str, title: str) -> None:
    im = cleared_template()
    draw = ImageDraw.Draw(im)

    kicker_font = ImageFont.truetype(FONT, 30)
    draw_tracked(draw, (TEXT_X, 58), kicker, kicker_font, 7, KICKER_COLOR)

    # Titelgroesse dynamisch, bis er in die Textzone links vom Watermark passt
    size = 84
    while size > 40:
        font = ImageFont.truetype(FONT, size)
        if tracked_width(draw, title, font, size * 0.16) <= 630:
            break
        size -= 2
    draw_tracked(draw, (TEXT_X, 108), title, font, size * 0.16, TITLE_COLOR)

    for x0, x1 in UNDERLINE_SEGMENTS:
        draw.rectangle([x0, 214, x1, 216], fill=UNDERLINE_COLOR)

    out = BANNERS / filename
    im.save(out)
    print(f"{out.name}: {im.size[0]}x{im.size[1]} geschrieben")


if __name__ == "__main__":
    for filename, kicker, title in BANNER_TEXTS:
        render(filename, kicker, title)
