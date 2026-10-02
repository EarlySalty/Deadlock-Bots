# Vertrag A: Spielerpool

Stand: 2026-10-02. Implementierung auf `feat/spielerpool-a`, noch vor Merge-Gate und Live-Prüfung. Der Vertrag beschreibt die tatsächlich angelegte Datenschicht. Änderungen an Signaturen oder Tabellen müssen hier nachgezogen und dem Delegator gemeldet werden.

## Zugriff und Identitäten

Das Crate heißt `dl-pool`. `PoolStore::new(PgPool)` nutzt die bestehende zentrale PostgreSQL-Verbindung; es öffnet keine eigene Verbindung, liest keine Secrets und migriert nicht selbst. Der kanonische Migrator bleibt `dl-central-migrate`.

`Scope { guild_id: i64, discord_id: i64 }` gehört zu allen Profiloperationen. Produktiv gilt die Guild `1289721245281292288`; die Datenschicht unterstützt getrennte Guilds und setzt die Guild nicht stillschweigend ein. B, C und E nehmen sie aus der vorhandenen normalen Bot-Konfiguration. D prüft für jeden Zugriff die authentifizierte Discord-Session und die aktuelle Guild-Mitgliedschaft. Eigene Profiländerungen, Steam-Link, DM-Opt-in, Löschen und Feedback verwenden ausschließlich die Discord-ID aus dieser Session. Vom Browser gelieferte Identitätsfelder sind keine Autorisierung.

Die Library ist eine interne Datenschicht und prüft keine Discord-Mitgliedschaft. E authentifiziert seinen internen Endpunkt mit dem vorhandenen Bot-Token. D nutzt `public_profile` und `list_profiles` für fremde Profile. `own_profile`, `preferred_players` und `own_feedback` sind nur für den jeweils authentifizierten Eigentümer bestimmt. DM-Opt-in und bevorzugte Mitspieler sind keine öffentlichen Profilfelder. Feedback hat keine öffentliche Leserfunktion.

Discord-, Guild-, Kanal-, Match- und Session-IDs sind in PostgreSQL und Rust positive `BIGINT`/`i64`. D und E serialisieren diese IDs an JSON-Grenzen als Dezimalstrings, damit JavaScript sie nicht rundet. Die Serde-Ableitungen der internen Rust-Typen erledigen diese Umwandlung nicht.

## Migration und Tabellen

Migration: `rust/crates/dl-central-db/migrations/20261002000000_spielerpool.sql`. Die Nummer wurde nach frischem Fetch von `origin/main` vergeben und ist größer als alle vorhandenen Nummern. Angewandte Migrationen bleiben unverändert.

Das Schema `pool` ist für `PUBLIC` gesperrt. Die bestehende Rolle `deadlock` erhält Nutzung des Schemas, Lesen und Schreiben der Tabellen sowie Nutzung der Session-Sequenz. Weitere Rollen erhalten keine automatische Freigabe. Die Mitgliedschaftsprüfung erfolgt in den späteren Diensten, es gibt keine RLS mit einer vom Client gesetzten Identität.

| Tabelle | Schlüssel und Inhalt |
| --- | --- |
| `pool.profiles` | `(guild_id, discord_id)`, FK auf `core.users`; Modi, Gruppengröße, Spielstil, Voice-Vorliebe, Sprachen, Zeitzone, `dm_opt_in DEFAULT false`, `published DEFAULT false`, Interviewabschluss und Zeitpunkte |
| `pool.availability` | `(guild_id, discord_id, weekday, start_minute, end_minute)`; wöchentliche Zeitfenster in der Profilzeitzone |
| `pool.preferred_players` | `(guild_id, discord_id, target_discord_id)`; gerichtete Auswahl bevorzugter Mitspieler, beide Profile in derselben Guild |
| `pool.api_snapshots` | `(guild_id, discord_id)`; genau ein aktueller API-Stand mit ausgewählter `steam_id`, optionalem Rang und Gesamtwerten, beobachtetem Umfang, Zeitraum, Abrufzeit und Heatmap |
| `pool.matches` | `(guild_id, discord_id, match_id)`; eigener Anteil der abgerufenen Historie mit Startzeit, optionaler Dauer und optionalem Modus |
| `pool.co_players` | `(guild_id, discord_id, target_discord_id)` mit `discord_id < target_discord_id`; ungerichtete Paare mit Anzahl gemeinsamer Spiele, letzter Spielzeit, Beobachtungszeitraum und Abrufzeit |
| `pool.sessions` | generierte `session_id`, zusätzlich eindeutig `(guild_id, session_id)`; Initiator, Modus, Kanal, Status, Erstellungs-, Ablauf-, Start- und Endzeit |
| `pool.session_participants` | `(guild_id, session_id, discord_id)`; Teilnehmer und erster beobachteter Beitritt |
| `pool.feedback` | `(guild_id, session_id, discord_id)`; vier Checkboxen und Zeitpunkte, FK auf die eigene Teilnahme |

