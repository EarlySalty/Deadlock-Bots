# Bidirektionaler Voice-Pair-Guard

## Ziel

Die Discord-Nutzer `887664726421671976` und `279971744964542464` dürfen sich in keinem Voice-Channel des Servers gleichzeitig aufhalten. Kanalbesitz, TempVoice-Owner-Bans und Unban-Aktionen dürfen diese Regel nicht aushebeln.

## Verhalten

- Sobald einer der beiden Nutzer einem Voice-Channel beitritt, erhält der andere in genau diesem Kanal ein explizites `CONNECT: deny`.
- Beim Verlassen wird die Sondersperre im alten Kanal entfernt.
- Bei einem Kanalwechsel wird zuerst der alte Kanal bereinigt und anschließend der neue Kanal geschützt.
- Die Regel gilt bidirektional und für alle Voice-Channels, nicht nur für TempVoice-Kanäle.
- Andere Voice-Channels bleiben für den jeweils gesperrten Nutzer unverändert nutzbar.
- Der Bot trennt oder verschiebt keinen Nutzer für diese Regel.

## Technischer Ansatz

Der bestehende serielle Rust-Voice-State-Subscriber verarbeitet Join, Leave und Move bereits zentral. Der Pair-Guard wird dort vor der TempVoice-spezifischen Kanalprüfung ausgeführt und nutzt die vorhandene Permission-Overwrite-Schnittstelle.

Die wirksame `CONNECT`-Entscheidung wird zentral aufgelöst:

1. Eine aktive Pair-Sperre erzwingt `deny`.
2. Ohne Pair-Sperre gilt weiterhin ein vorhandener TempVoice-Owner-Ban.
3. Ohne beide Gründe wird nur das vom Pair-Guard gesetzte `CONNECT`-Bit entfernt; andere Overwrite-Bits bleiben erhalten.

Owner-Wechsel und Unban-Aktionen müssen dieselbe Auflösung verwenden, damit sie eine aktive Pair-Sperre nicht löschen. Für die zwei festen Nutzer-IDs wird keine allgemeine Regel-Engine oder Konfiguration eingeführt.

## Fehlerbehandlung und Grenzen

Fehler beim Setzen oder Entfernen eines Overwrites werden mit Guild-, Kanal- und Nutzer-ID geloggt und beenden den Voice-State-Subscriber nicht. Ein späteres Voice-State-Ereignis versucht die gewünschte Berechtigung erneut herzustellen.

Discord meldet einen Join erst als Voice-State-Ereignis. Bei nahezu exakt gleichzeitigen Joins besteht deshalb ein kurzes Rennen, das ohne nachträglichen Disconnect technisch nicht vollständig geschlossen werden kann. Ein Disconnect ist ausdrücklich nicht Teil dieses Designs.

## Tests

Die bestehende Rust-Testinfrastruktur erhält fokussierte Tests für:

- beide Nutzer-Richtungen;
- Join in einen beliebigen, nicht verwalteten Voice-Channel;
- Leave und Move mit Bereinigung des alten Kanals;
- aktive Pair-Sperre trotz TempVoice-Owner-Unban;
- Erhalt eines normalen Owner-Bans nach Ende der Pair-Sperre;
- unverändertes Verhalten für unbeteiligte Nutzer.

Die Tests werden vor der Implementierung geschrieben und zunächst mit der erwarteten fehlenden Pair-Guard-Funktion fehlschlagen.

## Nicht im Scope

- Disconnect oder automatisches Verschieben;
- serverweiter Discord-Ban oder dauerhafte Voice-Sperre;
- frei konfigurierbare Nutzerpaare;
- Änderungen am nicht mehr live verwendeten Python-TempVoice.
