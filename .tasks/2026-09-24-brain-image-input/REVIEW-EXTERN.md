# Unabhängige Abnahme: Brain Bild-Eingabe

Reviewer: unabhängiger Thread `cc670e0a-f1c3-42af-8aa1-633bf3302a92`, Modell GLM 5.3 Flash, 24.09.2026, finale Delta-Fassung nach Round6.

Zuordnung durch die Hauptsession nach Abschluss des Reviews: Der nachstehend geprüfte Quellbaum wurde unverändert als `ad5859a8e7600ff7df0cfdfbbbe8a6ee4b39a340` committed und gepusht. Die alten Angaben zu noch uncommittetem HEAD und fehlender STATUS.md beschreiben Zwischenstände während der Prüfung; STATUS.md liegt inzwischen vor. Das korrekte Verzeichnis des vollständigen grünen Laufs ist `/tmp/brain-image-verify-20260924-round6`; die Verzeichnisangabe unten wurde redaktionell berichtigt. Urteil und fachliche Befunde des Reviewers bleiben unverändert.
Geprüfter Stand: Working Tree von `feat/brain-image-input-20260924`, HEAD = Basis `ff635f7b354cb09909c01ddd6f773d0682dd89c9`, alle Änderungen uncommitted. Diff: 7 Dateien (743+/59−) plus neue Dateien `rust/crates/dl-ai/src/discord_image.rs` (287 Zeilen) und `docs/brain-image-input.md`. Erstvollnahme im Round2-Stand, Delta-Prüfung der/neuen Stellen transparency.rs, Chunked-Test, expect-Ersatz, sort_by_key-Fix. Nur gelesen, nichts geändert, kein eigener Build, kein Merge, kein Deploy.

## Urteil

**ALLOW**, eingeschlossen der neuen Datenschutz-Ausblendung im Transparenzprotokoll. Keine blockierenden Befunde. Freigabe gilt für den Baum ab modglue-Stand 04:21:38 (sort_by_key-Fix); genau dieser Stand ist in Round6 vollständig gegrün (siehe Teststand). Die Hinweise unten sind ohne Fixbedarf.

## Prüfschwerpunkte

**Command und Auflösung.** `brain` bekommt die optionale Attachment-Option `bild` (modglue.rs:842-850). `flatten_resolved_command` (dispatch.rs:72-101) löst Attachment-Optionen deterministisch über `resolved.attachments` auf; ohne resolved-Eintrag bleibt der Schlüssel mit `null` vorhanden statt zu verschwinden (Test `unresolved_attachment_is_present_but_invalid_not_silently_omitted`). Die Route nach `null` endet in einer Ephemeral-Fehlermeldung (modglue.rs:630-656), nicht in einer bildlosen Antwort.

**Validierung vor kostenpflichtiger Analyse.** `brain_image_urls` (modglue.rs:630-656) prüft deklarierten Content-Type und Größe über `validate_attachment`; `discord_image.rs` begrenzt Hosts auf `cdn.discordapp.com`/`media.discordapp.net`, verlangt HTTPS, Port 443, ohne userinfo, ohne Fragment, Pfad `/attachments/<Ziffern>/<Ziffern>/<Datei>` mit `%2f`/`%5c`-Ausschluss (Zeilen 24-56). Beim Download: Redirect-Policy `none` plus 302-Test, kein Proxy, Content-Length-Vorprüfung, Streaming-Obergrenze 8 MiB auch ohne Content-Length, Magic-Byte-Prüfung für PNG/JPEG/WebP, 10-s-Timeout. Kein Dateischreiben, keine Logausgabe von URL oder Bildinhalt auf dem neuen Pfad.

**Keine stille Verwerfung.** Trait-Default `answer_with_images` (dl-brain/lib.rs:43-52) antwortet mit Fehler, wenn ein Answerer Bilder nicht unterstützt; fehlender Vision-Client ergibt expliziten Fehler (modglue.rs:410-414); leeres oder fehlgeschlagenes Vision-Resultat ergibt eigene Fehlermeldungen. Bildfehler verbrauchen keinen Cooldown (dl-brain/lib.rs:99-111, Test `unsupported_images_are_not_ignored_and_do_not_consume_cooldown`).

