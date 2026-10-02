# Briefing Delegator: mitspieler-pool

[Orchestrator] Du führst den Auftrag `mitspieler-pool` als Delegator. Du baust nicht selbst.

- Auftrag: `/home/nathanael/repos/Deadlock-Bots/.tasks/2026-10-02-mitspieler-pool/AUFTRAG.md`
- Pakete: `.../PAKETE.md` (gleicher Ordner), Register: `.../REGISTER.md`
- Intent-Thread: b61e3506

## Deine Aufgabe

1. AUFTRAG.md und PAKETE.md vollständig lesen.
2. Paket A als eigenen T3-Thread starten (`t3-thread.py new --project Deadlock-Bots --rolle
   worker_mittel --title "Spielerpool (A)"`), Briefing nach
   `~/.claude/skills/rolle-intent-agent/templates/WORKER-BRIEFING.md` als Datei im
   `.tasks`-Ordner, Intent-Thread im Briefing ist DEIN Thread (Bump-ups und Fertigmeldungen
   kommen zu dir).
3. Nach Merge von A: B, C, E parallel starten (je eigener Thread, Worktree, Branch), D nach
   dem Vertrag aus PAKETE.md im Projekt `Website`. Modell je Paketgröße aus `/local-pyramide`
   (`worker_klein` / `worker_mittel` / `worker_gross`).
4. Review allein über den Merge-Gate (`gate_hook.py --review`). Bei BLOCK je Runde ein NEUER
   Fixer-Thread (`--rolle fixer`), Funde nie in den Thread des Implementierers.
5. Register und PAKETE.md bei jeder Änderung nachziehen. Fertige Threads settlen.
6. Am Ende: Fertig-Kriterium aus AUFTRAG.md mit dem Nutzer-Konto prüfen lassen, was nur der
   Nutzer selbst klicken kann (Steam-Login, Zweitkonto), als kurze Testanleitung an
   `t3-thread.py send --thread b61e3506 "[Fertig] mitspieler-pool: ..."` melden.

## Regeln

- Du bist der einzige Delegator-Thread. Keine Claude-Subagenten für Bauarbeit.
- `send` nie mit `--model`. Kontingent-Sperren nicht speichern; ist ein Modell weg, nächstes
  Modell der Zeile starten.
- Nur bei Produktfragen, Datenverlust oder Geldwirkung an den Intent-Thread zurückfragen,
  sonst selbst entscheiden und im Register begründen.
- Allerletzter Schritt nach der Fertigmeldung: `t3-thread.py settle --selbst`.
