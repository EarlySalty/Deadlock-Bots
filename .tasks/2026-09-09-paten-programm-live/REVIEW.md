# Review: Paten-Programm live

status: aktiv
datum: 2026-09-09
branch: feat/paten-programm-live
geprüfter Commit: f9e652f4 (Diff origin/main...HEAD, 4 Commits efc18d3b, 2b18bc7d, 0e9e6ae5, f9e652f4)
Reviewer: adversarial, read-only

## Ergebnis: MÄNGEL

Vier blockierende Mängel, dazu drei Punkte zum Nachbessern und ein Hinweis. Die
Kernmechanik von REQ-3 (Eskalation, Idempotenz, Claim-Guard) und REQ-4, REQ-5,
REQ-6, REQ-8 ist im Code sauber umgesetzt. Blockierend sind: das Feature ist live
inert (Proaktiv-Flag fehlt), die neuen Nutzertexte tragen durchgängig ae/oe/ue
statt echter Umlaute, die vorgeschriebene Onboarding-Options-Beschreibung wurde
nicht geändert, und der reaktive Weg bricht nach einer 24-h-Schließung mit einer
falschen Bestätigung.

## Blockierende Mängel

### 1. REQ-2 nicht geliefert, dadurch REQ-1 live komplett inert  (blockierend)
`scripts/run_dl_bot_service.sh` (nicht geändert) und
`rust/crates/dl-community/src/concierge.rs:3373`

`handle_native_onboarding_completed_inner` steigt bei `!self.config.proactive`
sofort aus (concierge.rs:3373), und `proactive` kommt aus `DL_CONCIERGE_PROACTIVE`
(concierge.rs:311, Default false). Das Live-Startskript
`scripts/run_dl_bot_service.sh` setzt aktuell nur `DL_CONCIERGE_ENABLED=1`
(Zeile 78), kein `DL_CONCIERGE_PROACTIVE`. Ohne dieses Flag feuert der komplette
Frischling-T0-Pfad (REQ-1) nie, das ganze Feature ist im Deploy-Zustand tot.
Zusatzproblem: `scripts/run_dl_bot_service.sh` ist per `.gitignore` ausgeschlossen
(`git check-ignore` bestätigt, `git ls-files` findet es nicht), das Flag kann also
gar nicht über den Branch ausgeliefert werden, obwohl der Contract die Datei im
erlaubten Bereich führt. REQ-2, harte Voraussetzung für REQ-1.
Fix: Der Deployer trägt `export DL_CONCIERGE_PROACTIVE="${DL_CONCIERGE_PROACTIVE:-1}"`
direkt bei `DL_CONCIERGE_ENABLED` in die Live-Datei ein und der Merge/Deploy hält
im Protokoll fest, dass das Flag manuell gesetzt wurde (Datei bleibt gitignored).

### 2. INV-8 flächig verletzt: ae/oe/ue statt echter Umlaute in neuen Nutzertexten  (blockierend)
`assets/paten_leitfaden.toml` (mehrere Zeilen), `assets/welcome_texts.toml:23`,
`rust/crates/dl-community/src/concierge.rs:21-22` (PATE_ESCALATION_24H_CARD_TEXT,
PATE_UNBESETZT_DM_TEXT) und die Render-Labels (render_pate_leitfaden_content:
"So uebernimmst du"), `rust/crates/dl-community/src/team_applications.rs`
(PATE_ACCEPTED_DM_TEXT: "Schoen", "laeuft").

Beispiele: Leitfaden "laeuft", "fuehlt", "Regelverstoessen", "moechte",
"Uebernehmen-Knopf", "hoechstens", "loesen", "oeffentlich"; welcome paten_intro
"Waehlst", "haette", "kuemmere"; 24h-DM "fuer" mehrfach; Karte "Uebernahme";
Accepted-DM "Schoen, dass du dabei bist ... Wie alles laeuft". Alle diese Texte
landen sichtbar beim Nutzer (angepinnte Karte, DMs, Willkommen-Hub). Die
umliegenden Bestandstexte nutzen echte Umlaute (ä ö ü ß), die neuen sind also
inkonsistent und verletzen INV-8 ("mit echten Umlauten").
Fix: Alle neuen Nutzertexte auf echte Umlaute umstellen (läuft, fühlt, möchte,
Übernehmen, höchstens, wählst, hätte, kümmere, für, Übernahme, Schön).

### 3. REQ-7 Teil zwei fehlt: Onboarding-Option 🌱 nicht umformuliert  (blockierend)
`rust/bin/dl-bot/src/serversync.rs:6010` (Datei im Branch unverändert)

