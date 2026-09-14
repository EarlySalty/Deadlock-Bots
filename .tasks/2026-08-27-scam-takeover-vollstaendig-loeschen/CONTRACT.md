status: aktiv

# Contract: Takeover-Scam vollstaendig loeschen, eine Nachricht komplett spiegeln

## Ziel
Bei einem account_takeover-Treffer werden ALLE Scam-Nachrichten derselben Welle
(gleichzeitig in mehreren Kanaelen gepostet, ~1 min) entfernt, nicht nur zwei. Als Beweis
wird GENAU EINE Nachricht vollstaendig gespiegelt (alle ihre Bilder), nicht ein ueber mehrere
Nachrichten gemischter Ausschnitt.

## Problem (Ist)
1. Unvollstaendiges Loeschen. Events werden sequenziell abgearbeitet
   (`moderation_system.rs` `spawn()`, eine Loop ueber `events.recv()`). Ablauf einer Welle,
   die gleichzeitig in N Kanaelen postet:
   - Nachricht 1 (Kanal A): `record_recent`, kein Trigger (nur 1 Bild-Kanal).
   - Nachricht 2 (Kanal B): Trigger `detect_takeover` (>= TAKEOVER_IMAGE_CHANNELS=2). Geloescht
     werden die bis hier erfassten Nachrichten. Danach `history.remove` + `suppress_user`
     (CASE_COOLDOWN_SECONDS=600).
   - Nachrichten 3..N (Kanaele C..): waren zum Trigger-Zeitpunkt bereits gepostet und liegen
     in der Broadcast-Queue. Bei ihrer Verarbeitung bricht `detect()` an `is_suppressed` mit
     `None` ab -> KEIN Delete. Sie bleiben stehen.
   Effekt: es werden faktisch immer nur ~2 Nachrichten geloescht, obwohl die ganze Welle schon
   zum Trigger-Zeitpunkt gepostet war. Code-Bug, kein fixer Loesch-Zaehler. Das Konto ist nach
   dem Treffer im Timeout und postet nicht neu; es geht ausschliesslich um die schon
   vorhandenen, gleichzeitig geposteten Nachrichten.
2. Beweis-Spiegelung mischt statt komplett. `EVIDENCE_IMAGE_LIMIT=4`
   (behavior_detector.rs:29) sammelt die ersten 4 Bilder ueber ALLE Wellennachrichten hinweg,
   `MODERATION_EVIDENCE_IMAGE_LIMIT=4` (modglue.rs:32) kappt den Download auf 4. Bei 2
   Nachrichten a 4 Bildern entsteht ein 4er-Mix, keine vollstaendige Nachricht.

## REQ
- REQ1: Bereits geposteten Nachrichten derselben Welle, die nach dem Trigger noch in der
  Verarbeitungs-Queue liegen, werden geloescht, obwohl das Konto bereits einen Case ausgeloest
  hat und im Cooldown steht. Es wird KEIN zweiter Moderations-Case gepostet und KEIN zweiter
  Timeout/Ban gesetzt: nur geloescht.
- REQ2: Beim Takeover-Treffer selbst werden alle im relevanten Fenster erfassten Nachrichten
  des Kontos ueber alle Kanaele geloescht, nicht nur das enge 30-s-Fenster.
- REQ3: Als Beweis wird genau EINE Nachricht vollstaendig gespiegelt (alle Bilder dieser einen
  Nachricht, bis zur Discord-Grenze von 10 Anhaengen), nicht ein ueber mehrere Nachrichten
  gemischter 4er-Ausschnitt. Da die Welle dieselben Bilder wiederholt, genuegt eine komplette
  Nachricht. Beide bisherigen 4er-Deckel entsprechend anpassen.

## INV (unveraenderlich)
- INV1: Pro Konto pro Cooldown hoechstens EIN Review-Case und hoechstens EIN Timeout/Ban. Der
  Cooldown bleibt fuer Cases und Sanktionen wirksam; nur das Loeschen der Restwelle ist
  davon ausgenommen.
- INV2: Staff/Admin/Manage-Messages-Konten werden nie moderiert (bestehende Gates bleiben).
- INV3: Shadow-Modus (`config.enforce == false`) loescht weiterhin nichts.
- INV4: Keine Verhaltensaenderung fuer Nicht-Takeover-Trigger (burst_rate, keyword etc.),
  ausser der geaenderten Bild-Spiegelung.
- INV5: Ein Konto, das nach Ablauf des Cooldowns erneut auffaellt, wird wieder normal als
  neuer Case behandelt (keine Dauer-Unterdrueckung).

## Nicht-Ziele
- Kein Umbau des LLM-Richters oder der Policy-Entscheidung.
- Keine neue DB-Struktur, kein neuer OAuth/Token-Pfad.
- Keine Aenderung an Timeout-/Ban-Dauer.

## Erlaubter Bereich
- `rust/crates/dl-moderation/src/behavior_detector.rs`
- `rust/crates/dl-moderation/src/moderation_system.rs`
- `rust/bin/dl-bot/src/modglue.rs` (nur Bild-Deckel-Konstante)
- Tests in denselben Dateien.

## Amendments
