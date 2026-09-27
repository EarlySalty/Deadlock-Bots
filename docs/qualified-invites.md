# Qualifizierte Twitch-Einladungen

Stand: 27. September 2026. Dieses Paket ist ausschließlich für eine gemeinsame PR-Prüfung mit Deadlock-Twitch-Bot vorgesehen. Es führt keine Migration auf einer laufenden Installation aus und enthält keinen Deploy oder Dienstneustart.

## Wiederverwendete Quellen

Vor dem Bau wurde der lokale Graphify-Graph geprüft: `graphify-out/graph.json`, Graph-Commit `56c00515d6064fe63539b4f177c45484c8be9a39`. Der Graph war älter als die Arbeitsbasis `42175e5f`; alle relevanten Knoten wurden am aktuellen Quelltext nachgeprüft. Wiederverwendet werden der InviteTracker, `dl_activity::join_source::classify`, `dl-twitch-invite-sync`, die bestehenden Broker-Authentifizierungsfunktionen und der Discord-Adapter. Die Aktivitätsquellen bleiben `activity.member_events`, `activity.message_activity` und `activity.voice_session_log` sowie die bereits bestehenden VoiceTracker-Events und bestätigten Gateway-Snapshots.

Der bisherige Nachrichtenbestand hat nur Gesamtzähler. Der vorhandene Nachrichten-Writer speichert deshalb zusätzlich Nachricht-ID und Originalzeitpunkt als minimalen, idempotenten Nachweis; weder Inhalt noch Name werden dafür übernommen. Die separate Punktelogik des VoiceTrackers verlangt bisher mehrere aktive Personen. Ein optionaler Beobachter im selben Event- und Keepalive-Pfad erfasst für Einladungen stattdessen nur den ersten belegten 15-Minuten-Zeitpunkt. Damit zählen auch alleinige und stummgeschaltete Teilnehmer. Es gibt keinen zweiten Gateway, keinen zweiten Voice-Abonnenten und keinen zweiten allgemeinen Sitzungsverlauf.

## Discord-Limit und Konfiguration

Discord dokumentiert Fehlercode `30016` für das Maximum von **1.000 Einladungen pro Guild**. Geprüft am 27. September 2026:

- https://docs.discord.com/developers/topics/opcodes-and-status-codes
- https://docs.discord.com/developers/resources/channel#create-channel-invite

Die Erstellung verwendet `max_age = 0`, `max_uses = 0`, `temporary = false`, `unique = true`: unbegrenzte Gültigkeit, unbegrenzte Nutzungen, keine temporäre Mitgliedschaft, kein versehentlich gemeinsam verwendeter Invite.

Die normale Datei `config/bot.toml` enthält:

```toml
[twitch_invites]
personal_links_per_channel = 10
guild_invite_reserve = 50
evaluation_interval_seconds = 300
excluded_voice_channel_ids = []
sync_dry_run = false
```

Im ausgelieferten TOML ist dieselbe Sektion als Inline-Tabelle abgelegt. Fehlt die Sektion in einer bestehenden Installation, ist der persönliche Deckel standardmäßig 0; neue persönliche Links bleiben dann aus. Bereits gespeicherte Links werden auch bei einem nachträglich niedrigeren Deckel wiederverwendet. Beim Deckel, bei erschöpfter Guild-Reserve oder bei einem Discord-Fehler fällt die Erstellung auf den vorhandenen Kanallink zurück. Damit bleibt `inviter_twitch_user_id` für daraus entstandene Joins NULL: Ein Fallback darf nicht fälschlich einem Zuschauer zugerechnet werden.

Die Reserve berücksichtigt den tatsächlich von Discord gelieferten gesamten Invite-Bestand, nicht nur eigene Tabellenzeilen. Der bekannte API-Höchstwert ist eine Plattformgrenze, der Kanaldeckel und die Reserve sind Dateikonfiguration. Gleichzeitige eigene Erstellungen werden über eine Postgres-Transaktionssperre je Guild serialisiert. Externe Ersteller können trotzdem zeitgleich Discord-Kapazität verbrauchen; auch der anschließende API-Fehler führt zum Kanallink.

AFK kommt aus dem bestätigten Guild-Cache. Staging-Kanäle kommen aus der vorhandenen TempVoice-Konfiguration. Zusätzliche ausgeschlossene Kanäle lassen sich über `excluded_voice_channel_ids` eintragen. Betriebswerte kommen aus TOML; der Broker und der Invite-Sync verwenden die vorhandenen internen Secrets aus dem Infisical-Bootstrap. Der Sync liest seine URL aus `runtime.bridges.twitch_api_url`, akzeptiert nur numerisches Loopback und verfolgt keine Redirects.