**Trennung von Intent, Kontext und Publish.** Die Frage bleibt im Payload unverändert, der Bildkontext liegt als separates Feld `image_context` vor; im Grounded-Pfad ist das Bild ein eigener Beleg `Source::UserImage` mit `kind: user_image` (dl-answer/lib.rs:352-362) und kein kanonischer Spielbeleg; Scope bleibt `GameOnly`. `IMAGE_CONTEXT_SYSTEM` (dl-answer/lib.rs:14) verlangt Beobachtungs statt Befehlsbehandlung. Der Bildpfad erreicht den Build-Publish-Code nicht; Build-Fragen werden mit „Bildberatung: kein In-Game-Build veröffentlicht.“ markiert (modglue.rs:331-365, Test `brain_image_context_cannot_trigger_build_publication` mit befehlender Bildbeobachtung). Bildkontext separat auf 2200 Zeichen begrenzt (`MAX_IMAGE_CONTEXT_CHARS`, dl-answer/lib.rs:13, 16-23), Eingangs- und Ausgabefilter über `contains_sensitive_material` in beiden Antwortpfaden.

**Fehler, Timeout, Admission.** Fein granulare deutsche Fehlermeldungen je Fehlerklasse (modglue.rs:404-453). Gesamt-Timeout 100 s, Download 10 s, Vision 45 s; der AnswerEngine nutzt `concierge_config.ai_timeout` (main.rs:999), die 20 s in main.rs:996 gehören zum `CommunityRetriever` und betreffen nur die Wissenssuche. Semaphor-Admission mit 2 Plätzen über `try_acquire` mit eigener Auslastungsmeldung (modglue.rs:420-426). Cooldown-Prüfung läuft vor dem Bildpfad und bleibt wirksam. Die Discord-Antwortzeit ist durch das bestehende Defer nach 2 s mit Followup abgesichert (dispatch.rs:527-556).

**Kein Modellwechsel.** Bildzusammenfassung nutzt den vorhandenen Moderations-Vision-Client (`moderation_image_analyze_client`, main.rs:932, verdrahtet in main.rs:1060-1064, Modell `MOD_IMAGE_ANALYZE_MODEL`); die Antwort nutzt die bestehende AnswerEngine-Verdrahtung über den Concierge-Provider. Data-URIs werden vom vorhandenen Vision-Code ohne Zweitdownload durchgereicht (dl-ai/lib.rs:268-271).

**Textpfad unverändert.** Nachrichtenpfad übergibt leere Bildliste (modglue.rs:549-567), `handle_brain_query` delegiert mit leerer Liste, `answer()` ist unverändert; Test `text_only_open_answer_has_no_image_payload` bestätigt den Payload ohne Bildfeld.

## Delta-Befunde der finalen Fassung

**Datenschutzpfad transparency.rs.** `TransparencyProvider::chat` erkennt eine Bildfrage daran, dass eine User-Nachricht als JSON mit String-Feld `image_context` parses (transparency.rs:205-214), und blendet vor dem Protokollieren Promptauszug, Systemauszug, Gesprächsspur und Antwort inhaltlich aus („Brain-Bildfrage (Inhalt nicht protokolliert)“, transparency.rs:255-263). Modell, Dauer und Erfolgs- beziehungsweise Fehlerstatus bleiben erhalten. Der Wrap greift für alle Provider aus der zentralen Fabrik (`wrap_with_transparency`, chat_provider.rs:489), über die auch der Brain-Answer-Provider läuft; nur der Bildpfad setzt `image_context`, der Textpfad bleibt ungetrübt (Gegenprobe im Test `text_only_open_answer_has_no_image_payload`). Eigener Test `brain_image_context_and_answer_do_not_enter_content_log` (transparency.rs:533-558) zeigt: Antwortinhalt kommt unverändert beim Nutzer an, kein `PRIVATE_IMAGE`-String im Protokollsatz, Modellfeld bleibt gesetzt. Der Fehlerstring bei Fehlern enthält Provider-/HTTP-Diagnose, keine Bildinhalte. Kein Befund.

**Chunked-Test deterministisch.** `chunked_response_is_bounded_without_content_length` (discord_image.rs:256-290) liest die Anfrage des Clients Byte für Byte bis zum Kopfende ein, bevor die gechunkte Antwort geschrieben wird; der Größen-Obergrenzentest ist damit reihenfolgensicher. Semantik unverändert: 9 MiB ohne Content-Length enden in `ImageError::TooLarge`.

**expect statt unwrap.** Neue Test-unwraps sind durch aussagekräftige `expect`-Meldungen ersetzt (unter anderem modglue-Test „gültiger Bildfrage-Vertrag“, discord_image „lokaler Testport“, „Chunked-Testantwort“, transparency „unveränderte Bildantwort“); produktiver Code ist nicht betroffen.

**Clippy-Fund aus Round5 behoben.** Round5-Clippy (`-D warnings`) meldete `unnecessary_sort_by` an modglue.rs:94 an der Emoji-Index-Sortierung; die Zeile stammt aus der Basis, der Diff fasst die Funktion aber an. Korrigiert auf `entries.sort_by_key(|entry| std::cmp::Reverse(entry.0.len()))` (modglue.rs:94). Absteigende, stabile Sortierung bleibt identisch, die Deko-Reihenfolge ist unverändert. Round6 (dieser Stand) ist inklusive Clippy und rustfmt-Check grün.

