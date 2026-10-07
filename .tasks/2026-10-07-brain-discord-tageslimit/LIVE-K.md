# K: Tagesquotenrelease aktiviert, Antwortprobe offen

status: aktiv, 7. Oktober 2026

## Stand und Herkunft

Eigener Worktree bots-k-live-20261007, Branch feat/bots-k-live-20261007. Frischer Fetch und erneuter Remote-main-Abgleich vor abschließender Aktivierung: 0fb873c6887c6ec8df6ce50d15c8ded9781fbadf. Bestehender Sourcegate 0758b1f2 ALLOW und frühere 81 scoped Fälle gehören zum identischen übernommenen Code. Diese Sitzung hat keine Bots-Source geändert und keinen neuen Bots-Releasebau behauptet.

Retained dl-bot tatsächlich unabhängig geprüft: 79301128 Bytes, SHA256 700d9ab5f4b9695de0891f793bed2e70f12862b06bdd344146f31e9037532fee, Anker brain_daily_user_limit vorhanden. Bekannter früherer Build 7m14s, Exit 0. Retention unverändert erhalten.

Git-Archiv des beauftragten SHA in privater Stage, dl-bot aus Retention; sieben Begleitbinaries gegen installierte SHA256SUMS geprüft und unverändert übernommen. dl-web bleibt der Build e1f11614, übrige sechs bleiben 600b832a laut ursprünglicher Provenance. BUILD-PROVENANCE.toml im neuen Release führt die gemischte Herkunft ausdrücklich auf. Kein frischer dl-web-/Workspacebau behauptet. Geprüfter Diff seit e1f11614: Discord-Consumer/Quota/Brain-API, optionales Quotenfeld in dl-core und Concierge-Text. dl-web hat keinen direkten dl-brain/dl-community-Eingang. Feeder/Verbinder verwenden dl-brain, bekommen hier aber keine neue Consumerabnahme und wurden nicht neu gestartet.

## Tatsächliche Lieferung und Korrekturen

Bestehender bestätigter Weg: Stage nach /opt/deadlock/bots/releases/<SHA>, root:root, go-w, temporärer Symlink, atomarer mv -T, bot-restart dl-bot web. FD3-Launcher und Units unverändert.

Erster Hashprüfversuch stoppte vor Änderungen wegen falschem CWD für relative Manifestpfade. Im korrigierten Stage-CWD acht Hashes OK. Erste Aktivierung danach erfolgt; beide Dienste scheiterten mit 200/CHDIR, weil die Stagewurzel nach root-Eigentümerwechsel noch Modus 700 hatte. Auf bisherigen Release-Modus 755 korrigiert, ohne Gruppen-/Fremdschreibrechte, danach beide Restarts erfolgreich. Anfangs NRestarts 5, Fehlerjournal seit erster Aktivierung nicht leer.

Erster Sperraufruf war außerdem falsch: flock --exclusive 9 mit Folgekommando interpretierte 9 als Dateinamen. Es entstand eine eigene leere Datei 9 im Brain-Worktree, keine echte gemeinsame Deploysperre. Nach 0-Byte-Prüfung entfernt. Korrigiert durch flock --exclusive 9 ohne Folgekommando innerhalb der Shell, mit bereits offenem FD9 auf /run/lock/deploy-deadlock-bots-release.lock. Unter dieser tatsächlichen exklusiven bestehenden Sperre Remote-main, current und acht Releasehashes erneut geprüft, denselben aktuellen Release atomar reaktiviert und beide Dienste regulär gestartet. Abschließende Aktivierung 2026-10-07T20:53:25.057Z. Kein fremder Lock gelöscht, kein Hook geändert oder umgangen.

## Abschließender technischer Nachweis

current: releases/0fb873c6887c6ec8df6ce50d15c8ded9781fbadf.

- Tatsächlicher dl-bot PID 2848890 zu 2766584. exe im neuen Release ohne deleted, laufender SHA256 exakt 700d9ab5..., Inhaltsanker vorhanden. Launcher-MainPID 2766542 ist nicht der Anwendungsprozess.
- Tatsächlicher dl-web PID 2848935 zu 2766645. exe im neuen Release ohne deleted, unveränderter SHA256 df3e4463... exakt. Launcher-MainPID 2766600.
- Beide active/running, NRestarts nach abschließendem regulärem Restart 0. Kein reset-failed verwendet. Beide Fehlerjournale seit abschließender Aktivierung 0 Datensätze, Journal-Exit 0. Anfänglicher Fehler oben erhalten.
- http://127.0.0.1:8768/ HTTP 200, text/html; charset=utf-8, inhaltlicher Stats/Statistik/Deadlock-Anker bestätigt. Unveränderter Webpfad funktioniert.
- Seit abschließender Aktivierung 0 Marker Discord-Brain-Antwort empfangen. Keine echte Anfrage erzeugt, keine Nutzerprobe positiv behauptet. Kein Secret gelesen, kein Testkonto gesucht, keine privaten Journaloriginale in Git. Tageszähler bleibt prozesslokal; Zustellhinweis-NIT bleibt offen.

LIVEBEWEIS[DV-1]: PID 2848890->2766584 | exe ohne (deleted) | journal -p err seit abschließender Aktivierung leer, Anfangsfehler dokumentiert | Anker "brain_daily_user_limit" in Binary | Funktion: technischer Quotenrelease aktiv, echte Antwort- und Nutzerprobe offen | Ort: https://discord.com/channels/1289721245281292288/1426220702054355077 #frag-die-community

MERGEPROTOKOLL[MS-1]: 0 Git-Schritte für neuen Source-Merge | Anläufe: 0 | Gate: bestehender Source0758b1f2 ALLOW, kein neuer Source-Diff
