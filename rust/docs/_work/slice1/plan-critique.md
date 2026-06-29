# Slice 1 Plan-Kritik

Modus: adversarialer Read-only-Review des Plans
`rust/docs/plans/2026-06-29-coaching-etage-slice1.md` gegen die Slice-1-Faktenbasis und Spec.

Verdikt vorab: **REWORK NOETIG**.

Zaehler: **HIGH 6**, **MEDIUM 9**, **LOW 2**.

## Pruefpunkt-Status

1. AUTH WIRE-COMPAT: Luecken in S1-03. Cookie/JWT-Grundwerte sind im Plan vorhanden, aber echte Cross-Runtime-Tests, OAuth-State-Details, Rollenquellen und Forward-Auth sind nicht hart genug spezifiziert.
2. DB-SCHEMA-PARITAET: Luecken in S1-02. Groesste Luecke ist der fehlende fruehe Beweis der physischen DB-Datei-Identitaet. Zusaetzlich ist die `foreign_keys=ON`-Abweichung von Python nicht entschieden.
3. AI-ENTFERNUNG: Grob abgedeckt in S1-04/S1-05, aber der Bot-Mirror/In-Process-Service-Pfad muss expliziter aus S1-07 herausgetestet werden.
4. PARITAETS-EIGENHEITEN: `GET /requests` bleibt offen, OK. Nicht-idempotente Legacy-Flows sind nur generisch genannt, nicht konkret getestet. Discord-i64 ist adressiert, aber mit einer wichtigen TEXT-Ausnahme.
5. BRIDGE-COLLAPSE-SEQUENZ: Luecke in S1-07/S1-10. Der Plan entfernt HTTP-Loops zu frueh und ohne klaren Live-Fallback.
6. WORKSPACE-KONSISTENZ: Luecke. Neue Crates werden nicht in den richtigen Tickets explizit als Workspace-Member aufgenommen.
7. CADDY-CUTOVER: Path-Split ist im Default korrekt, aber Live-Blast-Radius-Check und Rollback fehlen.
8. DAG/DoD: Teilweise nicht unabhaengig verifizierbar; mehrere `curl`-Checks sind ohne laufenden Server/Auth-Kontext nicht ausfuehrbar.
9. CLIPPY/TEST-SCOPING: OK, keine workspace-weiten Cargo-Kommandos gefunden. Problem ist nur, dass neue Crates vorher Workspace-Member sein muessen.
10. SCOPE: Status-Mutationen/Matching-Cockpit sind sauber out-of-scope. Pflichtpunkte POST+PUT `/api/scrim/participants/me`, `/api/health` und Service-Name sind enthalten.

## HIGH

### H-01: Physische DB-Datei-Identitaet wird nicht frueh bewiesen

- Ticket: S1-01/S1-02, spaeter S1-09.
- Problem: Der Plan entscheidet "geteilte SQLite", beweist aber nicht vor Schema-/Service-Arbeit, dass Rust wirklich exakt dieselbe Datei wie live Python nutzt. In "Offene Fragen" bleibt sogar offen, ob `DEADLOCK_DB_PATH` und Python-`DB_PATH` auseinanderlaufen. Das verletzt die Orchestrator-Entscheidung: gleicher inode muss frueh bewiesen werden, nicht angenommen.
- Risiko: Rust bootstrapt oder schreibt in eine zweite SQLite-Datei. Dann wirken Auth, Coaching, Notifications und Bot-Sync in Tests korrekt, live aber gegen unterschiedliche Daten.
- Fix: In S1-01 oder als blockierende S1-02-Preflight-DoD aufnehmen:
  - Python-DB-Pfad und Rust-DB-Pfad ohne Secret-Werte ermitteln.
  - `stat -Lc '%d:%i %n' <python-db> <rust-db>` muss identische Device/Inode fuer beide Pfade zeigen.
  - Zusaetzlich `sqlite3 <db> 'PRAGMA database_list;'` fuer den tatsaechlich geoeffneten Pfad dokumentieren.
  - Kein `bootstrap_schema()` gegen die Live-Konfiguration, bevor dieser Check bestanden ist.

### H-02: `dl-db`-FK-Semantik kann Python-Paritaet brechen