Alle abhängigen Tabellen sind über zusammengesetzte Fremdschlüssel an Profile und Sessions derselben Guild gebunden. Profile referenzieren `core.users`. Statistik und eigene Matches referenzieren den aktuellen Schlüssel `(discord_id, steam_id)` von `core.steam_links`, wobei `steam_id` ein Textfeld ist. Die alte Architekturübersicht mit dem Primärschlüssel `(discord_id, steam_id64)` beschreibt hier einen überholten Stand.

## Typen und fachliche Werte

Die öffentlich exportierten Typen sind `Scope`, `Mode`, `PlayStyle`, `VoicePreference`, `SessionStatus`, `Availability`, `Preferences`, `Profile`, `PublicProfile`, `ApiSnapshot`, `Match`, `CoPlayer`, `SteamLink`, `PoolFilter`, `Session`, `Participant`, `OwnSession` und `Feedback`.

| Typ | Zulässige Werte beziehungsweise Bedeutung |
| --- | --- |
| `Mode` | `Casual`, `Ranked`, `StreetBrawl`; Serde/SQL: `casual`, `ranked`, `street_brawl` |
| `PlayStyle` | `Any`, `Relaxed`, `Competitive`, `Learning`; Serde/SQL in snake_case |
| `VoicePreference` | `Any`, `WithVoice`, `WithoutVoice`; Serde/SQL in snake_case |
| Gruppengröße | minimale und maximale Gesamtzahl einschließlich eigener Person, jeweils 2 bis 6 |
| `Availability` | `weekday: i16`, Montag 0 bis Sonntag 6; `start_minute` einschließlich, `end_minute` ausschließlich, 0 bis 1440; Fenster über Mitternacht in zwei Einträge teilen |
| Zeitzone | gültiger IANA-Name, Default `Europe/Berlin`; Zeitfenster sind lokale Wochenzeiten |
| Heatmap | genau 168 nichtnegative `i32`, Index `weekday * 24 + hour`; C verwendet UTC mit Montag 0, zählt Matchstarts pro Stunde und nennt den gespeicherten Beobachtungszeitraum |
| Rang | `rank_tier` und `rank_subtier` als optionale nichtnegative `i32`; C trägt ausschließlich tatsächlich gelieferte API-Werte ein, D beschriftet deren konkrete API-Skala |
| Gesamtwerte | `games_played` und `total_play_seconds` als optionale `i64`; nur befüllen, wenn die API den Gesamtwert tatsächlich liefert |
| Beobachtete Werte | `observed_games` und `observed_play_seconds` als nichtnegative `i64` innerhalb `window_start` bis `window_end`; keine Aussage über die gesamte Karriere |
| `Feedback` | `play_again`, `friendly`, `good_communication`, `balanced_match`, jeweils `bool`, Default false; keine Freitextfelder |

C klärt die verfügbaren API-Felder und Endpunkte in seinem Paket. Dieser Vertrag behauptet weder, dass ein Gesamtstunden-Endpunkt existiert, noch dass Rang aus einem bestimmten API-Feld kommt. Fehlende API-Werte bleiben `None`/SQL-NULL. Aus Dauerwerten berechnete Stunden sind Sekunden geteilt durch 3600; beobachtete Stunden müssen in D als solche erkennbar sein. Dauer und Modus einzelner Matches dürfen fehlen. Es werden keine rohen API-Antworten und keine Steam-IDs von Personen außerhalb des Pools gespeichert.

