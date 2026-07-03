#!/usr/bin/env python3
"""Generiert die Router-Panel-Emojis als gold getönte Line-Icons.

Quelle: Lucide-Icons (MIT, https://github.com/lucide-icons/lucide),
umgefärbt auf den dl-brand-Goldton und als 128x128-PNG gerendert.
Ausgabe: assets/router-emojis/*.png — werden einmalig als Server-Emojis
hochgeladen; die Emoji-IDs stehen als Konstanten in dl-voice/src/router.rs.

Aufruf:  <venv-python mit cairosvg+pillow> scripts/gen_router_emojis.py
"""

from __future__ import annotations

import pathlib
import urllib.request

import cairosvg

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
OUT_DIR = REPO_ROOT / "assets" / "router-emojis"

# dl-brand tokens.css: --gold-bright #efd49d (lesbar bei 22px Emoji-Größe)
GOLD_BRIGHT = "#efd49d"
STROKE_WIDTH = "2.25"
SIZE = 128

LUCIDE_BASE = "https://raw.githubusercontent.com/lucide-icons/lucide/main/icons"

# (Emoji-Name, Lucide-Icon) — Namen sind die Server-Emoji-Namen.
ICONS = [
    ("dl_crown", "crown"),          # Owner übernehmen
    ("dl_limit", "users"),          # Limit setzen
    ("dl_rename", "pencil"),        # Umbenennen
    ("dl_kick", "user-minus"),      # Kick
    ("dl_ban", "ban"),              # Ban
    ("dl_unban", "rotate-ccw"),     # Unban
    ("dl_mode", "arrow-right-left"),  # Modus wechseln
    ("dl_casual", "gamepad-2"),     # Casual
    ("dl_ranked", "trophy"),        # Ranked
    ("dl_brawl", "swords"),         # Street Brawl
]


def fetch_svg(icon: str) -> str:
    with urllib.request.urlopen(f"{LUCIDE_BASE}/{icon}.svg", timeout=30) as resp:
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
