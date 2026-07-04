#!/usr/bin/env python3
"""Generiert die LFG-Forum-Tag-Emojis als gold getönte Lucide-Line-Icons.

Gleiches Muster wie gen_router_emojis.py: Lucide-SVG (MIT) auf den
dl-brand-Goldton umgefärbt, als 128x128-PNG gerendert.
Ausgabe: assets/lfg-tag-emojis/*.png — werden einmalig als Server-Emojis
hochgeladen und auf die available_tags des Forums 🎯mitspieler-suche
(1522769149208821881) gesetzt (emoji_id). Die Modus-Emojis (dl_casual/
dl_ranked/dl_brawl) stammen aus dem Router-Panel und werden wiederverwendet.

Emoji-IDs (Stand Erstanlage, Server "Deutsche Deadlock Community"):
  dl_rang_einsteiger      1522801035922636900
  dl_rang_fortgeschritten 1522801037549895720
  dl_rang_erfahren        1522801038770573403
  dl_rang_elite           1522801040288645190
  dl_rang_egal            1522801043803472064
  dl_lfg_aktiv            1522801045321945098
  dl_lfg_sucht            1522801046509064263

Aufruf:  <venv-python mit cairosvg+pillow> rust/scripts/gen_lfg_tag_emojis.py
"""
from __future__ import annotations

import pathlib
import urllib.request

import cairosvg

# dl-brand tokens.css: --gold-bright #efd49d (lesbar bei 22px Emoji-Größe)
GOLD_BRIGHT = "#efd49d"
STROKE_WIDTH = "2.25"
SIZE = 128
LUCIDE_BASE = "https://raw.githubusercontent.com/lucide-icons/lucide/main/icons"

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
OUT_DIR = REPO_ROOT / "assets" / "lfg-tag-emojis"

# (Emoji-Name, Lucide-Icon) — Rang-Brackets + Status. Modus kommt vom Router.
ICONS = [
    ("dl_rang_einsteiger", "sprout"),           # Rang-Tag "Einsteiger"
    ("dl_rang_fortgeschritten", "trending-up"),  # Rang-Tag "Fortgeschritten"
    ("dl_rang_erfahren", "flame"),              # Rang-Tag "Erfahren"
    ("dl_rang_elite", "gem"),                   # Rang-Tag "Elite"
    ("dl_rang_egal", "shuffle"),                # Rang-Tag "Rang egal"
    ("dl_lfg_aktiv", "radio"),                  # Status-Tag "Jetzt aktiv"
    ("dl_lfg_sucht", "search"),                 # Status-Tag "Sucht noch"
]


def fetch_svg(icon: str) -> str:
    req = urllib.request.Request(
        f"{LUCIDE_BASE}/{icon}.svg",
        headers={"User-Agent": "gen-lfg-tag-emojis/1.0"},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return resp.read().decode("utf-8")


def tint(svg: str) -> str:
    return svg.replace('stroke="currentColor"', f'stroke="{GOLD_BRIGHT}"').replace(
        'stroke-width="2"', f'stroke-width="{STROKE_WIDTH}"'
    )


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    for name, icon in ICONS:
        svg = tint(fetch_svg(icon))
        out = OUT_DIR / f"{name}.png"
        cairosvg.svg2png(
            bytestring=svg.encode("utf-8"),
            write_to=str(out),
            output_width=SIZE,
            output_height=SIZE,
        )
        print(f"{name}.png <- lucide/{icon}")


if __name__ == "__main__":
    main()
