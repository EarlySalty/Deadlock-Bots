#!/usr/bin/env python3
"""Generiert die Welcome-Hub-Banner im Website-Brand-Look (dl-brand).

Quelle der Wahrheit für Farben/Fonts ist das dl-brand-Paket im Website-Repo
(read-only). Ausgabe: assets/welcome-banners/*.png (1100x300), von
welcome-publish als Attachments verwendet. Neuer Banner = Eintrag in BANNERS.

Aufruf:  python3 scripts/gen_welcome_banners.py
"""

from __future__ import annotations

import io
import math
import pathlib

from fontTools.ttLib import TTFont
from PIL import Image, ImageDraw, ImageFont

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
BRAND = pathlib.Path("/home/naniadm/Documents/Website/dl-brand")
OUT_DIR = REPO_ROOT / "assets" / "welcome-banners"

W, H = 1100, 300

# dl-brand tokens.css
INK = (11, 9, 7)          # --ink #0b0907
INK_RAISED = (28, 22, 14)  # --ink-raised #1c160e
GOLD = (200, 168, 107)     # --gold #c8a86b
GOLD_BRIGHT = (239, 212, 157)  # --gold-bright #efd49d
GOLD_DARK = (128, 101, 52)     # --gold-dark #806534

BANNERS = [
    ("hero.png", "DEUTSCHE DEADLOCK", "COMMUNITY"),
    ("navigation.png", "KANAL", "NAVIGATION"),
    ("team.png", "COMMUNITY", "TEAM"),
    ("socials.png", "LINKS &", "SOCIALS"),
    ("router-hero.png", "DEADLOCK", "ROUTER"),
    ("rang-guide-hero.png", "STEAM", "VERKNÜPFEN"),
    ("regelwerk-hero.png", "COMMUNITY", "REGELWERK"),
    ("support-hero.png", "SERVER", "UNTERSTÜTZEN"),
    ("faq-hero.png", "SERVER", "FAQ"),
]


def woff2_to_ttf(woff2_path: pathlib.Path) -> io.BytesIO:
    font = TTFont(str(woff2_path))
    font.flavor = None
    buf = io.BytesIO()
    font.save(buf)
    buf.seek(0)
    return buf


def load_font(name: str, size: int) -> ImageFont.FreeTypeFont:
    buf = woff2_to_ttf(BRAND / "fonts" / name)
    return ImageFont.truetype(buf, size=size)


def key_out_background(img: Image.Image, tolerance: int = 28) -> Image.Image:
    """Entfernt den flachen Logo-Hintergrund (Farbdistanz zum Eckpixel)."""
    rgba = img.convert("RGBA")
    bg = rgba.getpixel((0, 0))[:3]
    data = [
        (r, g, b, 0 if abs(r - bg[0]) + abs(g - bg[1]) + abs(b - bg[2]) < tolerance else a)
        for r, g, b, a in rgba.getdata()
    ]
    rgba.putdata(data)
    return rgba


def grid_layer() -> Image.Image:
    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    step = 44
    for x in range(step, W, step):
        draw.line([(x, 30), (x, H - 30)], fill=GOLD_DARK + (14,), width=1)
    for y in range(step, H, step):
        draw.line([(48, y), (W - 48, y)], fill=GOLD_DARK + (14,), width=1)
    return layer


def vertical_glow(base: Image.Image) -> None:
    overlay = Image.new("L", (1, H), 0)
    for y in range(H):
        t = y / H
        overlay.putpixel((0, y), int(26 * math.sin(math.pi * t)))
    glow = Image.merge(
        "RGBA",
        [
            Image.new("L", (1, H), GOLD[0]),
            Image.new("L", (1, H), GOLD[1]),
            Image.new("L", (1, H), GOLD[2]),
            overlay,
        ],
    ).resize((W, H))
    base.alpha_composite(glow)


def spaced(text: str, tracking: str = "  ") -> str:
    return tracking.join(text)


def fitted_font(draw: ImageDraw.ImageDraw, text: str, start_size: int, max_width: int) -> ImageFont.FreeTypeFont:
    """Skaliert die Fontgröße runter, bis der Text in max_width passt."""
    size = start_size
    while size > 24:
        font = load_font("sora-latin.woff2", size)
        if draw.textlength(text, font=font) <= max_width:
            return font
        size -= 4
    return load_font("sora-latin.woff2", size)