Checkboxnamen und Spielstilwerte sind die technischen Auswahlmöglichkeiten der Datenschicht. B und D liefern ihre deutschen Beschriftungen. Feedback bezieht sich auf die gesamte Session, hat keinen bewerteten Einzelspieler und benötigt keine Benachrichtigung.

## API für B und D

Alle Methoden sind asynchron. Sofern unten nichts anderes steht, liefern sie `PoolResult<T>`. `PoolError` unterscheidet `Database`, `Invalid`, `OptedOut`, `NotFound`, `UnverifiedSteamLink`, `InvalidSessionState` und `StaleSnapshot`.

| Methode | Ergebnis und Verhalten |
| --- | --- |
| `save_preferences(Scope, &Preferences, completed: bool)` | `Profile`; ersetzt strukturierte Präferenzen, Zeitfenster und bevorzugte Spieler atomar. `completed=false` speichert einen unsichtbaren Entwurf. `true` verlangt mindestens einen Modus, veröffentlicht das Profil und setzt den Interviewabschluss. Bestehendes DM-Opt-in bleibt erhalten. |
| `own_profile(Scope)` | `Option<Profile>`; eigene gespeicherte Profilfelder einschließlich DM-Schalter |
| `availability(Scope)` | `Vec<Availability>` in sortierter Reihenfolge; bei fremden Profilen darf D sie erst nach einem erfolgreichen `public_profile`-Zugriff ausgeben |
| `preferred_players(Scope)` | `Vec<i64>`; nur eigene Auswahl, Ziel muss beim Speichern ein veröffentlichtes Profil derselben Guild besitzen |
| `set_dm_opt_in(Scope, enabled: bool)` | `()`; gesonderte bewusste Änderung des Schalters |
| `list_profiles(guild_id, &PoolFilter)` | `Vec<PublicProfile>`; nur veröffentlichte Profile ohne Datenschutz-Opt-out, optional mit API-Werten; keine Steam-ID, DM-Einstellung oder bevorzugten Spieler |
| `public_profile(Scope)` | `Option<PublicProfile>`; dieselbe öffentliche Projektion eines Profils |
| `recently_played_with(Scope, limit)` | `Vec<CoPlayer>`; nur Paare, deren beide Profile veröffentlicht sind und keinen Opt-out haben; höchste letzte Spielzeit zuerst, maximal 100 |
| `delete_profile(Scope)` | `u64`; idempotent, entfernt alle Pool-Daten dieses Profils in dieser Guild einschließlich gemeinsamer Sessions und deren Feedback. Globaler Steam-Link bleibt bestehen. |

`Preferences` enthält `modes`, `group_size_min`, `group_size_max`, `play_style`, `voice_preference`, `languages`, `timezone`, `availability` und `preferred_discord_ids`. Ein neuer Entwurf hat noch keine Modi, Gesamtgruppengröße 2 bis 6, Spielstil/Voice `Any`, Sprache `de`, Zeitzone `Europe/Berlin` und keine ausgewählten Spieler oder Zeitfenster. Es gibt keinen Freitext-Interviewverlauf in diesem Crate.

`PoolFilter` enthält optionale `mode`, `min_rank`, `max_rank`, `weekday`, `minute`, `timezone` sowie `limit` und `offset`. Ranggrenzen vergleichen `rank_tier`; unbekannter Rang erfüllt einen gesetzten Rangfilter nicht. Der Zeitfilter benötigt gemeinsam Wochentag, Minute und Zeitzone und prüft angegebene Verfügbarkeit bei Profilen mit exakt dieser Zeitzone. Er vergleicht keine aus API-Matches abgeleitete Wahrscheinlichkeit. Ohne Zeitfilter erscheinen auch Profile ohne Zeitfenster. `limit=0` verwendet 50; sonst maximal 100, `offset` darf nicht negativ sein. Andere Zeitzonen muss D vor einem Vergleich bewusst behandeln; ein Sommerzeitversprechen lässt sich aus bloßen Wochenzeiten nicht ableiten.

