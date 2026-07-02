# Twitch-Streamer-Dashboard

## Worum geht es?

Der Twitch-Bot hat mehrere Streamer-Surfaces: eine Startseite mit Status und Schnellaktionen, ein Analytics-Dashboard, eine Verwaltungsseite für OAuth/Discord/Steam, eine Billing-Seite, ein Affiliate-Portal und den Go-Live-Builder für Discord. Alles dreht sich darum, dass du deinen Kanal autorisieren, Raids automatisieren, Analytics lesen und optionale Premium-Funktionen direkt selbst steuern kannst.

## Wie läuft der Login?

Der normale Einstieg ist der Twitch-Login. Danach landet dein Account in der Streamer-Ansicht und der Bot weiß, zu welchem Kanal die Session gehört. Wenn wichtige Twitch-Scopes fehlen oder ein Token neu autorisiert werden muss, zeigt die Verwaltungsseite das direkt an und bietet einen Reconnect-Link an. Dort siehst du auch:

- aktive Twitch-Scopes
- fehlende Scopes
- den Status der Discord-Verbindung
- die Steam-Verknüpfung
- deinen Twitch-Login und Anzeigenamen

Ohne gültige Twitch-Autorisierung bleiben besonders Raid- und einige Analyse-Funktionen eingeschränkt. Die Verwaltungsseite ist deshalb der zentrale Ort für Re-Auth.

## Was zeigt das Dashboard?

Die Startseite zeigt den aktuellen Betriebszustand deines Setups: Twitch-OAuth verbunden oder nicht, Discord verbunden oder nicht, Raid-Status, letzte Aktionen und Warnungen, Health-Score, Wochenvergleich und letzte Streams.

Das Analyse-Dashboard hat diese Tabs: `Übersicht`, `Streams`, `Publikum`, `Wachstum`, `Planung`, `Was tun?` und `Monetization`. Von der Startseite kommst du außerdem zur Verwaltung, zum Pricing und zum Affiliate-Bereich. Admins können den Streamer-Kontext wechseln, normale Streamer sehen nur den eigenen Account.

## Free vs. Paid: wo sind die echten Cutoffs?

Die Tab-Freischaltung hängt fast komplett am `analytics`-Entitlement: Ohne Analyse-Plan sind `Publikum`, `Was tun?` und `Monetization` gesperrt — `Übersicht`, `Streams`, `Wachstum` und `Planung` sind frei.

Der aktuelle Katalog (Monatspreise, netto):

| Plan | Preis/Monat | Was er freischaltet |
|---|---|---|
| `Raid Free` | 0,00 € | Auto-Raid-Grundfunktion + freie Dashboard-Tabs |
| `Werbefrei` | 1,99 € | Bot-Werbung im Chat aus — kein Analytics-Upgrade |
| `Raid Boost` | 1,99 € | Raid-Priorisierung + Lurker-Tax-Erinnerungen — ebenfalls kein Analytics-Upgrade |
| `Werbefrei + Raid Boost` | 3,49 € | beides kombiniert |
| `Analyse Dashboard` | 1,99 € | volle Analytics-Tabs + Lurker-Tax |
| `Werbefrei + Analyse` | 3,49 € | Analytics + Werbung aus |
| `Analyse + Raid Boost` | 3,49 € | Analytics + Raid-Priorisierung |
| `Alles drin` | 4,99 € | alle Plan-Familien kombiniert |

Merkregel: Nur Pläne mit „Analyse" im Namen schalten die gesperrten Tabs frei. Raid Boost und Werbefrei ändern am Dashboard nichts.

## Testphase, Stripe und Kündigung

Die Billing-Seite nutzt Stripe. Für den monatlichen Analyse-Plan gibt es aktuell eine 30-Tage-Testphase. Im UI gibt es auch eine Jahresoption; preislich rechnet der Katalog dabei derzeit keinen Rabatt.

Wichtig für Nutzer:

- Checkout, Rechnungsdaten und Rechnungen laufen über die Billing-Surface
- Kündigung ist jederzeit möglich
- der Zugang bleibt bis Periodenende aktiv
- nach der Kündigung bleiben Analytics-Daten noch für 30 Tage gespeichert

## Raids, Live-Status und Go-Live

Die Raid-Funktionen bauen auf Twitch-OAuth auf. Sobald dein Kanal korrekt autorisiert ist, kannst du Auto-Raids aktivieren und den Status sehen. Free deckt die Grundfunktion ab, `Raid Boost` verändert die Priorisierung im Netzwerk.

Für Live-Kommunikation gibt es zusätzlich einen Go-Live-Builder für Discord. Dort kannst du Content, Embed, Rolle und Button konfigurieren, eine Preview erzeugen und Testsendungen auslösen. Auch Silent-Modus und Tip-Hinweis-Opt-out stellst du dort selbst ein.

## Affiliate

Das Affiliate-Programm ist implementiert, aber nur für freigeschaltete Accounts aktiv. Wenn du freigeschaltet bist, zeigt dir das Portal deinen Referral-Link, Gesamt-Claims, Gesamt-Provision, Claims des laufenden Monats, ausstehende Auszahlung und die letzten Claims; Auszahlungen laufen über eine separate Stripe-Connect-Anbindung. Ohne Freischaltung zeigt das Portal genau das an — „noch kein Affiliate" — statt leere Daten vorzutäuschen.

## Grenzen und häufige Fragen

- Fehlende Scopes bremsen vor allem Raid- und Analyse-Funktionen. Reconnect immer über die Verwaltungsseite.
- Werbefrei und Raid Boost sind kein Ersatz für den Analyse-Plan — die gesperrten Tabs öffnen nur die Analyse-Pläne.
- Der Social-Media-Bereich ist ein eigener Admin-Bereich, kein Streamer-Self-Service.
