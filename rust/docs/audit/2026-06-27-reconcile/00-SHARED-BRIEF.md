# Re-Konziliations-Audit Py→Rust — Gemeinsames Briefing (2026-06-27)

**Repo:** `/home/naniadm/Documents/Deadlock-Bots` (bereits dein cwd).
**Lies dieses Dokument zuerst, dann bearbeite deinen Subsystem-Auftrag.**

## Lage
- **Python ist LIVE und Source-of-Truth:** `cogs/`, `bot_core/`, `main_bot.py`.
- **Rust-Port (`rust/`) ist ~50 % Parität und aktuell INAKTIV** (`deadlock-bot-rust.service` installiert, aber aus; `deadlock-bot.service` läuft Python).
- **Ziel:** Rust-Port auf volle Parität bringen, damit er Python ablösen *kann*. Es werden **nur echte Lücken** geschlossen — **niemals bewusste Rust-Änderungen rückgängig gemacht**.

## Schon erledigte Arbeit (NICHT erneut als Lücke melden)
Ein Voll-Audit existiert: `rust/docs/audit/2026-06-21-py-rust-discord-parity-audit.md` (169 Befunde: 0 crit / 26 high / 71 med / 61 low / 11 widerlegt). **Lies den Abschnitt deines Subsystems.**

Danach wurden Wellen W1–W7 implementiert. Relevante Commits (git log):
- `99836a3` SecurityGuard + AIModerator Verhaltens-Parität (Welle 1)
- `135aa70` Welle-3 Parity #2-5,19-22 (Konstanten/Verhalten)
- `d52bfd7` SecurityGuard Shadow-Modus + False-Ban-Fix
- `0e294d0` Fremd-Invite-Erkennung + Neu-Account-Eskalation
- `24b9bb7` dl-dashboard fail-closed Auth + Security-Header (#24/#25)
- `3c52050` master-broker read-only `/discord/channel-info`
- `b866b5d` scam_revoke view_spec + Revoke-Button (#26)
- `fde6450` AI-Tool-Use-Loop für Ticket-Auto-Hilfe (#13-15)
- `31537fb` Coaching-Panel verweist auf Website (#16; **#17/#18 BEWUSST ENTFALLEN**)
- `7cc2996` Build-Publisher-Steuerlogik als gateway-gated Loop (#23)
- `4251927` + `9bb0e16` Bild-Scam-Vision auf OpenAI gpt-5.4-nano (#161)
- `a8d346c` master-broker `POST /discord/role/create`
- `016de1e` master-broker guild-stats + resolve-names, aiohttp ohne Brotli

**WICHTIG (Stale-Cache-Regel):** Verlasse dich NICHT auf Commit-Texte. Verifiziere jeden „erledigt"-Schluss am **aktuellen Rust-Code**. Ein Commit kann eine Sache nur teilweise umgesetzt haben.

## Bewusste Änderungen / Out-of-Scope (NICHT als Lücke melden)
- `cogs/bug_reporter.py` — bewusst NICHT portiert. Überspringen.
- `cogs/player_finder.py` + `cogs/steam` — in `cog_blocklist.json`, in Python live **nicht geladen**. Überspringen (keine Parität nötig).
- Coaching-Panel-Buttons **#17/#18** — bewusst entfallen.
- Cog-Hot-Reload (`!master reload`/`discover`/`unload`) — im statischen Rust-Binary konzeptionell nicht abbildbar. Nur `status`/`restart`/`sync_commands` sind portierbar und zählen als Lücke.
- Überall wo Audit, Changelog oder ein Code-Kommentar „bewusst" / „by design" / „deliberately" sagt → als gewollt behandeln, nur unter 🔵 DELIBERATE notieren, NICHT als Lücke.

## Deine Methode (read-only, KEINE Code-Änderung, KEIN Commit)
Zeilengenauer Vergleich Python-live ↔ aktueller Rust-Code für dein Subsystem. Klassifiziere jedes Verhalten / Command / Flow:
- ✅ **PARITY** — implementiert & verhaltensgleich (stichprobenartig verifizieren, nicht erschöpfend auflisten)
- ❌ **GAP** — fehlt oder weicht user-sichtbar/korrektheitsrelevant ab (DAS ist das Ziel)
- 🔵 **DELIBERATE** — weicht ab, aber gewollt (Quelle zitieren: Commit/Changelog/Kommentar)

Für jeden GAP:
| Severity | Typ | Python-Ref (file:line) | Rust-Ref (file:line oder ABSENT) | User-sichtbare/Korrektheits-Auswirkung (1–3 Sätze) | Aufwand S/M/L |

Sei **adversarial gegen deine eigenen GAP-Behauptungen**: wenn du nicht sicher bist, dass es eine echte Divergenz ist, markiere `_(unverifiziert)_`. Severity-Skala: high = user-sichtbarer Funktionsverlust oder Korrektheits-/Sicherheitsbug; medium = spürbare Verhaltensabweichung; low = kosmetisch/Edge-Case.

## Output
1. Schreibe deinen Bericht nach `rust/docs/audit/2026-06-27-reconcile/<deine-nummer>-<subsystem>.md` (Markdown-Tabelle + Kurz-Summary mit Zählung nach Severity).
2. Gib als finale Nachricht zurück: Zählung der GAPs nach Severity + die Top-5-Lücken (eine Zeile je Lücke).

## Tabu
Keine Secret-Files lesen (`*.env`, `service_token.json`, `/proc/*/environ`). Keine Library/API erfinden. Deutsch oder Englisch ok.