**Doku.** `docs/brain-image-input.md` beschreibt Bedienung, Grenzen, Limits und Datenschutzausblendung sachgerecht und macht keine Live-Aussagen.

## Korrekturen gegenüber der Erstfassung

1. Timeout-Zuordnung berichtigt: Die Antwortmaschine erhält `concierge_config.ai_timeout` (main.rs:999). Die 20 s in main.rs:996 sind der Timeout des `CommunityRetriever` für die Wissenssuche; die Erstfassung hatte das fälschlich als Antwortmaschinen-Timeout zitiert.
2. `truncate_brain_chars` (modglue.rs:764-783) precise Beschreibung: Die Begrenzung arbeitet in UTF-16-Einheiten (Guard über `encode_utf16().count()`, je Zeichen `len_utf16()`), die Schleife läuft zeichenweise und schneidet an Zeichengrenzen. `validate_image_context` zählt Zeichen (dl-answer/lib.rs:18). Die Richtung bleibt sicher: UTF-16-Zählung ist nie kleiner als die Zeichenzahl, ein gekürzter Kontext überschreitet die Zeichengrenze daher nicht. Abweichend zur Korrekturmitteilung („verwendet chars(), nicht UTF-16“) messe ich im aktuellen Stand weiterhin UTF-16-Einheiten als Zähleinheit; die Aussage in der Erstfassung bleibt danach inhaltlich bestehen, nur die Formulierung ist hier präzisiert.

## Nicht blockierende Hinweise

1. `image_retrieval_query` schneidet Frage plus Bildbeobachtung auf 4000 Zeichen (dl-answer/lib.rs:26-31). Bei maximal langen Fragen kann die Beobachtung aus dem Retrieval-Query herausfallen; Payload-Frage und `image_context` bleiben vollständig. Akzeptable Degradation.
2. `looks_like_build_request` ist eine Schlagwortprüfung (modglue.rs:242-247). Ungewöhnlich formulierte Build-Fragen erhalten den Klartext-Hinweis zur Nichtveröffentlichung eventuell nicht; veröffentlicht wird über den Bildpfad trotzdem nie. Rein darstellerisch.
3. `flatten_resolved_command` überschreibt Attachment-Werte für alle Slash-Commands; derzeit hat nur `brain` eine Typ-11-Option, kein anderes Command ist betroffen.
4. docs/brain-image-input.md verweist für Details auf `.tasks/2026-09-24-brain-image-input/STATUS.md`; diese Datei existiert im Worktree bisher nicht (nur AUFTRAG, REGISTER, REVIEW-AUFTRAG und dieses Dokument). Für die Hauptsession zur Berichtslage zu prüfen.

## Teststand

Round6, `/tmp/brain-image-verify-20260924-round6` (Verzeichnisname redaktionell berichtigt) mit Abschluss 04:23, Exit 0 für alle vier Einheiten auf dem freigegebenen Stand: `cargo +1.97.1 test --locked -j 2 -p dl-ai -p dl-brain -p dl-answer -p dl-discord --lib` mit 126 + 32 + 8 + 51 = 217 Tests, `cargo +1.97.1 test --locked -j 2 -p dl-bot modglue::tests::brain_` mit 17 Tests, `cargo +1.97.1 clippy --locked -j 2 -p dl-ai -p dl-brain -p dl-answer -p dl-discord -p dl-bot --all-targets --no-deps -- -D warnings` und rustfmt `--check` über alle acht Diff-Dateien, jeweils Exit 0. Verlauf: Round4 Bibliotheks- und Brain-Tests grün, Clippy fand neue Test-unwraps; Round5 mit den expect-Ersätzen grün in Tests, Clippy blieb am oben genannten sort_by hängen; der Fix daraufhin folgte um 04:21:38, Round6 belegt ihn.

Nicht von mir geprüft oder behauptet: Live-Discord- oder Live-Vision-Durchläufe, PR-CI, Aussagen über Speicherung beim Modellanbieter. Eigener Build wurde gemäß Auftrag nicht ausgeführt.

## Außerhalb dieses Reviews offen

- Fertigkriterium 6 des AUFTRAGs (Diagnose „Mein Hirn hakt grad“ bei Mo-&-Krill-Buildwunsch) ist Bestandteil keines Diffs dieses Pakets und bleibt von dieser Abnahme unberührt.
- STATUS.md zum Implementierungsstand fehlt (siehe Hinweis 4).