- Ticket: S1-02/S1-04/S1-05.
- Problem: `map-auth-blast-and-schema.md` warnt, dass Python kein `PRAGMA foreign_keys=ON` setzt. `map-infra-rust.md` beschreibt aber, dass `dl-db` beim Oeffnen `foreign_keys=ON` nutzt. Der Plan nennt diesen Konflikt nicht.
- Risiko: Rust repariert nicht nur Schema, sondern aendert Laufzeitsemantik. Legacy-Flows ohne Existenzchecks koennen scheitern: Match mit unbekanntem Coach, Goals/Notes ohne Coachee-/Session-Existenzcheck, Admin-/Caddy-Sub mit `coach_id=NULL` usw. Python laesst vieles durch, Rust mit FK-Enforcement ggf. nicht.
- Fix: S1-02 muss eine explizite Entscheidung enthalten:
  - fuer die geteilte Website-Coaching-DB keine strengere FK-Enforcement-Semantik als Python, oder
  - falls `foreign_keys=ON` zwingend bleibt, konkrete Paritaetstests fuer alle legacy-orphan-faehigen Inserts und eine freigegebene Abweichung.
  - DoD: `PRAGMA foreign_keys` wird fuer die von `dl-etage` genutzte Verbindung geprueft und die erwartete Semantik getestet.

### H-03: Cross-Decode-Paritaetstest ist nicht echt genug

- Ticket: S1-03.
- Problem: Der Plan fordert nur "Python-kompatibles JWT" und "Python-kompatible Decode-Logik". Das kann komplett in Rust implementiert sein und trotzdem von `python-jose`/Python-Claims-Optionen abweichen.
- Risiko: Bestehende Python-`ddc_session`-Cookies bleiben im Browser, werden von Rust aber nicht akzeptiert. Das ist faktisch Zwangs-Re-Login.
- Fix: S1-03-DoD muss einen echten Cross-Runtime-Test verlangen:
  - Python erzeugt mit Test-Secret ein JWT ueber die gleiche Bibliothek/Logik wie `auth.py`; Rust akzeptiert es.
  - Rust erzeugt ein Session-JWT; Python decodiert es mit Python-Decode-Fallback ohne Aud/Iss-Pflicht.
  - Test nutzt nur Test-Secrets aus isolierter Env, keine Live-Secrets.

### H-04: Bridge-Collapse entfernt HTTP-Loops vor einem sicheren Live-Fallback

- Ticket: S1-07/S1-10.
- Problem: S1-07 verlangt bereits, dass die produktiven HTTP-Aufrufe gegen `WEBSITE_API_BASE` nicht mehr der produktive Pfad sind und `rg` keine Bridge-Loop-Aufrufe mehr findet. S1-10 kommt erst danach. Es gibt keinen Feature-Flag-/Dual-Path-/Rollback-Plan fuer Notifications und Coach-Sync.
- Risiko: Ein Fehler im direkten DB-Pfad nimmt Notifications, Request-Acks oder Coach-Roster-Sync live raus. Ein Rollback erfordert dann Code-Redeploy statt nur Config-/Caddy-Rollback.
- Fix: Sequenz aendern:
  - S1-07 baut den In-Process-Pfad hinter klarer Config/Feature-Flag und testet ihn gegen die geteilte DB.
  - Alte HTTP-Loops bleiben bis zur verifizierten Live-Umschaltung als fallbackfaehiger Pfad erhalten.
  - Entfernung der HTTP-Loops erst nach S1-10-Live-Verifikation oder als eigenes Post-Cutover-Ticket.
  - DoD: waehrend Bot-Restart/Cutover gibt es keinen Zeitraum, in dem weder HTTP noch In-Process Notifications/Acks laufen.

### H-05: Caddy-Cutover hat keinen Live-Blast-Radius-Check und keinen Rollback