`CoPlayer` enthält das kanonisch geordnete Paar. D ermittelt die andere Person aus der eigenen abgefragten Discord-ID und zeigt nur öffentliche Poolprofile. Die Auswahl bevorzugter Spieler ist unabhängig von aus Matchdaten ermittelten Paaren.

## Steam-Link und C

`link_verified_steam(Scope, steam_id: &str) -> PoolResult<()>` schreibt in `core.steam_links`. D darf diese Methode erst nach erfolgreicher serverseitiger Steam-OpenID-Prüfung und gültiger Discord-Session aufrufen. Die Methode selbst führt keine OpenID-Prüfung aus. Sie prüft die dezimale individuelle SteamID64, markiert den Link als bestätigt und setzt beim ersten primären Konto `primary_account`. Eine bereits einer anderen Discord-ID gehörende Steam-ID wird durch die bestehenden DB-Guards abgewiesen. `is_steam_friend` bleibt beim neuen Link false, `friend_bot_account_id` NULL. Bestehende Freundschaften werden beim Bestätigen eines vorhandenen Links nicht geändert.

`steam_links_for_ingest(guild_id) -> PoolResult<Vec<SteamLink>>` liefert pro veröffentlichtem Poolprofil ein bestätigtes Konto ohne Opt-out. Primäre Konten gehen vor; bei Gleichstand entscheidet `steam_id` lexikografisch. `SteamLink` enthält `discord_id`, `steam_id` und `primary_account`. C rechnet die tatsächlich gewählte SteamID64 bei Bedarf auf die API-Account-ID um. Keine neue Steam-Bot-Freundschaft ist dafür nötig.

`store_api_snapshot(Scope, &ApiSnapshot, matches: &[Match]) -> PoolResult<()>` ersetzt Snapshot und die eigene gespeicherte Matchhistorie in einer Transaktion. Maximal 10.000 Matcheinträge pro Aufruf. Historie und Aggregation müssen denselben benannten Zeitraum beschreiben. Ältere Abrufzeiten werden mit `StaleSnapshot` abgewiesen; derselbe Abrufstand ist wiederholbar. Ein vorhandenes Profil und genau das derzeit von `steam_links_for_ingest` ausgewählte bestätigte eigene Steam-Konto sind erforderlich. Abweichende Konten werden mit `UnverifiedSteamLink` abgewiesen, auch wenn ein weiterer Link bestätigt ist. C darf gelöschte Profile nicht automatisch wieder anlegen.

`store_co_player(guild_id, &CoPlayer) -> PoolResult<()>` schreibt ein kanonisches Paar mit zwei vorhandenen veröffentlichten Profilen derselben Guild. Beide Profile brauchen weiterhin ein bestätigtes auswählbares Steam-Konto. C ordnet die IDs aufsteigend, zählt gemeinsam beobachtete Spiele und schreibt einen vollständigen Zählerstand statt eines Inkrements. Ein nach einer Entknüpfung verspätet eintreffender Paar-Write wird ohne bestätigten Link abgewiesen. Ältere Abrufzeiten werden abgewiesen. Andere Paarzeilen werden dadurch nicht ersetzt; außerhalb des gespeicherten Beobachtungszeitraums gibt es keine Aussage über gemeinsame Spiele. Bei einem gelöschten Profil verschwinden beide Paarseiten. Eine Steam-Entknüpfung oder Änderung von Bestätigung/Primärkonto invalidiert die vorhandene Pool-Statistik, Matches und Paare dieser Discord-ID in allen Guilds.

## E: Sessions und D: Feedback

