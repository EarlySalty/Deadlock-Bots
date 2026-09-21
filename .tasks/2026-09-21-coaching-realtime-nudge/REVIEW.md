# Review Runde 1: Coaching-Notification-Nudge

Stand: 2026-09-21 14:21Z — R1 beider Pakete abgeschlossen und hier abgelegt (B: Thread 6aa8b223, 14:10:47Z; A: Thread 6a44ab2b, 14:18:25Z). Beide Reviewer-Threads gesettelt, tote opus48-Nachfolger (4f14e725, dbcc4f2d) gesettelt.

---

## Paket B — Website-Backend (Review-Thread 6aa8b223, abgeschlossen 14:10:47Z)

Geprüft: `git diff b271acc..2de42c3` im Worktree `/home/nathanael/.worktrees/coaching-nudge-website-20260921` (clean, Branch `feat/coaching-notifications-nudge-20260921`), 3 Dateien. Zeilenangaben Stand 2de42c3.

### Mängelliste

**M1 — Verschiebung eines Termins macht die ‚created‘-Notification wieder fällig, ohne dass ein Nudge feuert („vierte Stelle“).**
- `builds/backend-rust/src/routes/platform.rs:979-988` (Bestandscode, unverändert): bei `scheduled_at`-Änderung setzt `update_appointment` `notify_created_at=NULL` **und** `notify_reminder_at=NULL`.
- `builds/backend-rust/src/routes/platform.rs:1025`: die due-Abfrage für ‚created‘ hat **kein Zeitfenster** (`status='scheduled' AND notify_created_at IS NULL`) — jede Verschiebung eines geplanten Termins macht die ‚created‘-Zustellung (aktualisierter Termin als neue DM) sofort wieder fällig.
- `builds/backend-rust/src/routes/platform.rs:998-1006`: der Nudge feuert ausschließlich beim Übergang auf ‚cancelled‘. Für Verschiebungen bleibt damit nur der 60-s-Poll.
- Widerspruch im Auftrag selbst: Ziel „Best-effort-Nudge nach **jeder** Mutation, die eine fällige Notification erzeugt“ (`AUFTRAG.md:16-18`) gegen die Arbeitsschritte, die nur drei Stellen listen (`AUFTRAG.md:50-56`). Der Reminder-Anteil des Re-Arms ist laut Prüfauftrag bewusst ohne Nudge; der mitaufgerichtete ‚created‘-Anteil ist aber eine echte fällige Notification aus einer Mutation ohne Nudge. → Entscheidung beim Orchestrator: Nudge auch bei `scheduled_at`-Wechsel, oder Lücke bewusst als Fallback-Fall akzeptieren.

### Boundary-Fälle, geprüft und in Ordnung (keine Mängel)

- `platform_sync`-INSERT (`routes/platform.rs:36`, `:145-175`) kann Zeilen mit `website_request_id` + Status ‚analyzed‘ anlegen (→ fällig), ist aber `require_bot_token`-geschützter Bot-Ursprung; der Bot hat die Mutation selbst ausgeführt und ist aktiv. Vertrag sieht Nudge nur für Website-Ursprung vor.
- `UPDATE coaching.requests SET status='matched'` (`routes/coaching.rs:478`) nimmt aus der Due-Menge heraus (‚matched‘ fehlt im Statusfilter, `routes/platform.rs:1062`); keine neue fällige Notification.
- Cancel + Reschedule in **einem** Request: Reset macht `notify_created_at=NULL`, damit ist weder ‚created‘ (Status-Filter) noch ‚cancelled‘ (verlangt `notify_created_at IS NOT NULL`, `routes/platform.rs:1044-1045`) fällig; der Nudge feuert dann ins Leere — harmlos, und Bestandsverhalten, das dieser Diff nicht verschlechtert.

### Prüfpunkte

**(1) Interface-Vertrag — keine Mängel.** `POST {MASTER_BROKER_BASE}/internal/master/v1/coaching/notifications-nudge` (`discord_broker.rs:403-414`), `X-Internal-Token` (`:413`), Token aus der bestehenden Kette `MASTER_BROKER_TOKEN → MAIN_BOT_INTERNAL_TOKEN → TWITCH_INTERNAL_API_TOKEN` (`config.rs:175-179`), Base aus `master_broker_base` (`config.rs:174`). Kein Body gesetzt (kein `.json()`/`.body()`), Test beweist `body().is_none()`.

**(2) Fire-and-forget — keine Mängel.** `tokio::spawn` in `spawn_coaching_notifications_nudge` (`discord_broker.rs:415-423`); der Request-Pfad awaitet den Aufruf nie. Fehler nur `tracing::warn!` inklusive Nicht-200 (`discord_broker.rs:420-422`, `:434-436`). Fehlender Token → Early-Return vor dem Spawn, gar kein Versand (`:416-418`). Der genutzte `state.http`-Client hat 20 s Timeout (`app.rs:48-50`), der Task kann nicht ewig hängen.

