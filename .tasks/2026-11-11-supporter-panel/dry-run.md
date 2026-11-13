# Support panel dry-run, funding 0 € of 250 €

Rendered by `build_support_publish_output` with `dry_run = true`, empty KV and payload format `3`, with the dl-core defaults (`config/bot.toml` sets no supporter values): tiers 5/10/20/50 €, goal 250 €, Twitch off (`supporter_twitch_api_base` unset), `supporter_role_grant_apply` unset (off). No warnings. Nothing was posted to Discord. Buttons are shown as label and custom_id.

With `supporter_role_grant_apply = true` (go-live) these paragraphs differ; the cards below show the flag-off render.

Card 1 (support):

- on only: ## Was du jetzt bekommst
- on only: **Mit den Streamern zusammen spielen.** Als Unterstützer bist du ein vertrauenswürdiges Mitglied der Community. Du kommst in den Streamer-VC und kannst dort mit den Streamern zusammen zocken.
- on only: **Die Rolle Server Unterstützer.** Damit sieht jeder, dass du den Server mitträgst.

Card 3 (support:zahlung):

- off only: **Monatlich über Stripe.** 5 €, 10 €, 20 € oder 50 € im Monat. Kündigen kannst du jederzeit selbst über den Button unten.
- off only: **Einmalig.** Den Betrag wählst du selbst, den Mindestbetrag zeigt dir die Zahlungsseite.
- off only: **Server-Boost.** Zählt vorerst als Unterstützung.
- on only: **Monatlich über Stripe.** 5 €, 10 €, 20 € oder 50 € im Monat. Die Rolle kommt automatisch. Kündigen kannst du jederzeit selbst über den Button unten.
- on only: **Einmalig.** Den Betrag wählst du selbst, den Mindestbetrag zeigt dir die Zahlungsseite. Die Rolle kommt automatisch.
- on only: **Server-Boost.** Zählt vorerst als Unterstützung. Die Rolle kommt automatisch.

## Card 1 (support), action planned_post

[Banner: attachment://support-hero.png]

# Server unterstützen

Hier steht, wie du den Server und die Projekte unterstützen kannst. Jeder Weg zählt gleich.

---

## Steam Bot

Als Nächstes wird der Steam Bot zur Funktion für Unterstützer. Dann holt er deine Matches bevorzugt ab, ähnlich wie bei https://deadlock-api.com/patron: schnellere Updates, deine ganze Match-Historie und dein Rang direkt von Steam.

Echtzeit verspreche ich nicht, was möglich ist, hängt von Steam ab.

## Card 2 (support:vorteile), action planned_post

## Was noch kommt

Das ist geplant und noch nicht fertig:

- **Patch-Watchlist.** Du suchst dir Helden und Items aus und bekommst Bescheid, wenn ein Patch genau daran etwas ändert.
- **Build-Helfer im Deadlock Brain.** Du wählst einen Helden und bekommst passende Items mit einer Erklärung, auch gegen bestimmte Gegner.
- **Mehr Match-Funktionen im Steam Bot.**
- **Turniere und Scrims.** Unterstützer testen neue Turnierfunktionen als Erste. Dazu kommen zusätzliche Community-Abende und Fun-Turniere.
- **Coaching-Material** für Unterstützer.
- **Mitbestimmen.** Unterstützer stimmen ab, was als Nächstes gebaut wird.

---

## Was kostenlos bleibt

Der Server wird nicht kostenpflichtig. Mitspieler suchen, die normalen Voice-Channels, Community-Aktivitäten, reguläre Turniere, Scrims und das normale Coaching bleiben frei.

Bei regulären Turnieren und beim normalen Coaching bringt Unterstützung keinen Vorteil.

## Card 3 (support:zahlung), action planned_post

## Monatsziel

`░░░░░░░░░░░░░░░░░░░░` 0 %
**0 € von 250 € im Monat**
-# Hier zählen die laufenden Abos und die einmaligen Beträge aus diesem Monat.

---

## So kannst du unterstützen

**Monatlich über Stripe.** 5 €, 10 €, 20 € oder 50 € im Monat. Kündigen kannst du jederzeit selbst über den Button unten.

**Einmalig.** Den Betrag wählst du selbst, den Mindestbetrag zeigt dir die Zahlungsseite.

**Server-Boost.** Zählt vorerst als Unterstützung.

---

Danke, dass du den Server und die Projekte mitträgst ❤️

---

[Supporter werden] (support:become) | [Einmalig unterstützen] (support:tip) | [Abo verwalten oder kündigen] (support:manage)