| Methode | Ergebnis und Verhalten |
| --- | --- |
| `create_session(initiator: Scope, targets: &[i64], Mode)` | `Session` mit Status `Pending`; 2 bis 6 verschiedene veröffentlichte Poolprofile derselben Guild, Initiator ist immer Teilnehmer; Erzeugung mit allen Teilnehmern atomar |
| `attach_channel(guild_id, session_id, channel_id)` | `Session`; nur `Pending` vor Ablauf nach `Open`; E gibt die Kanal-ID erst nach erfolgreicher Discord-Erstellung an |
| `own_sessions(Scope, limit, offset)` | `Vec<OwnSession>`; privater persistenter Leser beendeter Sessions mit eigenem gespeichertem Beitritt und eigenem optionalem Feedback; Endzeit und Session-ID absteigend, Standard 50/maximal 100, nichtnegative Seitengröße und Offset; andere Guilds, fremde Teilnehmer und Opt-out liefern keine Zeilen |
| `participant_session(Scope, session_id)` | `Option<Session>`; Session nur für eigene Teilnehmer |
| `participants(Scope, session_id)` | `Vec<Participant>`; vollständige Teilnehmerliste nur, wenn die angefragte Person selbst teilnimmt |
| `record_join(Scope, session_id, at)` | `()`; setzt ersten Beitritt und ersten Sessionstart, Übergang `Open` nach `Active`; nur eigene gelistete Teilnahme, Zeit innerhalb der ersten Stunde |
| `due_sessions(guild_id, now)` | `Vec<Session>`; interne E-Funktion, fällige `Pending`, `Open` und `Active`, nach Ablaufzeit sortiert |
| `finish_session(guild_id, session_id, SessionStatus, at)` | `Session`; `Failed` für Fehler, `Expired` erst nach einer Stunde ohne Beitritt, `Ended` nach tatsächlichem Start; ein Endzustand wird nicht nachträglich überschrieben |
| `save_feedback(Scope, session_id, Feedback)` | `()`; nur nach `Ended` und nur für Teilnehmer mit gespeichertem Beitritt; wiederholtes Speichern ersetzt die eigenen vier Checkboxen |
| `own_feedback(Scope, session_id)` | `Option<Feedback>`; ausschließlich eigene Antworten |

`OwnSession` enthält `session: Session`, den eigenen `joined_at`-Zeitpunkt und `feedback: Option<Feedback>`. D ruft diese Liste nach jedem Login im eigenen Profil auf, ohne bereits eine Session-ID kennen zu müssen. Auch gespeicherte Checkboxen bleiben über Browserwechsel hinweg sichtbar. Die Liste umfasst ausschließlich beendete Sessions mit eigener tatsächlicher Teilnahme; offene, abgelaufene oder nur angebotene Sessions erscheinen dort nicht. Es gibt keine öffentliche Sessionliste.

`expires_at = created_at + 1 Stunde` ist ein DB-Constraint. Eine aktive Session darf nach Ablauf dieser Stunde weiterbestehen. E entscheidet nach beobachteter Kanalbelegung, wann sie endet, und verwaltet den Discord-Kanal. `due_sessions` liefert aktive Sessions mit, damit E deren Zustand weiter prüfen kann; `Expired` ist für aktive Sessions gesperrt. Weder Migration noch Library erstellen Kanäle, pingen Nutzer oder senden DMs.

E prüft unmittelbar vor einer optionalen DM `own_profile` für den Teilnehmer und das dortige `dm_opt_in` sowie den bestehenden Datenschutz-/DM-Schutz des Bots. Fehlendes Profil bedeutet keine DM. D gibt Feedback ohne Benachrichtigungsauftrag ab.

Kanal-Erstellung und DB-Transaktion können nicht atomar über Discord laufen. E legt erst die Session an, erstellt dann den Kanal und ruft `attach_channel` auf. Scheitert die Zuordnung oder verschwindet die Session durch Löschen, entfernt E den angelegten Kanal als Ausgleich. Bei Profil-Löschung wird eine ganze gemeinsame Session entfernt, einschließlich anderer Feedbackantworten, damit keine Pool-Zuordnung zum gelöschten Teilnehmer bleibt. E muss verschwundene Sessionzeilen beim Kanal-Reconcile berücksichtigen und seine eigenen angelegten Kanäle ohne verbleibende DB-Zuordnung entfernen. Die Library hat keine externe Löschqueue.

