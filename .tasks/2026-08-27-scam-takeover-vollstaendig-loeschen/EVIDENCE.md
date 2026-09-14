# Evidence: Takeover-Scam Loesch- und Spiegel-Bug

## Loeschen: nur ~2 Nachrichten
- moderation_system.rs:864-873: `spawn()` verarbeitet Events sequenziell in einer Loop
  (`events.recv().await`), keine Parallelitaet.
- moderation_system.rs:182-186: `handle_message` ruft `detector.detect(guild_id, event)`; bei
  `None` kein behavior_signal.
- behavior_detector.rs:203-206: `detect()` bricht bei `is_suppressed` sofort mit `None` ab, VOR
  `record_recent`. Unterdrueckte Nachrichten werden nie erfasst und nie geloescht.
- behavior_detector.rs:214-217: nach einem Treffer `history.remove(author)` plus
  `suppress_user(author)` (Cooldown CASE_COOLDOWN_SECONDS=600, Zeile 25).
- behavior_detector.rs:594-621: `detect_takeover` liefert nur das 30-s-Fenster (`window`,
  TAKEOVER_WINDOW_SECONDS=30). Als Delete-Ziele dienen genau diese.
- moderation_system.rs:355-375: `auto_delete_targets(event, signal)` speist die Delete-Loop;
  `delete:{deleted_count}/{len}` im Embed. Zahl stimmt, Menge ist zu klein.
- moderation_system.rs:741-763: `auto_delete_targets` nimmt `signal.messages` (das
  30-s-Fenster) plus die Event-Nachricht.

## Spiegeln: 4er-Mix statt einer kompletten Nachricht
- behavior_detector.rs:29: `EVIDENCE_IMAGE_LIMIT = 4`.
- behavior_detector.rs:510-517: `build_evidence` flacht die Bilder ALLER Wellennachrichten
  zusammen und kappt via `.take(EVIDENCE_IMAGE_LIMIT)`, ergibt einen Mix.
- moderation_system.rs:765-783: `evidence_image_urls` mischt `signal.evidence.image_urls` plus
  `event.image_attachment_urls`, dedupe.
- moderation_system.rs:343-347: `mirror_evidence_images(...)` bekommt diese gemischte Liste;
  `evidence_file_count` landet im Embed ("N Bild(er) als Anhang gespiegelt").
- modglue.rs:32: `MODERATION_EVIDENCE_IMAGE_LIMIT = 4`.
- modglue.rs:930-971: `mirror_evidence_images` laedt max `MODERATION_EVIDENCE_IMAGE_LIMIT`
  Bilder (`.take(...)`), plus Groessen- und Timeout-Filter.

## Live-Beleg
- Case-Embed (Screenshots): Signale `images=8 attachments=8`, Ausgefuehrt `delete:2/2`,
  "4 Bild(er) als Anhang gespiegelt". Konto im Timeout. Weitere identische Scam-Nachrichten
  desselben Kontos stehen im Kanal weiter sichtbar.

## Bestehende Tests (Baseline)
- moderation_system.rs:1495:
  `takeover_signal_creates_one_case_embed_and_deletes_all_signal_messages`.
- behavior_detector.rs:872: `detector_returns_takeover_signal_once_and_suppresses_followups`
  (Verhalten aendert sich: Follow-ups derselben Welle sollen geloescht werden, ohne neuen Case,
  Test anpassen).
- behavior_detector.rs:794: `takeover_detection_requires_images_in_two_channels_within_30s`.
