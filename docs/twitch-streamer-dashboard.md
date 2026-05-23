# Twitch-Streamer-Dashboard

## Worum geht es?

Der Twitch-Bot hat mehrere Streamer-Surfaces: eine Startseite mit Status und Schnellaktionen, ein Analytics-Dashboard, eine Verwaltungsseite fuer OAuth und Discord, eine Billing-Seite, ein Affiliate-Portal und den Go-Live-Builder fuer Discord. Alles dreht sich darum, dass du deinen Kanal autorisieren, Raids automatisieren, Analytics lesen und optionale Premium-Funktionen direkt selbst steuern kannst.

## Wie laeuft der Login?

Der normale Einstieg ist der Twitch-Login. Danach landet dein Account in der Streamer-Ansicht und der Bot weiss, zu welchem Kanal die Session gehoert. Wenn wichtige Twitch-Scopes fehlen oder ein Token neu autorisiert werden muss, zeigt die Verwaltungsseite das direkt an und bietet einen Reconnect-Link an. Dort siehst du auch:

- aktive Twitch-Scopes
- fehlende Scopes
- den Status der Discord-Verbindung
- deinen Twitch-Login und Anzeigenamen

Ohne gueltige Twitch-Autorisierung bleiben besonders Raid- und einige Analyse-Funktionen eingeschraenkt. Die Verwaltungsseite ist deshalb der zentrale Ort fuer Re-Auth.

## Was zeigt das Dashboard?

Die Startseite zeigt den aktuellen Betriebszustand deines Setups:

- Twitch-OAuth verbunden oder nicht
- Discord verbunden oder nicht
- Raid-Status
- letzte Aktionen und Warnungen
- Health-Score, Wochenvergleich und letzte Streams

Von dort kommst du weiter ins Analyse-Dashboard, zur Verwaltung, zum Pricing und zum Affiliate-Bereich. Admins koennen den Streamer-Kontext wechseln, normale Streamer sehen nur den eigenen Account.

## Free vs. Paid: wo sind die echten Cutoffs?

Der wichtigste Cutoff ist nicht nur "kostenlos oder bezahlt", sondern welcher Plan welche Entitlements freischaltet.

### Kostenlos

`Free` beziehungsweise `Raid Free` kostet `0,00 EUR` im aktuellen Katalog. Enthalten sind:

- Auto-Raid-Grundfunktion
- Dashboard-Startseite
- die Analytics-Tabs `Overview`, `Streams`, `Schedule` und `Category`

Wenn du nur Basis-Uebersichten und das Raid-Grundsetup brauchst, reicht Free.

### Basic / Raid-orientierte Plaene

`Raid Boost` kostet `3,99 EUR` pro Monat. Damit bekommst du:

- bevorzugte Platzierung im Raid-Netzwerk
- Sichtbarkeit auch bei Inaktivitaet
- Lurker-Tax-Erinnerungen
- zusaetzlich die Basic-Analytics wie Chat, Growth, Audience und Compare
- KI-Mini-Analysen

`Werbefrei` kostet ebenfalls `3,99 EUR`, schaltet aber vor allem Bot-Werbung im Chat ab. Das ist wichtig: Werbefrei allein ist kein Analytics-Upgrade.

`Werbefrei + Raid Boost` kostet `6,99 EUR` und kombiniert:

- Chat-Werbung aus
- Raid-Priorisierung
- Lurker-Tax-Erinnerungen

### Extended / Analyse-orientierte Plaene

`Analyse Dashboard` kostet `8,49 EUR` pro Monat und ist der eigentliche Analytics-Premiumplan. Er schaltet zusaetzlich frei:

- die erweiterten Tabs `Viewers`, `Coaching`, `Monetization`, `Experimental` und den AI-Bereich
- Viewer-Profile und Deep Dives
- Retention-, Audience- und Vergleichsanalysen
- Coaching- und Monetarisierungs-Ansichten

Weitere Extended-Bundles im aktuellen Stripe-Katalog:

- `Werbefrei + Analyse`: `10,49 EUR` fuer volles Analytics-Dashboard plus deaktivierte Bot-Werbung
- `Analyse + Raid Boost`: `11,49 EUR` fuer Analytics plus Raid-Priorisierung und Lurker-Tax
- `Alles drin`: `13,99 EUR` fuer die Kombination aller Plan-Familien

Der AI-Bereich ist im Extended-Bereich modelliert. Fuer normale Streamer ist er produktiv aber noch nicht in allen Teilen gleich weit freigeschaltet wie der Rest des Dashboards. Das gilt vor allem fuer die tieferen AI-Analyse-Endpunkte.

## Testphase, Stripe und Kuendigung

Die Billing-Seite nutzt Stripe. Fuer den monatlichen Analyse-Plan gibt es aktuell eine `30 Tage` Testphase. Im UI gibt es auch eine Jahresoption; technisch wird dabei derzeit kein prozentualer Katalograbatt gerechnet, stattdessen werden bei Jahreskauf zusaetzliche Bonusmonate gewaehrt.

Wichtig fuer Nutzer:

- Checkout, Rechnungsdaten und Rechnungen laufen ueber die Billing-Surface
- Kuendigung ist jederzeit moeglich
- der Zugang bleibt bis Periodenende aktiv
- nach der Kuendigung bleiben Analytics-Daten noch fuer `30 Tage` gespeichert

## Raids, Live-Status und Go-Live

Die Raid-Funktionen bauen auf Twitch-OAuth auf. Sobald dein Kanal korrekt autorisiert ist, kannst du Auto-Raids aktivieren und den Status sehen. Free deckt die Grundfunktion ab, `Raid Boost` veraendert die Priorisierung im Netzwerk.

Fuer Live-Kommunikation gibt es zusaetzlich einen Go-Live-Builder fuer Discord. Dort kannst du Content, Embed, Rolle und Button konfigurieren, eine Preview erzeugen und Testsendungen ausloesen. Die Home-Seite zeigt parallel, ob OAuth, Discord und Raid-Lage gesund sind.

## Affiliate

Wenn du fuer das Affiliate-Programm freigeschaltet bist, hast du ein eigenes Portal. Dort findest du:

- deinen Referral-Link
- Gesamt-Claims
- Gesamt-Provision
- Claims des laufenden Monats
- ausstehende Auszahlung
- letzte Claims

Auszahlungen werden ueber eine separate Stripe-Connect-Anbindung vorbereitet. Wenn du noch kein Affiliate bist, zeigt das Portal genau das an, statt leere Daten vorzutaeuschen.

## Grenzen und haeufige Fragen

- Fehlende Scopes bremsen vor allem Raid- und Analyse-Funktionen. Reconnect immer ueber die Verwaltungsseite.
- Werbefrei ist kein Ersatz fuer den Analyse-Plan.
- Free hat nur die vier Basis-Tabs; viele Charts und Deep-Dive-Cards liegen bewusst hinter Paid-Cutoffs.
- Der Social-Media-Bereich ist aktuell noch kein normaler Streamer-Self-Service fuer alle.