REQ-7 verlangt, dass die Beschreibung der Option 🌱 danach lautet "Ich bin ganz
neu und will's lernen, ein Pate aus der Community zeigt mir alles". Im Code steht
weiterhin die alte Zeile (serversync.rs:6010, Wortlaut "Ich bin ganz neu und
will's lernen", danach ein Gedankenstrich, dann "nehmt mich an die Hand").
`serversync.rs` wurde auf dem Branch nicht angefasst; nur die Hub-Sektion
(welcome_publish.rs) ist vorhanden. Damit fehlt der zweite REQ-7-Teil, und die
alte Zeile lässt zusätzlich einen Gedankenstrich stehen (INV-8). Die Datei liegt
im erlaubten Bereich.
Fix: serversync.rs:6010 auf den vorgeschriebenen Wortlaut ohne Gedankenstrich
ändern.

### 4. Reaktiver Weg nach 24-h-Schließung bricht mit falscher Bestätigung  (blockierend)
`rust/crates/dl-community/src/concierge.rs:5498-5507` (request_pate-Guard) und
`concierge.rs` claim_pate_escalation_stage, 24h-UPDATE-Zweig (kein Reset von
`pate_requested`)

Die 24-h-Schließung setzt `status='closed_unbesetzt'`, lässt aber
`concierge_profiles.pate_requested = TRUE` stehen. Die 24-h-DM lädt den Neuling
ausdrücklich ein, sich nochmal zu melden (PATE_UNBESETZT_DM_TEXT:
"schreib mir einfach nochmal, ich bleib dran"). Tut er das per DM, trifft
`request_pate` auf `pate_requested = true` (concierge.rs:5498) und antwortet mit
`PATE_YES_TEXT` (concierge.rs:143): "Dein Patenwunsch ist raus und liegt jetzt
für das Patenteam sichtbar im internen Patenkanal. Sobald sich freiwillig jemand
die Patenschaft schnappt, richten wir für euch beide einen privaten Kanal ein."
Das ist nachweislich falsch: die Anfrage ist geschlossen, es liegt keine Karte
mehr im Kanal, es passiert nichts. Der Bot bricht damit im selben Feature das
Versprechen seiner eigenen Schließungs-DM und lässt den Neuling mit einer
Lügen-Bestätigung stehen. Das verfehlt das REQ-3-Ziel "keine Patenanfrage bleibt
still liegen" und verletzt die Ehrlichkeits-Vorgabe. Bewertung: REQ-3-Mangel,
nicht akzeptabel, weil die 24-h-DM genau diese Aktion einlädt.
Fix: Im 24-h-UPDATE (claim_pate_escalation_stage, TwentyFourHours) zusätzlich
`pate_requested = FALSE` (und pate_request_uncertain = FALSE) für den Nutzer
zurücksetzen, damit ein späterer reaktiver Wunsch eine frische Karte erzeugt.

## Sollte / Hinweis

### 5. Eskalations-Seiteneffekte nach dem Marker sind still best-effort  (sollte)
`concierge.rs` escalate_pate_owner_ping und escalate_pate_unbesetzt

Der Marker (escalated_2h_at bzw. escalated_24h_at + status=closed) wird atomar
VOR dem Seiteneffekt gesetzt. Schlägt danach das Karten-Edit (2h) oder die DM
(24h) fehl, bleibt der Marker gesetzt, es gibt keinen Retry, und die 24-h-DM
schluckt ihr Ergebnis komplett (`let _ = ... send_dm_v2`). REQ-3 "aktualisiert
die Karte einmal" bzw. "genau eine ehrliche DM" wird dann still nicht erfüllt.
Der Implementierer hat den Kompromiss dokumentiert (keine Doppel-DM). Vertretbar,
aber die 24-h-DM sollte ihren Fehlerfall wenigstens loggen statt `let _ =`.

### 6. T2-Nudge nicht gegen pate_offered gesperrt, mögliche Doppel-Ansprache  (sollte)
`concierge.rs:428-450` (cadence_due), T2_NUDGE_TEXT (concierge.rs:138)

`cadence_due` prüft `pate_offered` nicht. Ein Frischling, der beim T0 das
Paten-Angebot bekommen hat (pate_offered=TRUE), aber zwei Tage still bleibt,
erhält per T2 erneut ein Paten-Angebot (concierge:pate:yes/no). REQ-1 sieht das
T2-Angebot nur für die anderen Onboarding-Wahlen vor. Empfehlung: den
Paten-Teil des T2-Nudge unterdrücken, wenn pate_offered bereits gesetzt ist.

### 7. modglue.pin_message ist nicht im Amendment benannt  (Hinweis)
`rust/bin/dl-bot/src/modglue.rs` (neue Methode pin_message auf ConciergePort)

Das Amendment nennt nur `add_role` und `edit_channel_v2`. `pin_message` ist eine
dritte neue Portmethode, additiv und für REQ-5 (Anpinnen) nötig. Die Datei ist
über die Freigabe-Datei genehmigt, das Prinzip "nur additive Methoden" ist
gewahrt; rein formal ist die Methode im Amendment-Wortlaut nicht aufgeführt.
Kein Handlungsbedarf am Code, nur zur Kenntnis.

## Was sauber ist (keine Mängel)

- REQ-1 Frischling-Erkennung: `user_is_frischling` (concierge.rs:498) macht genau
  5 Versuche à `frischling_lookup_retry` (Default 2 s), schläft nicht nach dem
  letzten Versuch; Rolle nie sichtbar oder Lookup-Fehler ergibt false und damit
  den normalen T0 (saubere Degradierung). FRESHLING_T0_PATE_TEXT ist umlautrein.
  Buttons "Ja, gern"/"Nee, ich komm klar" (T2_BUTTON_YES/NO), custom_ids
  concierge:pate:yes/no. pate_offered wird im bestätigten Erfolgspfad gesetzt,
  Journey PateOffered nach Commit. T0 für Nicht-Frischlinge unverändert (t0_body
  nur refaktoriert, identischer Inhalt und Buttons).
- REQ-3 Idempotenz: `claim_pate_escalation_stage` claimt die Stufe atomar
  (bedingtes UPDATE mit `status='open' AND escalated_*_at IS NULL RETURNING`),
  nur der Gewinner sendet; Marker persistiert, kein Doppelfeuern über Neustarts.
  `due_pate_escalations` filtert `status='open'`. 24h-Schließung setzt status,
  closed_at und Marker in einem UPDATE. Opt-out unterdrückt die 24-h-DM
  (escalate_pate_unbesetzt prüft is_opted_out, Fehler wird als opted_out
  gewertet), INV-2 gewahrt. Claim nach Schließung ergibt PATE_REQUEST_CLOSED_TEXT,
  keine Patenschaft; Button-custom_id konsistent mit dem Handler.
- REQ-4: add_role nur bei kind==Pate nach erfolgreichem Status-Update und vor der
  DM; add_role-Fehler bricht mit Mod-Rückmeldung ab, idempotent wiederholbar.
  Andere Kinds unverändert (INV-7). Panel-Button nutzt dl_crown-Brand-Emoji (INV-6).
- REQ-5: ensure_pate_leitfaden postet einmalig, pinnt und speichert message_id und
  Fingerprint; gleicher Fingerprint ergibt no-op, anderer ergibt edit statt
  Doppelpost. Render enthält alle Felder inkl. zehn Server-Ecken, Eskalation,
  Datenschutz.
- REQ-6: Antwort nur bei Mention (bot_mention_question) und nur in
  Paten-Zentrale bzw. aktivem Patenkanal (is_pate_knowledge_channel); ohne
  Mention still, keine persönlichen Buttons (allow_personal_actions=false),
  keine Antwort in fremden Kanälen.
- REQ-8: paten_inventory_line nennt Rollen-Anzahl, offene Anfragen, aktive
  Patenschaften, Leitfaden gepostet ja/nein; in main.rs beim Start geloggt.
- REQ-9: acht neue Verhaltenstests, Rot-vor-Änderung per Sabotage-Gegenprobe in
  TESTS.md plausibel dokumentiert (6+1 FAILED). Keine bestehenden Tests gelöscht
  oder abgeschwächt (einzige geänderte Assertion ALL.len() 6 auf 7 ist die nötige
  Anpassung an das neue Kind). INV-5 gewahrt.
- Migrationen: additiv und idempotent. 18 legt Tabelle plus zwei Indizes an
  (IF NOT EXISTS, Unique-Index "eine offene Anfrage je Neuling"). 19 erweitert
  den kind-CHECK korrekt als Superset (DROP IF EXISTS plus ADD, Ursprungsset in
  2026083101 sind genau die sechs alten Werte), kein Datenverlust. Beide sortieren
  hinter die Bestandsmigrationen.
- Privacy: USER_TABLES um user_id und pate_id von concierge_pate_requests
  erweitert (privacy.rs), und der Scrub-Löschpfad in ConciergeStore löscht die
  neue Tabelle in derselben tx (WHERE user_id=$1 OR pate_id=$1). INV-2-Scrub gedeckt.
- INV-1/INV-3: request_pate (reaktiv) und claim_pate unverändert in ihren Guards
  (nur Paten-Zentrale, Paten-Rolle, Unique-Index); claim_pate nur um den
  Closed-Guard davor ergänzt. Ausnahme: der reaktive Weg ist nach 24-h-Schließung
  faktisch blockiert, siehe Mangel 4.
- Scope: alle geänderten Dateien liegen im erlaubten Bereich plus Amendment
  (modglue.rs, welcome_publish.rs). modglue add_role/edit_channel_v2 sind additiv.

## REQ-Erfüllung

| REQ | Status | Fundstelle |
| --- | --- | --- |
| REQ-1 | teilweise (Code korrekt, live inert ohne REQ-2) | concierge.rs:498 user_is_frischling, t0_body_frischling, set_pate_offered_tx; gated an :3373 |
| REQ-2 | nicht | scripts/run_dl_bot_service.sh (gitignored, Flag fehlt); concierge.rs:311/3373 |
| REQ-3 | teilweise (Mechanik ok, reaktiver Re-Request nach Schließung defekt) | claim_pate_escalation_stage, escalate_pate_*; Mangel 4 concierge.rs:5498 |
| REQ-4 | erfüllt (bis auf INV-8-Text) | team_applications.rs change_status Accept+Pate, add_role; modglue.rs |
| REQ-5 | erfüllt (bis auf INV-8-Text) | concierge.rs ensure_pate_leitfaden, render_pate_leitfaden_content; assets/paten_leitfaden.toml |
| REQ-6 | erfüllt | concierge.rs bot_mention_question, is_pate_knowledge_channel |
| REQ-7 | teilweise (Hub-Sektion ok, Options-Text fehlt) | welcome_publish.rs paten_message ok; serversync.rs:6010 fehlt |
| REQ-8 | erfüllt | aiglue.rs paten_inventory_line; main.rs Startlog |
| REQ-9 | erfüllt | concierge.rs/team_applications.rs Test-Module, TESTS.md |

## Nachreview 2026-09-09

geprüfter Stand: origin/feat/paten-programm-live (Nachdiff f9e652f4..7d3a3a16;
neue Commits 2c2e2920, 517de260, 7d3a3a16)

### Ergebnis: FREIGABE

Alle vier blockierenden Mängel und die zwei Sollte-Punkte sind sauber behoben.

- Mangel 1 (REQ-2): akzeptiert als Deployer-Schritt. `scripts/run_dl_bot_service.sh`
  ist host-lokal und per `.gitignore` ausgeschlossen, das Flag kann nicht über den
  Branch kommen. Nachvollziehbar; der Deployer muss `DL_CONCIERGE_PROACTIVE=1` beim
  Deploy setzen, sonst bleibt REQ-1 inert. Bleibt als Deploy-Auflage bestehen.
- Mangel 2 (INV-8): behoben. Nutzertexte in Leitfaden, welcome paten_intro,
  concierge-Konstanten, Render-Label ("So übernimmst du") und PATE_ACCEPTED_DM_TEXT
  tragen echte Umlaute. Rest-Treffer sind nur interne Fehlerstrings
  ("Zeitlimit ueberschritten", concierge.rs escalate_pate_*), nicht nutzersichtbar.
- Mangel 3 (REQ-7): behoben. serversync.rs:6010 description lautet exakt
  "Ich bin ganz neu und will's lernen, ein Pate aus der Community zeigt mir alles"
  ohne Gedankenstrich. (Das title-Feld darüber trägt weiter einen Gedankenstrich,
  REQ-7 fordert aber nur die Beschreibung; unkritisch.)
- Mangel 4 (REQ-3): behoben. claim_pate_escalation_stage (24h) läuft jetzt in einer
  eigenen Transaktion: atomarer Claim mit RETURNING user_id, danach im selben tx
  `concierge_profiles.pate_requested = FALSE, pate_request_uncertain = FALSE`, dann
  commit. Idempotenz bleibt (WHERE status='open' AND escalated_24h_at IS NULL). Der
  neue Test reaktiver_wunsch_nach_24h_schliessung_erzeugt_neue_anfrage
  (concierge.rs, testing) beweist echtes Verhalten: nach Schließung erzeugt
  request_pate wieder eine Karte im Paten-Kanal und genau eine offene Anfrage.
- Sollte 5: behoben. Die 24-h-DM matcht jetzt alle Delivery-Ausgänge und loggt
  CannotSend50007, Failed und Timeout je mit user_id/stufe statt `let _ =`.
- Sollte 6: behoben. cadence_due hat im T2-Zweig `&& !profile.pate_offered`; der Test
  kadenz_t2_entfaellt_wenn_pate_schon_angeboten_wurde zeigt T2 weg, T7 unberührt.
- Hinweis 7 (pin_message): unverändert, nur zur Kenntnis, kein Handlungsbedarf.

Offene Punkte: keine im Code. Einzige Auflage ist der Deploy-Schritt aus Mangel 1
(DL_CONCIERGE_PROACTIVE=1 in der host-lokalen Startdatei), ohne den REQ-1 live nicht
feuert.

## Nachreview 2, 2026-09-10

geprüfter Stand: origin/feat/paten-programm-live HEAD a6c257eb (Nachdiff
b68da8f2..a6c257eb; neue Commits 82bde414, 4af7406b, a6c257eb)

### Ergebnis: FREIGABE

Alle vier Punkte der zweiten Merge-Kritiker-Runde sind sauber umgesetzt.

- Punkt 1 (Intro-Vorschalt, INV-7): Panel-Buttons aller Bereiche nutzen weiter
  `team_apply:open:<slug>` (team_applications.rs:228/236). Im Handler geht nur
  kind==Pate auf `pate_intro_reply` (ephemer, Knopf "Weiter zum Formular",
  team_applications.rs:351), alle anderen Kinds öffnen wie bisher direkt
  `application_modal(kind)`. Der neue Prefix `team_apply:form:` (Handler ~1640)
  öffnet dann pate_application_modal; kein anderer Bereich erzeugt einen form-Knopf.
  Test pate_vorschalt_zeigt_intro_und_knopf_zum_formular prüft Intro plus
  form:pate-Knopf plus Ephemeral-Flag und dass der Intro-Text in keinem
  Modalfeld irgendeines Kinds steckt. INV-7 gewahrt.
- Punkt 2 (Übernehmen nur für die geklickte Karte): claim_open_pate_request_by_message_tx
  (concierge.rs:1936) fährt `UPDATE ... WHERE message_id = $2 AND user_id = $1
  AND status = 'open' RETURNING id`; die message_id kommt aus der Interaction
  (concierge.rs:6443). Stale Karte oder fremde message_id ergibt 0 Zeilen und
  PATE_REQUEST_CLOSED_TEXT. Fehlende message_id bindet SQL-NULL und schlägt sicher
  fehl (kein Claim). Test uebernehmen_uebernimmt_nur_die_angeklickte_offene_anfrage.
- Punkt 3 (atomarer Claim, Rollback ohne Nebenwirkung): der Claim-UPDATE läuft in
  derselben tx (concierge.rs:6558); bei Ok(false) `drop(tx)` = Rollback, bei Err
  ebenso. Die Kanalanlage (`pate-<user_id>`, create_private) steht erst NACH dem
  erfolgreichen UPDATE, es wird also kein Kanal und keine Patenschaft vor dem
  Claim angelegt; auch der claim_once-Dedup-Eintrag rollt zurück und lässt einen
  sauberen Retry zu. INV-3 intakt: Guild/Kanal-Gate, Paten-Rolle, Last-Limit 3,
  claim_pate bleibt der einzige Weg. Test
  claim_gegen_direkt_uebernommene_anfrage_erzeugt_keine_zweite_patenschaft.
- Punkt 4 (Fingerprint erst nach Pin): ensure_pate_leitfaden setzt beim Neupost
  zuerst message_id, pinnt (concierge.rs:5496) und schreibt den Fingerprint erst
  nach erfolgreichem Pin (concierge.rs:5433). Schlägt der Pin fehl, bleibt der
  Fingerprint leer, message_id ist aber gespeichert, sodass der nächste Start die
  vorhandene Nachricht editiert statt neu zu posten und den Pin nachholt. Kein
  Doppelpost. Test leitfaden_pin_fehler_wird_beim_naechsten_start_nachgeholt.

Die vier neuen Tests sind echte Verhaltenstests (DB-Seeds, Prüfung auf
Kanalanlage, Patenschaften und Anfragestatus bzw. Modal-/Komponenten-Inhalt).
Bestehende Tests wurden nicht verändert oder abgeschwächt (einzige Minus-Zeile ist
ein rustfmt-Umbruch in einem Mock-send_dm, team_applications.rs:1948). INV-5 gewahrt.

Offene Punkte: keine im Code. Es bleibt allein die Deploy-Auflage aus Mangel 1
(DL_CONCIERGE_PROACTIVE=1 in der host-lokalen scripts/run_dl_bot_service.sh).
