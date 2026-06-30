#!/usr/bin/env python3
"""SP1 Inventar-Completeness-Gate: Inventar-JSON == lebende SQLite-DB (Tabellen + Spalten)."""
import argparse, json, sqlite3, sys


def live_schema(db_path):
    con = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    cur = con.cursor()
    cur.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
    tables = {r[0] for r in cur.fetchall()}
    cols = {}
    for t in tables:
        cur.execute(f'PRAGMA table_info("{t}")')
        cols[t] = {r[1] for r in cur.fetchall()}
    con.close()
    return tables, cols


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", required=True)
    ap.add_argument("--inventory", required=True)
    args = ap.parse_args()
    inv = json.load(open(args.inventory, encoding="utf-8"))
    inv_tables = {t["name"] for t in inv["tables"]}
    inv_cols = {t["name"]: {c["name"] for c in t["columns"]} for t in inv["tables"]}
    live_tables, live_cols = live_schema(args.db)

    problems = []
    missing = live_tables - inv_tables
    extra = inv_tables - live_tables
    if missing:
        problems.append(f"FEHLT im Inventar (in DB, nicht inventarisiert): {sorted(missing)}")
    if extra:
        problems.append(f"EXTRA im Inventar (inventarisiert, nicht in DB): {sorted(extra)}")
    for t in sorted(live_tables & inv_tables):
        cmiss = live_cols[t] - inv_cols[t]
        cextra = inv_cols[t] - live_cols[t]
        if cmiss:
            problems.append(f"  {t}: Spalten fehlen im Inventar: {sorted(cmiss)}")
        if cextra:
            problems.append(f"  {t}: Spalten extra im Inventar: {sorted(cextra)}")

    if problems:
        print(f"GATE FAIL ({inv['project']}): Inventar deckt die lebende DB NICHT exakt ab")
        for p in problems:
            print(p)
        sys.exit(1)
    print(f"GATE PASS ({inv['project']}): {len(live_tables)} Tabellen, Spalten vollständig abgedeckt")
    sys.exit(0)


if __name__ == "__main__":
    main()
