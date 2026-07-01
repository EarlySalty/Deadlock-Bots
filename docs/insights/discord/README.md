# Discord Server-Insights-Archiv

Discord zeigt Server-Insights nur für die letzten **120 Tage** und löscht ältere Daten
rollierend. Dieses Verzeichnis archiviert die CSV-Exporte dauerhaft, damit wir
Aktivierung, Retention und Wachstum über lange Zeiträume auswerten können.

## Ablage-Konvention

Pro Export ein Ordner mit dem **Download-Datum**:

```
docs/insights/discord/
├── 2026-07-01/          ← Export vom 1. Juli 2026
│   ├── guild-activation.csv
│   ├── guild-communicators.csv
│   └── …
└── YYYY-MM-DD/          ← nächster Export
```

**Refresh-Routine:** Etwa alle 3 Monate (vor Ablauf des 120-Tage-Fensters) in Discord
unter *Servereinstellungen → Insights* die CSVs exportieren, in einen neuen
datierten Ordner legen, committen, pushen. Überlappende Zeiträume sind gewollt —
beim Auswerten nach `interval_start_timestamp` deduplizieren.

## Dateien und Bedeutung

| Datei | Inhalt |
|---|---|
| `guild-activation.csv` | Neue Mitglieder pro Intervall + % die kommuniziert / Kanäle geöffnet haben |
| `guild-retention.csv` | % der neuen Mitglieder, die nach 1 Woche noch da sind |
| `guild-joins-by-source.csv` | Joins nach Quelle (Discovery, Invites, Vanity-URL, …) |
| `guild-leavers.csv` | Austritte, aufgeteilt nach Mitgliedsdauer (< 1 Monat / 1 Monat+) |
| `guild-communicators.csv` | Besucher pro Intervall + % die etwas geschrieben haben |
| `guild-message-activity.csv` | Nachrichten gesamt + Nachrichten pro Kommunikator |
| `guild-total-membership.csv` | Mitgliederzahl-Verlauf (Tageswerte) |

Alle Zeitstempel sind UTC; Intervalle sind i.d.R. 30-Tage-Fenster
(`interval_start_timestamp` = Fensterbeginn).

## Stand 2026-07-01 (Kurzbefund)

- Neue Mitglieder/Monat sinken: 232 → 175 → 144 → 123 (Mrz→Jun)
- Aktivierung sinkt: 36 % → 28 % der Neuen schreiben je eine Nachricht
- ~50 % der Leaver gehen im ersten Monat
- Joins kommen primär über Vanity-URL und Invites, Discovery ~15-20 %