## Interner Vertrag

Beide Endpunkte verwenden den bestehenden Broker: echte Loopback-Gegenstelle, `X-Internal-Token`, bestehende Allowlist und vorhandene Antwort-Hülle. Eine externe Anfrage darf weder über einen Forwarded-Header noch über Nutzdaten die interne Herkunft vortäuschen.

### Persönlicher Link

`POST /internal/master/v1/twitch/personal-invite`

```json
{
  "streamer_login": "streamer",
  "streamer_twitch_user_id": "42",
  "inviter_twitch_user_id": "43"
}
```

Die Twitch-IDs stammen beim Aufrufer aus dem Twitch-Chatereignis. Die serverseitige Guild- und Kanalzuordnung kommt ausschließlich aus dem vorhandenen Streamer-Invite-Sync. Der Broker prüft die gespeicherte Kanalidentität und beide Discord-Allowlists. Gleiche Zuschauer- und Broadcaster-ID ergibt den Kanallink.

Ergebnis innerhalb der bestehenden `result`-Hülle:

```json
{"invite_url":"https://discord.gg/Beispielcode","personal":true}
```

Der Schlüssel ist `(streamer_twitch_user_id, inviter_twitch_user_id)`, nicht ein veränderlicher Twitch-Login. Es wird erst nach erfolgreichem Speichern ein persönlicher Link ausgeliefert. Bei einem sicher festgestellten Speicherfehler wird der neu erzeugte, noch nicht ausgelieferte Discord-Link widerrufen. Ein widerrufener persönlicher Link bleibt für die Attribution historischer Joins gespeichert; weitere Anfragen erhalten den Kanallink.

### Statusänderungen

`GET /internal/master/v1/twitch/qualified-invites?since=<RFC3339>`

Optionale Parameter: `limit` (1 bis 1.000, Standard 500), `until`, `after_updated_at`, `after_join_id`.

```json
{
  "invites": [{
    "join_id": "123",
    "streamer_login": "streamer",
    "inviter_twitch_user_id": "43",
    "joined_at": "2026-10-01T12:00:00Z",
    "status": "qualified",
    "qualified_at": "2026-10-15T12:00:00Z",
    "updated_at": "2026-10-15T12:02:00Z"
  }],
  "until": "2026-10-15T12:03:00Z",
  "next_cursor": null,
  "next_since": "2026-10-15T12:03:00Z"
}
```

Die Beispiele sind synthetisch. `since` filtert **Änderungszeit**, nicht Beitrittszeit. Ein vor Wochen erfolgter Join erscheint bei späterer Qualifikation erneut. `join_id` ist die stabile Quellereignis-ID als Zeichenfolge, keine Discord-Mitglieds-ID. Der Verbraucher verarbeitet die Zustände idempotent je Join; für einen späteren Punkte-Ledger bietet sich `qualified_invite:<join_id>` an. Dieser PR baut keinen Punkte-Ledger.

Bei `next_cursor` müssen derselbe `since`-Wert, das gelieferte `until` und beide Cursorfelder für die nächste Seite verwendet werden. Erst die letzte Seite liefert `next_since`. Die nächste Pollrunde beginnt inklusive dieses Zeitpunkts; Wiederholungen sind erwartete, deduplizierbare Zustandsmeldungen. Die Guild-Sperre synchronisiert Commit-Zeit und Wasserstand, damit noch nicht abgeschlossene Transaktionen nicht hinter einem bereits bestätigten Wasserstand verschwinden. Werden Einträge während einer Seitennavigation erneut geändert, erscheinen sie in der folgenden Pollrunde. Es gibt keine stille Begrenzung ohne Fortsetzung.

Der Export enthält keine Discord-Mitglieds-ID, keine Discord-Namen und keine Nachrichteninhalte.

## Zustände und Nachweise

Ein Join startet dauerhaft als `pending`. Er wird genau einmal `qualified` oder `expired`. Datenbank-Trigger protokollieren die Übergänge und verhindern das Löschen, das Überschreiben abgeschlossener Zustände und das spätere Austauschen der Attribution.

Qualifiziert wird erst, wenn beide Bedingungen spätestens 720 Stunden nach dem tatsächlichen Discord-Beitritt erfüllt waren:

1. Die ursprüngliche Mitgliedschaft bestand mindestens 336 Stunden ohne vorherigen Austritt.
2. Eine einzelne zulässige Voice-Sitzung erreichte 15 Minuten oder mindestens fünf unterschiedliche Nachricht-IDs lagen auf mindestens zwei Kalendertagen in Europe/Berlin.

