# Invite-Lounge: Brain-Status und 30-Minuten-Nachfrage

## Ziel

Wer in `#deadlock-invite` (Channel 1464736918951432222, Kategorie COMMUNITY 1289721245281292289) einen Steam-Freundescode postet, bekommt vom Deadlock Brain eine Antwort auf seinen Beitrag, mit Erwähnung. Die Bots erzeugen keine eigenen Antworttexte; alle Nutzertexte kommen vom Brain (Memory `ein-brain-statt-bot-antworten`).

## Ablauf je Anfrage (Spec des Nutzers, 06.11.2026)

1. Steam-Invite wird wie bisher über den Slash-Pfad `invite` (`SteamBotClient::invite_from_bot`) ausgelöst.
2. Das Brain antwortet auf die Anfrage mit dem tatsächlichen Stand:
   - "Freundschaftsanfrage ist raus" (Fa), Einladung folgt nach Annahme.
   - "Einladung ist raus".
   - Steam reagiert langsam: keine Fehlermeldung, Anfrage bleibt offen, es wird weiter geprüft, bis die Bots ihren Schritt gemacht haben. Erst dann kommt die nächste Antwort.
   - Konkretes Problem, wenn eines entsteht (z. B. Steam lehnt ab), in klarer Sprache.
3. Keine Antwort (weder Mensch noch Bot) 30 Minuten nach der Anfrage und noch keine bestätigte Einladung: das Brain fragt mit Erwähnung, ob die Einladung schon angekommen ist. Falls nicht: Einladung auslösen und nach Schritt 2 berichten.
4. Bereits eingeladen (Steam-Status oder Bestätigung des Nutzers): das Brain bestätigt kurz, ohne neue Einladung.

## Entscheidungen (von der Hauptsession getroffen, nicht offen lassen)

- Spätere Anfragen: Die 5-Minuten-Frische (`FRESHNESS_SECONDS`) verwirft heute still. Neu: Anfragen innerhalb des 7-Tage-Fensters (`NEWCOMER_MAX_JOIN_SECONDS`) werden bearbeitet. Anfragen älter als 7 Tage werden nicht mehr bearbeitet.
- Historie beim Deploy: Nur Anfragen, die nach dem Deploy-Zeitpunkt gepostet wurden, lösen Aktionen aus. Der Backfill darf keine Antworten oder Einladungen für ältere Posts auslösen. Die aktuell offenen Posts (fidelisfuchs 05.11., Patrick 06.11., S 06.11.) werden erst nach Freigabe des Nutzers bearbeitet.
- Nachfrage-Erwähnung: Die Antwort erwähnt den Verfasser (`allowed_mentions` mit `users`). Heute steht `parse: []`, das gilt nur für die Hinweis-Antwort und bleibt dort so.
- Keine Dauer-Fehlermeldungen: Pro Anfrage höchstens eine Nachricht je Zustand. Wiederholte Steam-Verzögerung erzeugt keine Wiederholungs-Posts.

## Wo der Code liegt

Bots (Worktree `/home/nathanael/.worktrees/invite-lounge-status-20261106`, Branch `invite-lounge-status-20261106`):
- `rust/crates/dl-community/src/invite_lounge.rs`: `record_message` (L359), `dispatch_due` (L468, Frische-Prüfung L478–497), `finish_dispatch` (L553), `record_history` (L602), `update_state` (L762), `is_fresh` (L857), Hinweis-Antwort `reply_text` (L430).
- `rust/crates/dl-community/src/invite_lounge_tests.rs`: bestehende Tests, müssen grün bleiben.
- `rust/bin/dl-bot/src/mcp/self_invite.rs`: Status-Evidenz für das Brain (Quelle des Eigenstatus).
- `rust/crates/dl-bridges/src/steam.rs` (`invite_from_bot`, L225).

Brain (Worktree `/home/nathanael/.worktrees/brain-invite-status-20261106`, Branch `brain-invite-status-20261106`):
- `rust/crates/brain-contracts/src/bot_tasks.rs`: `BotTask` hat heute nur `DiscordAnswer`, `TwitchTitle`. Dort fehlt eine Aufgabe für Invite-Status.
- `rust/crates/brain-contracts/src/invite.rs`: `SelfInviteStatus`, `InviteStatus` (Sent, Pending, FriendshipMissing, AlreadyHasGame, Error, Unknown, Unavailable) und `status_evidence`. Wiederverwenden, nicht neu erfinden.

Steam-Bot-Texte (Referenz für Wortlaut und Zustände, nicht ändern): `/home/nathanael/Documents/Deadlock-Steam-Bot/rust/crates/steam-flows/src/invite.rs`, z. B. L228 "📨 Freundschaftsanfrage ist raus", L61 "schon eingeladen".

## Regeln (aus CLAUDE.md und Memory)

- Rust, keine Python-Erweiterungen. Keine Code-Kommentare. Kein `.unwrap()` im Produktionspfad, kein unnötiges `unsafe`. Kleine Funktionen.
- Persistenz in Postgres (vorhandenes `kv`/`invite_lounge:*`). Keine neue SQLite-Ablage. Keine angewandten Migrationen ändern; neue nur, wenn unvermeidbar.
- Kein hart kodierter Modellname, keine Timeouts als Modellkonstanten. Brain-Provider über die zentrale Schicht.
- Brain-Antworttexte auf Deutsch mit echten Umlauten und ß, keine Gedankenstriche, ohne KI-Floskeln. Wortlaut wie vom Nutzer vorgegeben: "FA ist raus" bzw. "Einladung ist raus", bei Problemen die konkrete Ursache in klarer Sprache.
- Builds nach `/home/nathanael/Documents/claude-config/machines/v50671/RUST-BUILDS.md` und Memory `cargo-slot-statt-flock-schleife`. DB-Tests nur auf ionos (Memory `postgres-disk-und-test-db-last`); lokal reicht `cargo check`, `cargo clippy`, `cargo fmt --check` und die reinen Tests ohne DB.
- Keine Live-Aktion: kein Discord-Post, keine Steam-Freundschaftsanfrage, keine Einladung, kein Deploy, kein Restart, kein Merge nach main.
- Nur eigene Dateien stagen. Verifizierten Zwischenstand auf dem eigenen Branch committen. Kein Force-Push, keine übersprungenen Hooks. Commit-Message auf Englisch. Jede Commit-Message endet mit der Zeile `Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>`.

## Abnahme

1. Zustandsübergänge (Fa raus, Einladung raus, Steam-Verzögerung, Problem, 30-Minuten-Nachfrage, bereits eingeladen) als reine Funktionen mit Tests, die die Spec abbilden, nicht die Implementierung.
2. Bestehende `invite_lounge`-Tests bleiben grün.
3. `cargo check`, `cargo clippy`, `cargo fmt --check` in beiden Repos grün (nur die jeweils betroffenen Crates).
4. Eine Vorschau-Datei `.tasks/2026-11-06-invite-lounge-status/VORSCHAU.md` mit dem exakten deutschen Text jedes Zustands für Anfrage-Beispiele (ohne echte Freundescodes, Platzhalter nutzen).
5. Abschlussbericht: Branches, Commits, was getestet wurde, was nicht (DB-Tests, Live-Lauf), offene Punkte.
