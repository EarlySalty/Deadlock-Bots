#!/usr/bin/env python3
"""Regeneriert rust/docs/db-schema.sql aus der Produktions-DB (READ-ONLY).

Der Dump ist der Vertrags-Snapshot für Tests und Reviews — siehe
rust/docs/01-db-contract.md.
"""

from __future__ import annotations

import datetime as dt
import sqlite3
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
DB_PATH = REPO_ROOT / "data" / "deadlock.sqlite3"
OUT_PATH = REPO_ROOT / "rust" / "docs" / "db-schema.sql"


def main() -> None:
    con = sqlite3.connect(f"file:{DB_PATH}?mode=ro", uri=True)
    try:
        rows = con.execute(
            "SELECT type, name, sql FROM sqlite_master"
            " WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'"
            " ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'index' THEN 1"
            " WHEN 'trigger' THEN 2 ELSE 3 END, name"
        ).fetchall()
    finally:
        con.close()

    stamp = dt.date.today().isoformat()
    lines = [
        "-- GENERIERT von rust/scripts/dump_schema.py — nicht von Hand editieren.",
        f"-- Quelle: data/deadlock.sqlite3 · Stand: {stamp}",
        "",
    ]
    for typ, name, sql in rows:
        lines.append(f"-- {typ}: {name}")
        lines.append(sql.strip() + ";")
        lines.append("")

    OUT_PATH.write_text("\n".join(lines), encoding="utf-8")
    print(f"{len(rows)} Objekte → {OUT_PATH}")


if __name__ == "__main__":
    main()
