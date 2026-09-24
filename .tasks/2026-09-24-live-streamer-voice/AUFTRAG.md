# Live-Streamer als temporäre Voice-Mitverwalter

## Auftrag
Mitglieder mit der bestehenden Streamer-Rolle sollen während eines bestätigten Twitch-Livestreams, unabhängig vom Spiel, im aktuell betretenen verwalteten Voice-Channel dieselben Panel-Aktionen wie dessen Owner nutzen können. Der Owner bleibt vor Kick und Ban durch diese zusätzliche Berechtigung geschützt.

## Vertrag
Die Freigabe gilt nur in bekannten TempVoice-Lanes der konfigurierten Guild, nicht in festen Channels oder Einstiegskanälen. Die vorhandene STREAMER_ROLE_ID-Konfiguration des Matchers wird wiederverwendet. Rollen werden bei jeder Berechtigungsprüfung per Discord REST gelesen; Bot-Accounts sind ausgeschlossen. Die verknüpfte Twitch-Identität und der Live-Status kommen aus dem bestehenden authentifizierten Diagnose-Endpunkt. Ein bestätigter Live-Tick muss höchstens 180 Sekunden alt sein. Fehlende, ungültige, zukünftige oder veraltete Zeitstempel sowie API-Fehler, Zeitüberschreitungen und ein nicht gesunder Voice-Cache erteilen keine Freigabe. Der gesamte externe Check ist auf zwei Sekunden begrenzt.

Es werden keine nativen Discord-Moderationsrechte oder dauerhaften Rollen vergeben. Der echte Owner bleibt unverändert; bestehende Owner- und Moderatorrechte bleiben bestehen. Persönliche Voreinstellungen und das reguläre Owner-Claim-Verfahren werden nicht automatisch zu Eigentumsübertragungen.

Mehrstufige Kick-/Ban-/Unban-Menüs sind an die ursprüngliche Lane und deren Owner gebunden. Bei Auswahl wird erneut autorisiert. Nach Stream-Ende, Rollenverlust, Voice-Verlassen oder Owner-Wechsel darf eine alte Auswahl keine fremde Lane verändern. Stream-Ende wird entsprechend dem vorhandenen Twitch-Polling erkannt, nicht synchron zum tatsächlichen Stream-Ende.

Der Owner wird im Kick-Menü ausgeblendet und auf der Serverseite vor Kick und Ban geschützt. Unveränderte echte Moderatorrechte sind davon getrennt. Kick und das sofortige Trennen beim Ban sind auf Mitglieder im selben Call beschränkt. Die vorhandene serverweite Mitgliedersuche beim Ban bleibt für bereits gegangene Störer erhalten, trennt aber niemanden aus einem anderen Call.

## Persistenz und Umfang der Sperren
Wie bei bisherigen Owner-Aktionen wird die bestehende Owner-Banliste verwendet. Ein Streamer-Ban ist daher keine nur bis zum Stream-Ende gültige Sperre: Er gilt für die Lanes dieses Owners, bis er aufgehoben wird. Das Panel und die Bestätigung benennen diesen Umfang. Ungebundene persönliche Alt-Menüs dürfen nicht durch einen späteren Streamstart stillschweigend auf eine fremde Owner-Liste wechseln. Keine neue Datenbanktabelle oder Migration.

## Zusammenspiel mit Twitch-Bot
Der ergänzende Branch `EarlySalty/Deadlock-Twitch-Bot:feat/live-streamer-status-freshness-20260924` stellt `last_seen_at` im bestehenden Diagnose-Endpunkt bereit. Bei später freigegebenem Rollout zuerst den Twitch-Endpunkt, anschließend diesen Bot-Stand ausliefern. Gegen einen alten Endpunkt ohne Zeitstempel bleibt die neue Berechtigung sicher gesperrt; vorhandene Owner-Rechte funktionieren weiter.

## Auslieferungsgrenze
PR-first-Testbetrieb: als offener Draft-PR sichern, keine Merge-Freigabe, kein Deploy, kein Produktionsdatenbankzugriff, keine Community-Nachrichten und kein Dienstneustart. Die vorhandene PR-Release-Automation schließt Drafts explizit aus. Keine Workflow- oder Hook-Änderung.