**(3) Verdrahtung — genau drei Stellen, korrekt; ein Mangel (M1 oben).**
- `create_appointment`: Nudge bedingungslos nach INSERT (`routes/platform.rs:867-878`, Nudge `:880-884`) ✓
- `update_appointment`: nur Übergang auf ‚cancelled‘, Alter Status aus dem bestehenden SELECT (`:957`), wiederholtes cancelled nudgt nicht (`:998-1006`); leerer Body/Frühreturn bei `values.is_empty()` (`:976`) nudgt nicht ✓
- `create_coaching_request`: nur bei `website_request_id.is_some()` (`routes/coaching.rs:391-396`); Bot-Zweig ohne `website_request_id` (Generierung blockt bei `bot_request_id`, `coaching.rs:340-346`) nudgt nicht ✓
- M1 = Verschiebung als fehlende vierte Stelle.

**(4) Scope-Zaun — keine Mängel.** Diff umfasst exakt die drei genannten Dateien; `notifications_due`/`notifications_ack` unberührt; keine Schema-/Migrations-Änderungen; Caddy unberührt; keine neuen Code-Kommentare im Diff (das Kommentar `platform.rs:1060` existiert bereits in b271acc:1042); kein fmt-Drift.

**(5) Tests — keine Mängel.** `coaching_notifications_nudge_request_sets_header_and_url` (`discord_broker.rs:801-822`): Method POST, exakte URL inkl. Trailing-Slash-Trim, Header `X-Internal-Token`, `body().is_none()` — im Stil des bestehenden `discord_role_request_sets_header_and_json_body` (`:482ff`), `Method`-Import vorhanden (`:477`).

**(6) Worker-Beweis „cargo test 165 passed“ — plausibel.** 166 Test-Attribute in `src/`, 0 in `tests/` (nur `fixtures/`), davon 1 `#[ignore]` (`src/db.rs:112`) → 165 lauffähige Tests, die Angabe ist rechnerisch konsistent. Der Harness erzwingt die lokale Test-Postgres: `test_state_with` → `dl_central_db::testing::test_pool()` (`app.rs:5141-5151`); `testing.rs:15,242,253` liest `CENTRAL_TEST_DSN`/`DATABASE_URL` und verweigert die Produktions-DB. Der neue Test selbst ist DB-frei (Unit). Caveat außerhalb des Diffs: `dl-central-db` ist eine Pfad-Dependency aus `~/repos/Deadlock-Bots` (`Cargo.toml:22`, aufgelöst über Symlink `.worktrees/Deadlock-Bots`); welcher Stand davon beim Testlauf kompiliert wurde, ist vom Diff aus nicht prüfbar.

---

**Ergebnis Paket B:** 1 Mangel (M1, Entscheidung beim Orchestrator nötig), alle übrigen Prüfpunkte ohne Befund. Read-only geprüft, nichts committet, nichts gefixt.

---

**Ergebnis Paket A:** 1 Mangel (Doc-Kommentar). Orchestrator-Entscheidung 2026-09-21: **geduldet** — die Datei dokumentiert Handler durchgängig per `///`, der Kommentar hält einen nicht-offensichtlichen Vertrag fest (Body ungelesen, idempotenter Nudge). Kein Fixer-Lauf für Paket A; R2 gegen die Liste entfällt, da der einzige Punkt per Entscheidung ohne Codeänderung erledigt ist. Paket A ist damit merge-fähig (Merge-Stand bbc3d05d gegen origin/main 65889a6e geprüft: Diff exakt die 4 Paketdateien).

---

## Paket A — Bot-Seite (Review-Thread 6a44ab2b, abgeschlossen 14:18:25Z)

Geprüft: `git diff bb03deb5..8d1c6e1f` im Worktree `/home/nathanael/.worktrees/coaching-nudge-bots-20260921` (4 Dateien, 107/6 Zeilen). Zeilenangaben gelten am Review-Commit `8d1c6e1f`. Durchgehend read-only; nichts committet, nichts gefixt.

### Mängelliste

1. **`rust/crates/dl-broker/src/handlers.rs:1759-1761`** — neuer `///`-Doc-Kommentar am Handler `coaching_notifications_nudge` verstößt gegen die Rahmen-Vorgabe „keine neuen Code-Kommentare". Abmildernd: Die Datei dokumentiert vergleichbare Handler durchgängig per `///` (z. B. `member_present` :277, `resolve_user` :387, `voice_members` :1456), Inhalt korrekt. → Orchestrator-Entscheidung über Duldung. Einzige Abweichung.

### Prüfpunkte

**(1) Interface-Vertrag — keine Mängel.** Route exakt `POST /internal/master/v1/coaching/notifications-nudge` (`lib.rs:246`, Handler `handlers.rs:1762`), kebab-case wie alle Geschwister-Routen. Auth über denselben `authorize()`-Pfad wie alle Token-Routen (`handlers.rs:1766`): Loopback-Check + `X-Internal-Token` gegen `BrokerState.token` mit konstanter Zeit (`lib.rs:312-343`); Token-Kette in `rust/bin/dl-bot/src/main.rs:1259-1263` unverändert. Kein Body-Extractor, Body bleibt ungelesen. Antwort `respond(200, json!({"ok": true}))` (`handlers.rs:1772`) ohne Envelope; kein Idempotency-Key, wie im Vertrag ausdrücklich vorgesehen.