Der HTTP-Vertrag aus `PAKETE.md` bleibt verbindlich: E stellt den bestehenden internen Endpunkt bereit, D ruft ihn auf. `session_id` ist die generierte Pool-Session-ID als Dezimalstring; `channel_url` entsteht aus Guild und Kanal. Port, Auth-Header und Endpunktverdrahtung legt E in seinem Vertrag fest.

## Datenschutz und vollständige Löschung

Alle neuen nutzerbezogenen Rust-Schreibpfade nehmen den bestehenden zentralen Privacy-Lock und prüfen darunter `core.user_privacy` auf `opted_out` oder `deleted_at`. Mehrere Teilnehmer werden in aufsteigender Discord-ID gesperrt. B verwendet für einen späteren bewussten Datenschutz-Wiedereinstieg den vorhandenen Opt-in-Pfad; die Pool-Library löscht keinen Datenschutz-Grabstein.

`delete_all_for_user_tx(&mut Transaction<Postgres>, discord_id) -> Result<u64, sqlx::Error>` entfernt die Profile dieser Discord-ID in allen Guilds innerhalb der aufrufenden Transaktion. Der Aufrufer hält beziehungsweise erhält den zentralen Privacy-Lock; die Funktion setzt selbst keinen Grabstein. `dl-community/src/privacy.rs` ruft sie im bestehenden vollständigen Löschpfad auf. Der bestehende Pfad setzt in derselben Transaktion den Datenschutz-Grabstein und entfernt auch globale Steam-Links. Bei einem späteren Fehler rollt die Pool-Löschung mit zurück.

Fremdschlüssel löschen Verfügbarkeit, bevorzugte Spieler, API-Statistik, Matches, Paarbeziehungen und Teilnahme/Feedback. Ein Trigger entfernt zuvor alle Sessions mit dem gelöschten Profil. Dieselbe Löschung greift bei direktem Profil-DELETE und beim Löschen von `core.users`. Alle neuen nutzerbezogenen Spalten sind auch in der bestehenden Datenschutz-Tabellenliste für Export und Schema-Vertragsprüfung registriert. Der Export ergänzt Sessions, an denen die eigene Person beteiligt war, und redigiert fremde Discord-IDs in Paaren, Präferenzen und Session-Metadaten; er enthält keine fremden Feedbackantworten. Es werden keine Rohpayloads, Interviewtexte oder DM-Inhalte im Pool gespeichert.

## Prüfung und Grenzen von Paket A

Die Library verwendet parametrisierte `sqlx::query`/`query_as`-Abfragen und explizite Rust-Ergebnistypen. Sie fügt keine `query!`-Makros hinzu; dadurch entstehen keine neuen `.sqlx`-Cacheeinträge. Die neue SQL-Struktur und alle zentralen Datenpfade werden gegen frisch migrierte isolierte PostgreSQL-Datenbanken geprüft.

Paket A liefert keinen Interview-Einhänger, keinen API-Client, keinen Session-HTTP-Endpunkt und keine Website. Diese Teile bleiben B, C, E und D zugeordnet. Eine Live-Prüfung von Interview, Website oder echten API-Werten ist vor deren Umsetzung nicht möglich. Nach dem Merge wird A die Migration und die Erreichbarkeit der Datenschicht belegen.

Die aktuellen Prüfungen umfassen 14 bestandene PostgreSQL-Integrationstests im Crate und drei bestandene Tests für Pool-Export, globale Erasure und Rollback im bestehenden `privacy.rs`. `dl-pool` besteht Clippy mit `-D warnings`. Die vorhandene vollständige Datenschutz-Schemaprüfung meldet elf unzugeordnete Twitch-/Patchnotes-Spalten außerhalb des Pools; alle neuen Pool-Spalten sind eingeordnet. Der identische Fehler wurde gegen den unveränderten Ausgangsstand `47ea7994` reproduziert, dort `privacy.rs:4690`; mit Paket A meldet er dieselben elf fremden Spalten, in `privacy.rs`. Er ist keine neue Pool-Regression.
