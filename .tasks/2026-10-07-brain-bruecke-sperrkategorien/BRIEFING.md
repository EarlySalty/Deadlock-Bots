# Auftrag: Ticket-, Moderations- und Teamkategorien für das Brain hart sperren

Rolle: Blatt-Worker (GPT 6.1 Sol). Keine weiteren T3-Threads; native Subagenten nur als frische Fixer bei Gate-BLOCK.
Auftraggeber: Claude-Session `3fcd8f71-443e-48ae-825c-527eb52fbe56` im Auftrag des Nutzers.

## Nutzerentscheidung (07.10.2026)

Ticket-Inhalte und Moderationssachen liegen in eigenen Kategorien des Discord-Servers. Das Deadlock Brain darf diese Kanäle nie lesen oder hineinschreiben, egal wer fragt, auch nicht wenn der Kanal ausdrücklich genannt oder darum gebeten wird. Die Sperre gehört in die Brücke (dl-bot `/mcp/public`), nicht ins Modell oder in einen Prompt.

## Befund

`rust/bin/dl-bot/src/mcp/public.rs` berechnet die Sichtbarkeit aus den Rollen des fragenden Mitglieds (`access`, `effective_permissions`). Fragt ein Moderator, Admin oder Ticket-Ersteller, darf `read_messages` (und `send_message`) damit auch Moderations-, Team- oder Ticketkanäle erreichen, und die Inhalte gehen an das externe Modell. `public_channels` filtert nur über Rechte, nicht über eine feste Sperrliste.

## Auftrag

1. Bestand zuerst über Graphify (Skill `code-suche`): woher die Server-Struktur kommt (`dl-server-as-code`, Kategorien Tickets, Moderation, Team/Staff, private Bereiche), bestehende Konfiguration für Kanal-/Kategorielisten (z. B. `brain_channel_allowlist`, `runtime_config`). Vorhandene Konfiguration erweitern, keine ENV-Variablen, keine einkompilierten IDs außerhalb der bestehenden Server-as-Code-Quelle.
2. Harte Sperrliste von Kategorien (und deren Kindkanälen und Threads, inklusive Ticket-Kanälen, die zur Laufzeit neu entstehen) in der Brücke: `read_messages`, `send_message`, `public_server_facts` und jede Kanalauflistung lehnen gesperrte Kanäle ab, unabhängig von den Rechten des Fragenden. Fail-closed: unbekannte Elternkategorie oder fehlende Konfiguration gilt als gesperrt, nicht als offen.
3. Zusätzlich darf `access` nie Administrator-Rechte des Fragenden in die Sicht übernehmen.
4. Regressionstests: Moderator/Admin/Ticket-Ersteller fragt nach Ticket-, Mod- und Teamkanal per ID, per Thread-ID und per Auflistung, alles abgelehnt; normale öffentliche Kanäle funktionieren weiter.
5. fmt, Clippy, Tests der betroffenen Crates über `cargo-slot` (nie die alte exec/flock-Schleife), Gate `gate_hook.py --review` bis ALLOW (bei BLOCK je Runde frischer nativer Fixer), dann Merge nach main mit `git push origin HEAD:main` als Einzelschritt, regulärer dl-bot-Deploy, Neustart, Live-Beweis über `/mcp/public` mit einem echten Mod-Kanal (Antwort `forbidden`) und einem öffentlichen Kanal (funktioniert). Live-Tests nur mit Test- oder Wegwerfkonten.

## Grenzen

- Eigener Worktree `~/.worktrees/Deadlock-Bots-brain-sperrkategorien`, Branch `fix/brain-bruecke-sperrkategorien`.
- Keine Inhalte aus gesperrten Kanälen in Logs, Tests oder Berichte schreiben.
- Rust only, keine Code-Kommentare, Secrets nie im Klartext, keine ENV-Konfiguration. Deutsch ohne Em-Dashes, echte Umlaute.

## Bericht

`AN_HAUPT.md` in diesem Ordner: Ursache, gesperrte Kategorien (Name und ID), Commits, Testzahlen, Gate-Antwort wörtlich, Deploy-SHA, Live-Beweis, Pflichtzeilen `MERGEPROTOKOLL[MS-1]` und `LIVEBEWEIS[DV-1]`. Fertig heißt gemergt, deployt, live geprüft, aufgeräumt; dann `python3 ~/Documents/tools/t3-thread.py settle --selbst`.