- Ticket: S1-10.
- Problem: Der Plan hat nur `rg` als Pre-Cutover-Check. Die Orchestrator-Entscheidung verlangt Live-curl. Rollback ist nicht definiert. Auch der Beweis, dass `:8772` ausser `/coaching/api` nichts Live-Relevantes verliert, ist nicht konkret.
- Risiko: Ein aktiver Legacy-Client oder `Website/builds/frontend` nutzt unter `/coaching/api/*` doch Meta/Auth-Pfade. Der Caddy-Reload bricht live Pfade, ohne vorherigen Beweis und ohne schnelle Rueckkehranweisung.
- Fix: S1-10 erweitern:
  - Vorher: `caddy validate`; aktive Caddy-Datei sichern; konkrete Rollback-Kommandos dokumentieren.
  - Live-curl vor Cutover fuer `/coaching/api/auth/me`, `/coaching/api/coaching/coaches`, `/coaching/api/health` und negative/positive Checks fuer `/coaching/api/{builds,items,heroes,patchnotes,tierlists,history,admin}`.
  - Nachher dieselben curls mit erwarteten Upstreams/Statuscodes.
  - Expliziter Beweis, dass aktive Caddy-Konfig keinen weiteren `:8772`-Pfad ausser dem geplanten Fallback nutzt.

### H-06: Workspace-Member fuer neue Crates sind nicht ticket-sicher

- Ticket: S1-01/S1-03/S1-04.
- Problem: S1-01 umfasst `rust/Cargo.toml` fuer `dl-etage`, aber die Crates `dl-auth` und `dl-mentoring` werden erst in S1-03/S1-04 angelegt. Diese Ticket-Scopes nennen `rust/Cargo.toml` nicht. Trotzdem lauten die DoD-Kommandos `cargo test -p dl-auth`, `cargo clippy -p dl-mentoring ...`.
- Risiko: Die scoped Cargo-Kommandos brechen, weil die Packages nicht Workspace-Member sind. Das ist ein Plan-/DAG-Fehler, kein Implementierungsdetail.
- Fix: Root-`rust/Cargo.toml` in die richtigen Tickets aufnehmen:
  - S1-01: `bin/dl-etage` als Workspace-Member plus Workspace-Dependency-Eintraege, soweit noetig.
  - S1-03: `crates/dl-auth` als Member und Dependency.
  - S1-04: `crates/dl-mentoring` als Member und Dependency.
  - DoD jedes Tickets: `cargo metadata -p <new-package>` oder `cargo package --list -p <new-package>` beweist, dass das Package sichtbar ist, bevor Clippy laeuft.

## MEDIUM

### M-01: OAuth-State-/CSRF- und Relay-Details sind unterdefiniert

- Ticket: S1-03.
- Problem: `map-auth.md` enthaelt genaue Details zu `next`-Normalisierung, Pre-Auth-Cookie, Query-`state_id`-Praeferenz, zentralem `/internal/v1/discord/initiate`/`consume-result`, `metadata.site=builds`, `scope=identify` und der Internal-Token-Kette. Der Plan sagt nur "OAuth" und Callback-Default.
- Risiko: Ein Entwickler "verbessert" CSRF durch Query-Cookie-Gleichheitszwang oder laesst `next`-/Tokenketten-Details weg. Das waere nicht wire-kompatibel.
- Fix: S1-03 muss den exakten OAuth-Relay-Vertrag aus `map-auth.md` aufnehmen und testen, inklusive Fehlerfaelle "Callback ohne/ungueltige Pre-Auth redirectet ohne Session".

### M-02: Rollenquelle und `discord_roles` sind nicht eindeutig als Paritaetsvertrag festgeschrieben

- Ticket: S1-03.
- Problem: Die Pruefliste nennt Guild-Roles->Coach/Admin-Bestimmung. Die Faktenbasis sagt fuer Builds-Auth das Gegenteil: `discord_roles` wird geliefert, aber von `auth.py` ignoriert; Admin kommt aus `meta_users.role`, Coach aus `coaches.discord_user_id/status`.
- Risiko: Je nach Worker-Interpretation entstehen zwei falsche Ports: entweder Discord-Rollen werden neu fuer Admin/Coach genutzt, oder das negative Verhalten wird nicht getestet.
- Fix: S1-03 muss explizit sagen und testen:
  - `discord_roles` aus dem OAuth-Consume-Payload wird fuer Builds-/Coaching-Auth ignoriert.
  - `role` kommt dynamisch aus `meta_users`, JWT nur Fallback.
  - `is_coach` kommt aus `role == admin` oder aktiver `coaches`-Zeile.
  - Falls Orchestrator wirklich Guild-Roles will, muss Spec/Faktenbasis vorher geaendert werden; sonst ist das Divergenz.

### M-03: Loopback-Forward-Auth-Admin fehlt im Auth-Plan