Die Halte- und Ablaufzeiten sind verstrichene Stunden, keine vom Datenbank-Zeitzonenwechsel beeinflussten Kalendertage. Nur die Nachrichten-Tagesgrenze verwendet Europe/Berlin. Mehrere kurze Voice-Sitzungen werden nicht zusammengerechnet. Aktivität vor dem Join oder nach dessen Frist zählt nicht. Ein nachweislich späterer Austritt löscht eine bereits erreichte Qualifikation nicht.

Der zusätzliche Voice-Nachweis ist ein kleiner Satz Zeitfelder im Mitgliedschafts-Ledger. Er entsteht aus den existierenden Voice-Ereignissen und dem zweiminütigen Keepalive. Bei unbestätigtem Cache, Prozessneustart oder einer Beobachtungslücke von mehr als zwei Keepalive-Intervallen wird die laufende Uhr verworfen, nicht die bereits belegte Qualifikation. Der periodische Auswerter bestätigt vor Ablaufentscheidungen zusätzlich den aktuellen Snapshot. Alte abgeschlossene `voice_session_log`-Zeilen bleiben eine weitere Nachweisquelle.

Die Auswertung benutzt Discord-Mitgliedsdaten inklusive ursprünglicher Beitrittszeit. Nur `Unknown Member` wird als bestätigte Abwesenheit behandelt; Transport-, Berechtigungs- und andere Discord-Fehler lassen den Join ausstehend. Ein späterer Lauf kann eine innerhalb der Frist bereits belegte Qualifikation mit deren tatsächlichem Zeitpunkt nachtragen.

## Altdaten, Mehrdeutigkeit und Grenzen

Die Migration markiert vorhandene Mitglieder aus Join-, Nachrichten- und Voice-Historie als bereits bekannt. Neue Gateway-Joins erhalten einen dauerhaften Erstmitgliedschafts-Nachweis. Rejoins setzen diese Sperre und die ursprüngliche Frist nicht zurück. Startup-Backfills und Joins ohne verlässliche Discord-Beitrittszeit gelten vorsichtshalber nicht als erstmalig. Botkonten werden schon im vorhandenen Member-Writer ausgeschlossen. Datenschutz-Opt-outs qualifizieren nicht; ein minimaler früherer Mitgliedschaftsnachweis verhindert späteres erneutes Zählen.

Es werden keine rückwirkenden Credits für Mitglieder vor Einführung der Migration erzeugt. Eine frühere Mitgliedschaft, für die vor Einführung keinerlei Historie mehr existiert, lässt sich technisch nicht rekonstruieren. Diese historische Grenze wird nicht mit erfundenen Nachweisen überdeckt.

Discord liefert im Member-Join keinen eindeutig referenzierten Invite-Code. Der bestehende Snapshotvergleich wird daher konservativer: Nur genau ein Invite mit genau einer zusätzlichen Nutzung wird zugeordnet. Mehrdeutige gleichzeitige Nutzungen, fehlende Snapshots und uneindeutige Codebesitzer bleiben unbekannt. Pro-Guild-Sperren verhindern, dass parallele Handler denselben Zähleranstieg mehrfach verbrauchen. Invite-Codes werden bei der Eigentümerauflösung exakt und unter Berücksichtigung der Groß-/Kleinschreibung verglichen.

Discord-Erstellung und Postgres-Commit sind keine gemeinsame verteilte Transaktion. Ein Prozessabbruch unmittelbar nach Discord-Erstellung oder ein unklarer Commit-Ausgang kann einen nicht ausgelieferten Invite hinterlassen. Solche Codes werden nicht als persönliche Links ausgegeben, ohne gespeicherte Zuordnung nicht kreditiert und durch die reale Guild-Kapazitätsprüfung berücksichtigt. Es gibt keine automatische Löschung fremder Discord-Einladungen.

## Gemeinsame Auslieferungsvoraussetzung

Vor einem später ausdrücklich beauftragten Rollout müssen zentrale Migration, beide Codepakete und der erweiterte Streamer-Invite-Sync gemeinsam berücksichtigt werden. Fehlen Twitch-ID oder Zielkanal in einer alten Synchronisation, erzeugt der Broker keinen persönlichen Link; der Twitch-Bot verwendet weiter den Kanallink. Bestehende Tokens werden wiederverwendet. Dieses Arbeitsziel endet mit offenen Draft-PRs und ihrer CI-Auswertung, nicht mit einem Rollout.
