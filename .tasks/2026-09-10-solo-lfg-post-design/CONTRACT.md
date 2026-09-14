status: aktiv
Datum: 2026-09-10

# Contract: Solo-LFG Auto-Post als vollwertige Gold-Karte

## Ziel

Der Auto-Post des Solo-LFG in #mitspieler-suche liest sich als gebrandete Karte
mit klarer Handlungsanweisung statt als nackter Textblock. Der Leser erkennt auf
einen Blick: jemand sucht Mitspieler, hier ist der Modus, und Beitreten zieht ihn
direkt in die Lane.

## Anforderungen (user-sichtbar, prüfbar)

- REQ-1: Der Post zeigt oben das Banner `divider-mitspieler-finden.png` als Media Gallery.
- REQ-2: Die Kopfzeile ist eine Section mit Logo-Thumbnail (`logo-badge.png`), Titel
  „Mitspieler gesucht“ mit `dl_lfg_sucht`, darunter Modus-Emoji, Modus und Rang.
- REQ-3: Platzelfüllungen im Rangfeld (`n/a`, `na`, `-`, `–`, `?`, `egal`) erscheinen
  nicht als Rang-Suffix im Titel; ein echter Rang schon.
- REQ-4: Unter den Fakten steht ein CTA-Absatz, der sagt, was der Beitreten-Knopf tut
  und wofür Lane öffnen da ist.
- REQ-5: Eine kleine Fußzeile nennt die Automatik und die 3-Stunden-Grenze
  (Codestelle: `MAX_POST_AGE`, solo_watch.rs).
- REQ-6: Fehlt eine Asset-Datei auf dem Server, postet der Bot ohne Banner/Logo im
  alten Textaufbau, nie ohne den Eintrag selbst (Router-Muster: `std::fs::read().ok()`).

## Invarianten (ändert sich nichts)

- Kanal (`LFG_CHANNEL_ID`), Custom-IDs, Button-Verhalten (Beitreten zieht in die
  Lane, Lane öffnen ist Link), Lösch-Automatik, DM-Flow (`dm_body`), Modal,
  Persistenz, Tagesbilanz.
- Gold-Akzent, Components-V2-Flag, `allowed_mentions parse []`, Stumm-Post ohne Ping.
- `mode_and_emoji`: Kategorie zu Modus und Emoji bleibt unverändert.
- Panel- und Guide-Nachrichten des Routers und des LFG-Panels bleiben unangetastet.

## Nicht-Ziele

- Gesuch-Posts in `dl-activity/lfg.rs`, Router-Panels, Welcome-Nachrichten.
- Neuer Banner-Asset; es gibt nur existierende Dateien aus `assets/welcome-banners/`.

## Änderungsbereich

- `rust/crates/dl-voice/src/solo_watch.rs`: Post-Builder, Port-Signatur `post_lfg`,
  Tests.
- `rust/crates/dl-voice/src/glue.rs`: `post_lfg`-Implementierung mit Multipart über
  die bestehenden Helfer `lfg_panel_files` und `send_router_message_payload`.

Verboten: Dateien außerhalb des Bereichs, neue Feature-Flags.

## Offene Produktfragen

- keine. Design folgt dem bestehenden Look von LFG-Panel und Voice-Guide
  (Galerie oben, Gold-Karte, dl_-Emojis).

## Amendments

- (keine)