- Ticket: S1-03.
- Problem: `map-auth.md` dokumentiert `X-Admin-Validated: 1` von localhost als Admin-Sonderfall. Der Plan erwaehnt ihn nicht.
- Risiko: Admin-/Caddy-validierte lokale Flows koennen nach Cutover anders reagieren. Auch wenn selten genutzt, ist es Teil der Auth-Helper-Paritaet.
- Fix: S1-03 entweder implementiert und testet diesen Sonderfall oder dokumentiert mit Live-/Caddy-Beweis, dass er unter `/coaching/api/*` nicht relevant ist.

### M-04: Schema-Zielliste ist inkonsistent und nicht als Golden-DDL pruefbar

- Ticket: S1-02/S1-05.
- Problem: Die Pruefliste spricht von 10 Tabellen aus `map-auth-blast-and-schema.md`; `map-coaching-platform.md` und Python `database.py` enthalten fuer Slice 1 aber auch `session_notes`. Der Plan nennt `session_notes`, aber nicht als aufgeloeste Inkonsistenz mit exakter DDL.
- Risiko: Ein Worker kann "10 Tabellen" umsetzen und Notes-Routen brechen, oder eine falsche `session_notes`-Variante bauen.
- Fix: S1-02 muss eine finale Golden-Liste aufnehmen: `meta_users` plus die Coaching/Platform-Tabellen inklusive `session_notes`. DoD: fuer jede Tabelle `PRAGMA table_info`, `PRAGMA foreign_key_list`, `PRAGMA index_list` gegen einen erwarteten Snapshot vergleichen.

### M-05: Discord-i64-Regel braucht TEXT-Ausnahmen

- Ticket: S1-02/S1-05.
- Problem: Der Plan sagt pauschal "Discord-IDs `i64`/SQLite `INTEGER`". In der Python-DDL ist `assigned_coach_id` aber `TEXT` und wird im Platform-Sync als Discord-ID-String gespeichert. Auch `coaches.id` bleibt interne TEXT-ID.
- Risiko: Zu aggressive i64-Normalisierung veraendert Queue-/Reservation-Paritaet.
- Fix: S1-02/S1-05 muessen eine ID-Typ-Matrix enthalten:
  - `discord_user_id`, `discord_channel_id`, Bot-IDs: INTEGER/i64.
  - `assigned_coach_id`: TEXT, obwohl Inhalt eine Discord-ID sein kann.
  - interne IDs (`coaches.id`, `coachees.id`, Goals usw.): TEXT.

### M-06: Nicht-idempotente Legacy-Flows sind nicht konkret getestet

- Ticket: S1-04.
- Problem: Der Plan sagt nur "nicht-idempotente Legacy-Eigenheiten werden dokumentiert getestet". Die kritischen Faelle aus der Faktenbasis sind nicht namentlich in der DoD.
- Risiko: Implementierer "reparieren" die Flows und erzeugen API-Divergenz.
- Fix: S1-04-Tests explizit:
  - `PATCH /requests/{id}/match` darf bei Mehrfachaufruf mehrere Sessions erzeugen.
  - Duplicate `POST /surveys` laeuft gegen `coaching_surveys.session_id UNIQUE` und wird nicht idempotent gemacht.
  - erneutes Admin-Approve kann gegen `coaches.discord_user_id UNIQUE` laufen.
  - `POST /sessions/{id}/end` liefert auch bei 0 betroffenen Zeilen `{"status":"completed"}`.

### M-07: Platform-Routenliste ist nicht vollstaendig konkretisiert

- Ticket: S1-05.
- Problem: Der Plan sagt "Alle 24 Platform-Routen", listet aber Goals/Milestones/Notes nur summarisch. Die Faktenbasis enthaelt konkrete DELETE/PATCH/POST-Pfade.
- Risiko: Ein Worker kann z. B. DELETE Goals/Milestones/Notes oder PATCH Notes uebersehen und trotzdem meinen, "Goals, Milestones, Notes" seien erledigt.
- Fix: S1-05 muss die 24 Routen exakt aufzahlen und pro Route mindestens einen Handler-/Router-Test haben. Besonders: `DELETE /goals/{id}`, `DELETE /milestones/{id}`, `POST/PATCH/DELETE /notes`, `GET/PATCH /coaches/me`.

### M-08: Mehrere Verifikationskommandos sind nicht selbststaendig ausfuehrbar