**(2) Wake-Mechanik — keine Mängel.** Ein Nudge während eines laufenden `process_notifications()` geht nicht verloren: tokio 1.52.3 (laut `Cargo.lock` fixesimal) speichert `notify_one()` bei fehlendem Warter genau ein Permit (EMPTY → NOTIFIED), das das danach erzeugte `notified()` im `select!` (`coaching.rs:505-509`) konsumiert → außerplanmäßiger Lauf. Mehrere Nudges während eines Laufs verschmelzen zu einem Permit → ein sofortiger Sammellauf; unschädlich, da jeder Lauf alle fälligen Items abholt. Kantenfall „Permit liegt an, wenn der Sleep-Zweig gewinnt": zusätzlicher sofortiger Leerlauf, best-effort-korrekt.

**(3) 60-s-Fallback, Roster-Sync, Zustelllogik — keine Mängel.** `NOTIFICATION_INTERVAL` bleibt 60 s und weiter der Sleep-Zweig (`coaching.rs:21`, `:507`). Roster-Sync- und Role-Event-Task sind im Diff unangetastet, `process_notifications` unverändert, kein Eingriff in Caddy, Bot-Umgebung oder Python-Bestand.

**(4) Scope-Zaun — ein Mangel, siehe oben (Punkt 1).** Diff umfasst exakt die 4 vertraglichen Dateien, kein Fremd-Refactoring, kein fmt-Drift. Doku-Änderung am bestehenden Modulkopf (`coaching.rs:9-10`, „alle 60 s oder sofort per Nudge") ist inhaltsrichtig und die ausdrücklich zu bewertende erlaubte Kategorie: in Ordnung.

**(5) Tests — keine Mängel.** Handler-Test (`handlers.rs:2032-2063`): 401 ohne Token, 401 mit falschem Token, Negativ-Kontrolle „kein Permit durch unberechtigte Aufrufe" (20-ms-Timeout auf `notified()`), 200 mit exaktem Body `{"ok": true}`, Wake-Beweis; deterministisch. Loop-Wake-Test (`coaching.rs:759-793`): echte `spawn()`-Taskgruppe, erster Lauf abgewartet, fälliges Item gequeued, Ack innerhalb von 5 s; ohne die select-Verdrahtung wäre der Test rot. Nebeneffekte der mitgestarteten Roster-Tasks: keine (leerer Client → Abbruch nach Warnung). Hinweis: Das Interleaving „Nudge exakt während `process_notifications`" ist im Test nicht deterministisch abgedeckt, wird aber durch die tokio-Permit-Semantik (Punkt 2) abgesichert.

**(6) Baseline-Rots — gegenprobt, Behauptung bestätigt.** Direkter Baseline-Lauf am `bb03deb5` in isoliertem Temp-Worktree: `TESTNACHWEIS[TW-1]: 265 passed (dl-broker 33 + dl-community 232), 0 ignored | Baseline: 3 rot (bb03deb5, identische Tests)`. Beide Läufe mit `cargo test -p … -- --include-ignored` (cargo 1.97.1). Identische 3 Rots auf beiden Ständen, jeweils `CENTRAL_TEST_DSN muss gesetzt sein`: `privacy::runtime_gate_privacy_tests::privacy_loeschung_funktioniert_im_turniere_runtime`, `reaction_roles::runtime_gate_tests::reaction_role_schreibt_roster_nur_im_legacy_runtime`, `scrim_signup::tests::self_service_signup_schreibt_nur_im_legacy_runtime`. dl-community: 231 passed/3 failed (Baseline) vs. 232/3 (Review-Stand); +1 = exakt der neue Nudge-Test. dl-broker am Review-Stand: 33/0. `cargo build -p dl-bot` am Review-Stand: EXIT=0 (3m 23s).

**Beobachtung außerhalb des Review-Range (zur Kenntnis fürs Merge-Gate):** Branch-HEAD liegt inzwischen bei `bbc3d05d` (Merge von origin/main in den Feature-Branch, nach `8d1c6e1f`). Der Merge lässt den Nudge-Code in allen 4 Dateien byte-gleich intakt (verifiziert); er bringt aber Main-Inhalt in den Branch-Diff (u. a. Config-Refactor in `dl-bot/src/main.rs`, health-Handler-Erweiterung in `dl-broker`). Das Fertig-Kriterium „nur die genannten Dateien im Diff" ist dadurch nur auf die geprüfte Range `bb03deb5..8d1c6e1f` bezogen erfüllt; Kompilier- und Testnachweise beziehen sich auf `8d1c6e1f`. Empfehlung: Merge-Stand `bbc3d05d` vor dem Merge-Push gegen origin/main nachprüfen.

**Ergebnis Paket A:** 1 Mangel (Doc-Kommentar `handlers.rs:1759-1761`, Duldungsentscheidung beim Orchestrator), alle übrigen Prüfpunkte ohne Befund.
