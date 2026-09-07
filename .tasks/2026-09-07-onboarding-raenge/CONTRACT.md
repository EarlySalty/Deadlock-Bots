# Contract: Discord-Onboarding auf aktuelle Deadlock-Ränge und -Bilder

status: aktiv
datum: 2026-09-07
klasse: mittel
repo: Deadlock-Bots

Dieser Contract ist der Maßstab für Implementierung und Merge-Kritiker. Nach dem
Anlegen ist er unveränderlich: der Hook lässt nur noch die `status:`-Zeile und
Anhänge unter `## Amendments` zu. Wer ein REQ oder INV ändern will, schreibt ein
Amendment mit Begründung; Produkt-, API- oder Datenänderungen entscheidet der User.

## Ziel

Wer dem Discord beitritt, sieht in der Rang-Frage die aktuellen Deadlock-Rangnamen
und die aktuellen Rangbilder aus der Deadlock-API, nicht mehr Alchemist, Arcanist
und Archon mit den alten Emojis.

## Anforderungen (user-sichtbares Verhalten)

- REQ-01: Die Onboarding-Frage „Wähle hier deinen aktuellen Deadlock Rang aus.“
  listet die elf aktuellen Ränge Initiate, Seeker, Acolyte, Sentinel, Mystic,
  Ritualist, Emissary, Oracle, Phantom, Ascendant, Eternus plus „Neu im Game /
  Rang ist Unbekannt“. Alchemist, Arcanist und Archon kommen dort nicht mehr vor.
- REQ-02: Jede der elf Rang-Optionen zeigt das aktuelle Rangbild aus
  `GET https://api.deadlock-api.com/v1/assets/ranks` (large-PNG je Tier 1–11)
  als Guild-Emoji. Die Option „Neu im Game / Rang ist Unbekannt“ bleibt bei ❓.
- REQ-03: Jede Rang-Option behält dieselbe unverifizierte Rang-Rolle wie zuvor
  (Zuordnung über die Rollen-ID, nicht über den alten Namen). Initiate bleibt
  Initiate-unverifiziert, die bisherige Alchemist-Option bleibt Acolyte-unverifiziert
  (1492960350755225730), analog Sentinel/Mystic/Ritualist/Emissary.
- REQ-04: `build_welle2b_onboarding_config` schreibt die aktuellen Titel und
  Emoji-IDs auf den übernommenen Rang-Prompt, statt ihn unverändert zu kopieren.
  Ein späteres Onboarding-Apply darf die alten Namen nicht zurückspielen.
- REQ-05: Die zusätzlichen Live-Prompts „Willst du Starthilfe?“ und „Deinen
  Account verknüpfen“ bleiben beim Apply erhalten (Live hat fünf Prompts;
  der Builder darf sie nicht mehr still streichen).
- REQ-06: LFG-Rangauswahl und Rang-Namenslisten im Bot (TempVoice, Stats,
  Activity-Parser, Rank-Voice) nutzen dieselben aktuellen Namen, damit ein
  Nutzer mit Rolle „Acolyte 3“ nicht als ranglos gilt. Alte Namen bleiben
  Aliasse auf denselben Tier-Index, soweit sie nicht selbst ein aktueller
  offizieller Name sind (Ritualist und Emissary folgen dem neuen Index).

## Invarianten (darf sich nicht ändern)

- INV-01: Option-IDs und Prompt-ID des Rang-Prompts bleiben die Live-Snowflakes
  (kein Neu-Anlegen der Frage).
- INV-02: Weiche-Prompt und Ping-Prompt werden weiter vom Builder gebaut, nicht
  aus der Live-Config übernommen.
- INV-03: Default-Kanäle bleiben Owner-Auswahl aus Live, unverändert.
- INV-04: Obscurus bekommt keine Onboarding-Option und keine neue Rolle.
- INV-05: Bestehende Tests dürfen nicht gelöscht oder abgeschwächt werden;
  Erwartungen auf alte Rangnamen als aktuelle Labels werden auf die neuen Namen
  umgestellt.

## Nicht-Ziele

- Keine neuen Discord-Rollen, keine Umbenennung der bereits umbenannten
  Subrang- und Unverifiziert-Rollen.
- Kein Umbau von Weiche, Pings, Server-Guide oder Rang-Guide-Texten.
- Website-Frontend und Steam-Bot (Namen dort schon aktuell) bleiben außen vor.
- Kein Obscurus-Onboarding, kein zweites „Unbekannt“-Emoji.

## Erlaubter Änderungsbereich

- `rust/bin/dl-bot/src/serversync.rs` (Rang-Overlay, Extra-Prompts, Tests)
- `rust/crates/dl-voice/src/lfg_panel.rs` (Rang-Select Labels/Values/Emoji-IDs)
- `rust/crates/dl-voice/src/tempvoice/logic.rs` (RANK_ORDER, Kurzformen, Aliasse)
- `rust/crates/dl-voice/src/rank.rs` (Anzeigenamen, RANK_VALUES, SHORT_TO_RANK)
- `rust/crates/dl-voice/src/adaptive.rs` (Kommentare/Testdaten mit alten Namen)
- `rust/bin/dl-bot/src/modglue.rs` (DISCORD_RANK_ROLES, UNVERIFIED_RANK_ROLES,
  RANK_SHORT_NAMES)
- `rust/crates/dl-activity/src/lfg.rs` (RANK_NAMES, RANK_TOKENS, Aliasse)
- `rust/crates/dl-stats/src/ranks.rs` (RANK_ORDER, RANK_COLORS)
- `rust/crates/dl-community/src/scrim_signup.rs` (RANK_ROLE_IDS-Namen)
- Tests in denselben Dateien plus abhängige Testdaten
- `.tasks/2026-09-07-onboarding-raenge/`

## Verbotene Änderungen

- Andere Onboarding-Prompts inhaltlich umschreiben
- Role-IDs der Rang-Rollen ändern
- Migrationen, Lint-Config, Infisical, ENV-Schalter
- `Cargo.toml` / neue Crates

## Offene Produktfragen

- keine

## Amendments

- 2026-09-07: REQ-04 und REQ-05 gelten nicht. Server Sync ist aktuell nicht korrekt; die Rangfrage wird nicht über `onboarding-apply` geschrieben. Live-Schreibe nur per Discord-REST über den Bot-MCP (`api_call`). Overlay in `serversync.rs` ist zurückgenommen. entschieden von User