- Ticket: S1-04/S1-05/S1-06/S1-10.
- Problem: Die Tickets enthalten `curl`-Checks, ohne in jedem Ticket einen laufenden `dl-etage`-Bind oder Testserver zu starten. S1-10 curlt `GET /api/scrim/pool?status=new`, obwohl die Route coach-gated sein soll und ohne Cookie/Coach-Kontext voraussichtlich 401 liefert.
- Risiko: DoD wird entweder uebersprungen oder faelschlich als fehlgeschlagen bewertet.
- Fix: Fuer jedes Ticket:
  - entweder Axum-Routertests ohne externen Port,
  - oder explizites Startkommando mit Test-DB/Test-Env und sauberem Stop.
  - Auth-geschuetzte curl-Checks muessen Test-Cookie oder erwarteten 401/403-Status definieren.

### M-09: AI-Entfernung im Bot-Mirror/In-Process-Pfad ist nicht hart genug

- Ticket: S1-05/S1-07.
- Problem: S1-05 deckt Platform-Sync ab, aber `map-coaching-platform.md` nennt auch den heutigen Rust-Bot-Mirror, der `ai_summary` sendet. Beim In-Process-Umbau in S1-07 darf dieses Feld nicht versehentlich ueber Services weitergeschrieben werden.
- Risiko: AI-Felder bleiben zwar aus HTTP-DTOs raus, werden aber intern weiter in die geteilte SQLite geschrieben.
- Fix: S1-07-DoD ergaenzen:
  - Bot-Mirror-/Service-DTOs nehmen kein `ai_summary`/`ai_insights_json` an oder ignorieren sie.
  - DB-Test: Sync/Mirror schreibt neue Requests mit beiden Spalten `NULL`.
  - Queue/Requests/Platform-Me Responses enthalten keine AI-Felder.

## LOW

### L-01: Cookie-Delete-Varianten sind nicht separat testpflichtig

- Ticket: S1-03.
- Problem: Der Plan nennt Delete-Varianten, aber der Header-Test ist allgemein formuliert.
- Risiko: Logout loescht nur host-only oder nur Domain-Cookie; User bleibt je nach Host weiter eingeloggt.
- Fix: S1-03 Cookie-Testmatrix um Set- und Delete-Header fuer host-only und DDC-Domain-Variante erweitern, inklusive Legacy `auth_token`.

### L-02: TTL-/Cookie-ENV-Overrides sind im Testumfang unklar

- Ticket: S1-03.
- Problem: Der Plan nennt Default-Max-Age 2592000/600, aber `map-auth.md` erlaubt ENV-Overrides fuer Cookie-Namen, TTL, SameSite, Secure und Domain.
- Risiko: Default-Fall passt, Deployment mit Overrides nicht.
- Fix: Mindestens ein Test mit nicht-default `AUTH_COOKIE_NAME`, `AUTH_PRE_AUTH_COOKIE_NAME`, TTL und Domain erzwingen.

## Pflicht-Aenderungen vor Freigabe

1. Fruehen inode-/Device-Beweis fuer Python-DB == Rust-DB in S1-01/S1-02 aufnehmen.
2. FK-Semantik der geteilten SQLite verbindlich entscheiden und testen.
3. S1-03 um echten Python<->Rust-JWT-Cross-Decode-Test ergaenzen.
4. OAuth-State/CSRF/Relay-Details aus `map-auth.md` exakt in S1-03 uebernehmen.
5. Rollenvertrag klaeren: `discord_roles` ignorieren und DB-Rollen testen, oder Faktenbasis/Spec aendern.
6. Workspace-Member-Aenderungen in die Tickets aufnehmen, die neue Packages erzeugen.
7. S1-07 so umbauen, dass HTTP-Bridge-Loops erst nach verifiziertem Live-Direktpfad und mit Fallback entfernt werden.
8. S1-10 um Live-curl-Blast-Radius-Check, Caddy-Validate und expliziten Rollback ergaenzen.
9. Schema-Golden-Liste inklusive `session_notes`, `meta_users`, Defaults, CHECKs, FKs, UNIQUEs und TEXT/INTEGER-Ausnahmen festschreiben.
10. Konkrete Paritaetstests fuer nicht-idempotente Legacy-Flows und alle 24 Platform-Routen ergaenzen.

VERDIKT: **REWORK NOETIG**.
