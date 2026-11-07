# Invite-Lounge: Brain schreibt und stellt Status und Nachfrage zu

## Ziel

Wer in `#deadlock-invite` (1464736918951432222) einen Steam-Freundescode postet, bekommt vom Deadlock Brain eine Antwort auf seinen Beitrag mit Erwähnung des Verfassers. Der Bot postet diese Meldungen nicht mehr selbst. Das Brain schreibt den Text über seinen zentralen Provider und stellt über den dl-bot Master-Broker zu (`127.0.0.1:8770`, `/internal/master/v1/discord/send-message` bzw. `/send-dm`, Header `X-Internal-Token` und `X-Idempotency-Key`).

## Aufteilung

- Bot (`dl-community/src/invite_lounge*.rs`): hält den Zustand in Postgres (`bot.kv_store`, ns `invite_lounge`) und eine dauerhafte Ausgangsliste je Anfrage und Zustand. Übergibt dem Brain alle Fakten: Status, Verfasser, Kanal, Anfrage-Nachricht, Ziel, Idempotenzschlüssel, Steam-Ergebnis, bereits eingeladen, Antworttext bei der Nachfrage.
- Brain (`brain-api`, `brain-serve/src/discord_delivery.rs`): Text über den Provider, Zustellung über den Broker. Kein Discord-Token, kein Zugriff auf Bot-Postgres.
- Broker (`dl-broker`, `dl-discord`): `send-message` kann mit `reply_to_message_id` und `allowed_user_ids` antworten und nur den Verfasser erwähnen.

## Regeln

- Ziel: Antwort im Anfragekanal mit `allowed_mentions.users=[Verfasser]`. DM nur ohne Kanal.
- Nachfrage nach 30 Minuten nur in `#deadlock-invite`, nur ohne echte Antwort, nie bei vermerkter Einladung.
- Eine Meldung je Zustandswechsel. Fehler bei Brain oder Broker lassen die Meldung liegen, der nächste Versuch sendet genau einmal.
- Keine Persona-Vorlagen, kein Fallback-Text, keine Wortlisten.
- Keine Live-Aktion, kein Deploy. Alte Posts von fidelisfuchs, Patrick und S nicht anfassen.
