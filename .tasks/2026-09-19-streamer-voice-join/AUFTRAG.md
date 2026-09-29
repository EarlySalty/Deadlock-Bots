# Mitspielen: direkt zum Sprachkanal des Streamers

## Nutzerwunsch

Bei einer Mitspiel-Anfrage im Twitch-Chat direkt zum aktuellen Sprachkanal des Streamers auf dem Community-Discord einladen. Ist dieser voll (beispielsweise 8/8), genau einen freien Platz schaffen. Niemanden entfernen und keine Zugriffsrechte erweitern.

## Umsetzung

- Twitch: eigener Service-Antwortpfad vor Spielzugangs- und Werbe-Pitches. Bestehenden LFG-Judge und Chatverlauf wiederverwenden. Bestehende Discord-Mitglieder und Moderatoren dürfen Hilfe erhalten; keine Werbe-Zielgruppenprüfung für diese angeforderte Hilfe.
- Identität aus Twitch-Broadcaster-ID und vorhandener Partner/Discord-Verknüpfung, niemals Namensähnlichkeit.
- Discord: bestehender authentifizierter Master-Broker, aktuelle Gateway-Präsenz und REST-Kanalzustand. Nur bestehende öffentliche Community-Spielkanäle, keine gesperrten, privaten, AFK- oder Stage-Kanäle.
- Kanalspezifischer zeitlich begrenzter Invite. Bei vollem Kanal Limit auf aktuelle Belegung plus eins setzen, sonst Limit unverändert. Maximal 99. Änderung bleibt bis zu einer normalen Kanaländerung bestehen; keine unangekündigte automatische Rücksetzung.
- Serialisierte Änderung, idempotente Nachrichten, Abklingzeiten und erneute Prüfung bei Kanalwechsel. Keine Secrets, neuen Modelle oder ENV-Konfigurationen.
- Fehler oder fehlender Voice: kein erfundener Voice-Link und keine Behauptung, ein Platz sei frei.

## Arbeitsbäume

Twitch: `/home/nathanael/.worktrees/tb-streamer-voice-join`
Discord: `/home/nathanael/.worktrees/db-streamer-voice-join`
Beide: `feat/streamer-voice-join`, getrennt von fremden Änderungen.