def render(fname: str, line1: str | None, line2: str, logo: Image.Image) -> None:
    img = Image.new("RGBA", (W, H), INK + (255,))

    img.alpha_composite(grid_layer())
    vertical_glow(img)

    # Logo-Wasserzeichen rechts (freigestellt), dezent
    watermark = logo.resize((250, 250))
    watermark.putalpha(watermark.getchannel("A").point(lambda a: a * 10 // 100))
    img.alpha_composite(watermark, (W - 300, H // 2 - 125))

    badge = logo.resize((54, 54))
    badge.putalpha(badge.getchannel("A").point(lambda a: a * 85 // 100))
    img.alpha_composite(badge, (W - 112, H - 24 - 54 - 14))

    draw = ImageDraw.Draw(img, "RGBA")

    # Rahmen: feine Gold-Linien oben/unten, Eck-Akzente
    draw.line([(40, 24), (W - 40, 24)], fill=GOLD_DARK + (150,), width=1)
    draw.line([(40, H - 24), (W - 40, H - 24)], fill=GOLD_DARK + (150,), width=1)
    for cx, cy, dx, dy in [(40, 24, 1, 1), (W - 40, 24, -1, 1), (40, H - 24, 1, -1), (W - 40, H - 24, -1, -1)]:
        draw.line([(cx, cy), (cx + 18 * dx, cy)], fill=GOLD + (255,), width=3)
        draw.line([(cx, cy), (cx, cy + 18 * dy)], fill=GOLD + (255,), width=3)

    x = 72
    max_text_w = W - x - 320  # rechts Platz fürs Logo-Wasserzeichen lassen

    if line1 is not None:
        big = spaced(line2, " ")
        sora_big = fitted_font(draw, big, 64, max_text_w)
        sora_small = load_font("sora-latin.woff2", 30)
        draw.text((x, 74), spaced(line1, " "), font=sora_small, fill=GOLD + (255,))
        draw.text((x, 118), big, font=sora_big, fill=GOLD_BRIGHT + (255,))
        y_rule = 222
    else:
        # Ein-Zeilen-Banner (Sektions-Header): Headline vertikal zentriert
        big = spaced(line2, " ")
        sora_big = fitted_font(draw, big, 64, max_text_w)
        bbox = draw.textbbox((0, 0), big, font=sora_big)
        text_h = bbox[3] - bbox[1]
        y_text = (H - text_h) // 2 - bbox[1] - 14
        draw.text((x, y_text), big, font=sora_big, fill=GOLD_BRIGHT + (255,))
        y_rule = y_text + bbox[3] + 26

    # Zierlinie unter der Headline
    draw.line([(x, y_rule), (x + 340, y_rule)], fill=GOLD + (200,), width=2)
    draw.line([(x + 348, y_rule), (x + 420, y_rule)], fill=GOLD_DARK + (160,), width=2)

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    img.convert("RGB").save(OUT_DIR / fname, "PNG")
    print(f"{fname}: {OUT_DIR / fname}")



DIVIDERS = [
    ("divider-information.png", "INFORMATION"),
    ("divider-community.png", "COMMUNITY"),
    ("divider-deadlock.png", "DEADLOCK"),
    ("divider-medien.png", "MEDIEN"),
    ("divider-coaching.png", "COACHING"),
    ("divider-router.png", "VOICE & LANES"),
    ("divider-custom.png", "CUSTOM GAMES"),
    ("divider-quickstart.png", "SCHNELLSTART"),
    ("divider-lane-verwalten.png", "LANE VERWALTEN"),
    ("divider-anleitung.png", "ANLEITUNG"),
]

def render_divider(fname: str, label: str, logo: Image.Image) -> None:
    """Sektions-Header in voller Banner-Größe (gleicher Look wie hero.png)."""
    render(fname, None, label, logo)


def render_badge(logo: Image.Image) -> None:
    badge = Image.new("RGBA", (256, 256), (0, 0, 0, 0))
    draw = ImageDraw.Draw(badge)
    draw.ellipse([4, 4, 252, 252], fill=INK + (255,), outline=GOLD + (255,), width=5)
    draw.ellipse([16, 16, 240, 240], outline=GOLD_DARK + (140,), width=2)
    badge.alpha_composite(logo.resize((196, 196)), (30, 30))
    badge.save(OUT_DIR / "logo-badge.png", "PNG")
    print(f"logo-badge.png: {OUT_DIR / 'logo-badge.png'}")

def main() -> None:
    logo = key_out_background(Image.open(BRAND / "logo" / "deadlock-d-logo.png"))
    for fname, line1, line2 in BANNERS:
        render(fname, line1, line2, logo)
    for fname, label in DIVIDERS:
        render_divider(fname, label, logo)
    render_badge(logo)


if __name__ == "__main__":
    main()
