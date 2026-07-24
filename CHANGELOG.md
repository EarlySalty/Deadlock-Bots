## #290 — Scrim-Lagebilder erreichen OpenAI wieder

Problem: Der Live-Provider lehnte neue Scrim-Lagebild-Anfragen wegen nicht mehr passender Anfrageparameter ab; im Dashboard blieb dadurch nur der vorsichtige Fehlerstand.

Änderung: Moderne OpenAI-Modelle erhalten das kompatible Ausgabelimit und keine von ihnen abgelehnte Sampling-Angabe. Andere angebundene Chatmodelle behalten ihr bisheriges Anfrageformat.

Aktuelles Verhalten: Wochenlauf und Korrekturdialog können wieder echte Scrim-Lagebilder erzeugen; Fehler bleiben weiterhin sichtbar und werden später erneut versucht.

## #289 — Scrim-Orga bleibt auch bei späten Antworten eindeutig

Problem: Späte Button-Antworten, unklare Reminder-Ziele sowie mehrere oder hängen gebliebene Ergebnisabrufe konnten den sichtbaren Planungsstand verfälschen oder blockieren.

Änderung: Freigabe und Antworten sind gegeneinander abgesichert, Reminder prüfen direkt vor dem Post ein wirklich pingbares Ziel, und Ergebnisabrufe behalten erfolgreiche Teilstände bei weiteren Fehlern. Lagebild und sichtbares AI-Entscheidungsprotokoll werden gemeinsam gespeichert.

Aktuelles Verhalten: Operative Scrim-Posts bleiben in den Teamkanälen; hängende Abrufe heilen sich später selbst, ein laufendes Match kann manuell beendet werden, und Teilstände sowie alte Match-IDs bleiben nachvollziehbar sichtbar.

## #288 — Scrim-Lagebild-AI nutzt den vorhandenen OpenAI-Pfad

Problem: Der neue Scrim-Lagebild-Use-Case defaultete auf Mistral, während live nur der bestehende OpenAI-Key gesetzt ist. Dadurch waren Lagebild-Überarbeitung und Bot-Generierung nach dem Neustart inaktiv.

Änderung: `ScrimLagebild` nutzt ohne expliziten Override jetzt den bestehenden OpenAI-Provider. Der Provider-Default ist im vorhandenen `dl-ai`-Config-Test abgesichert.

Aktuelles Verhalten: Web und Bot können die Scrim-Lagebild-AI mit der vorhandenen OpenAI-Konfiguration starten. Ein expliziter `DL_LLM_PROVIDER_SCRIM_LAGEBILD`-Override bleibt weiter möglich.

## #287 — Scrim-Lagebilder im Dashboard korrigieren

Problem: Das Lagebild-Backend konnte Scrim-Teams bereits auswerten, aber Coaches hatten noch keine nutzbare Dashboard-Ansicht für aktuellen Stand, Timeline, Evidenzen und Korrekturdialog.

Änderung: Der Scrim-Tab zeigt pro Team das aktuelle Lagebild, die Snapshot-Timeline, gespeicherte Evidenzen und den dauerhaften Korrektur-Chat. Korrekturen laufen über die bestehende Dashboard-API und bleiben Dashboard-only.

Aktuelles Verhalten: Coaches korrigieren Lagebilder direkt im Dashboard per Freitext. Die letzte Fassung bleibt sichtbar, Evidenzen bleiben nachvollziehbar verlinkt, und aus dem Lagebild heraus wird nichts öffentlich in Discord gepostet.

## #286 — Scrim-Match-IDs holen Ergebnisse ins Dashboard

Problem: Coaches konnten gespielte Scrim-Matches noch nicht einfach mit Deadlock-Match-IDs verknüpfen; Ergebnisabrufe waren dadurch nicht sauber nachvollziehbar.

Änderung: Das Dashboard speichert Match-IDs direkt am Scrim-Match, verhindert doppelte IDs und übergibt sie an den bestehenden Ergebnisabruf. Abrufstatus, Fehler, Rohdaten und normalisierte Ergebnisdaten bleiben sichtbar gespeichert.

Aktuelles Verhalten: Coaches tragen eine Match-ID ein, sie wird automatisch gespeichert und das nächste leere Feld ist bereit. Abgerufene Ergebnisse erscheinen in der Match-History; pending/failed bleibt sichtbar statt still zu verschwinden.

## #285 — Scrim-Match-Status im Teamkanal

Problem: Nach einer Slot-Freigabe gab es im Teamkanal noch keinen verbindlichen Match-Stand, und erneute Updates hätten leicht doppelte Nachrichten erzeugt.

Änderung: Der Bot erstellt nach Freigabe pro Teamkanal eine pingfreie Statusnachricht, speichert die Discord-Message-IDs und nutzt vorhandene Nachrichten für spätere Edits.

Aktuelles Verhalten: Teams sehen Match, finalen Termin, Abstimmung, Zusagen, offene oder unsichere Spieler, Ersatzbedarf, nächste Aktion und den Link zur Terminabfrage direkt im Teamkanal.

## #284 — Scrim-Block-Flow im Dashboard

Problem: Scrim-Match-Abfragen konnten mehrere Matches speichern, aber im Dashboard fehlte ein Arbeitsweg, um einen ganzen Block in einem Schritt anzulegen.

Änderung: Der Scrim-Tab bündelt Blockanlage mit Zwei-Wochen-Vorlage, Paarungen, gemeinsamen Slots und optionalen eigenen Slots pro Match.

Aktuelles Verhalten: Leo kann mehrere Terminabfragen als einen Scrim-Block vorbereiten und gemeinsam anlegen. Reminder, Statusnachrichten und AI bleiben getrennte Schritte.

## #283 — Scrim-Slot kann nach Frist freigegeben werden

Problem: Das Dashboard konnte nach Fristende einen Slot empfehlen, aber Leo brauchte eine verbindliche Freigabe und eine nachvollziehbare manuelle Überschreibung.

Änderung: Terminabfragen speichern den freigegebenen Slot mit Zeitpunkt und Dashboard-Nutzer. Wenn Leo einen anderen Slot wählt, bleibt der optionale Override-Grund getrennt von der Empfehlung sichtbar.

Aktuelles Verhalten: Der Scrim-Tab zeigt empfohlenen Slot, freigegebenen Slot und Override-Grund getrennt; es wird dabei noch keine Discord-Statusnachricht gepostet.

## #282 — Scrim-Dashboard zeigt konkreten Ersatzbedarf

Problem: Nach Fristende war sichtbar, welcher Slot vorne liegt, aber nicht konkret, wer für diesen Slot fehlt oder warum Ersatz nötig ist.

Änderung: Das Dashboard zeigt nach Fristende pro Match, Team und Slot die betroffene Person mit Grund. Berücksichtigt werden Teammitglieder, Slot-Antworten und `Kein Slot passt`; fehlende Rollen-/Lineup-Daten werden als begrenzte Datenlage markiert.

Aktuelles Verhalten: Vor Fristende bleibt Ersatzbedarf leer. Es startet keine automatische Ersatzsuche.

## #281 — Fehlende Scrim-Stimmen können gezielt erinnert werden

Problem: Leo sah offene Stimmen in Terminabfragen, konnte daraus aber noch keinen freigegebenen Reminder auslösen.

Änderung: Das Dashboard bietet bei fehlenden Antworten jetzt eine Reminder-Freigabe pro betroffenem Team. Der Bot postet erst nach dieser Freigabe im Teamkanal, bezieht sich auf die ursprüngliche Terminabfrage und erwähnt nur eindeutig fehlende Mitglieder.

Aktuelles Verhalten: Es gibt keine automatischen Reminder. Jede Reminder-Aktion speichert Ziel, Zeitpunkt, Status, Discord-Nachricht und Fehler für die Orga-Spur.

## #280 — Infisical-Exporter wiederhergestellt

Problem: Patchnotes-Bot und Website-Backend konnten nach einem Restart ihre Secrets nicht mehr laden, weil der gemeinsame Infisical-Exporter beim Python-Aufräumen entfernt wurde.

Änderung: Der gemeinsame Exporter ist wieder Teil des Bot-Repos und sein Startvertrag ist mit einem kleinen Test abgesichert.

Aktuelles Verhalten: Beide Dienste kommen nach einem Restart wieder aus dem Secret-Bootstrap heraus und starten ihren eigentlichen Prozess.

## #279 — Scrim-Dashboard zeigt Terminabfragen mit Slot-Empfehlung

Problem: Scrim-Teams konnten bereits per Button antworten, aber das Dashboard zeigte daraus noch keine zusammengefasste Planungsbasis nach Fristende.

Änderung: Die Scrim-Übersicht liefert jetzt aktuelle Terminabfragen mit Antwortständen, fehlenden Stimmen, `Kein Slot passt` und einer Slot-Empfehlung nach Fristende, wenn beide beteiligten Teams für denselben Slot zugesagt haben. Der Scrim-Tab zeigt diese Auswertung direkt bei den Matches.

Aktuelles Verhalten: Leo sieht im Dashboard, welcher gemeinsame Slot nach den gesammelten Teamantworten vorne liegt oder dass es noch keine gemeinsame Zusage gibt. Reminder, Ersatzbedarf und Freigabe bleiben eigene nächste Schritte.

## #278 — Scrim-Teams antworten per Termin-Button

Problem: Terminabfragen wurden zwar in Teamkanäle gepostet, aber Antworten konnten noch nicht strukturiert über den Bot erfasst werden.

Änderung: Jede Terminabfrage bekommt Slot-Buttons und `Kein Slot passt`. Der Bot nimmt Klicks nur aus der gespeicherten Teamnachricht an und speichert sie nur, wenn der User zum betroffenen Team gehört.

Aktuelles Verhalten: Teamantworten landen strukturiert in der zentralen DB; falsche Teamklicks werden abgewiesen. Fristende-Auswertung und Slot-Empfehlung bleiben der nächste Schritt.

## #277 — Scrim-Terminabfragen gehen in die Teamkanäle

Problem: Angelegte Scrim-Match-Abfragen lagen bisher nur im Dashboard-Speicher. Teams bekamen daraus noch keine konkrete Terminabfrage im eigenen Kanal.

Änderung: Der Bot übernimmt neue Abfrage-Batches, postet je betroffenem Team eine pingfreie Terminabfrage mit Gegner, Frist und Slots und speichert die Nachrichten-IDs für die weitere Auswertung.

Aktuelles Verhalten: Nach dem Anlegen kann der Bot die Terminabfragen in die Teamkanäle bringen; Antwort- und Button-Auswertung bleibt der nächste separate Schritt.

## #276 — Scrim-Lobbycode blockiert Ergebnisaktionen bis zum Bot-Post

Problem: Direkt nach dem Klick auf `Lobby ist offen` konnte ein Ergebnisabruf den noch offenen Lobbycode-Post überschreiben.

Änderung: Der offene Lobbycode gilt jetzt bis zur Bot-Verarbeitung als gesperrter Bot-Zustand.

Aktuelles Verhalten: Erst nachdem der Bot den Lobbycode verarbeitet hat, sind weitere Scrim-Aktionen wieder möglich.

## #275 — Scrim-Dashboard bekommt den Lobbycode-Button

Problem: Der neue Lobbycode-Flow war backendseitig vorhanden, aber im Scrim-Dashboard fehlte noch der sichtbare Arbeitsweg für Coaches.

Änderung: In der Match-Zeile gibt es jetzt ein 5-Zeichen-Feld und den Button `Lobby ist offen`. Der alte `Start`-Button ist aus der UI entfernt.

Aktuelles Verhalten: Coaches geben den Code direkt am Match ein; das Dashboard validiert grob und übergibt an den zielsystem-konformen Lobbycode-Flow.

## #274 — Scrim-Lobbycode läuft über den manuellen Zielsystem-Flow

Problem: Der alte Scrim-Lobby-Pfad konnte eine Custom Lobby vollautomatisch starten und danach lange Discord-Texte wie „Scrim-Lobby steht!" posten. Das passte nicht zum Zielsystem, in dem die Lobby zunächst manuell geöffnet und nur der Code sauber ausgespielt wird.

Änderung: Das Dashboard speichert jetzt einen 5-stelligen Lobbycode am Match, normalisiert ihn auf Großbuchstaben und setzt den Bot-State auf `lobby_open`. Der Bot postet oder editiert daraus in beiden Teamkanälen nur noch die schlichte Textnachricht `Lobby Code: ABC12`. Der alte `/start`-Dashboard-Auslöser startet keine Vollautomatik mehr.

Aktuelles Verhalten: Coaches können den Code als Orga-Fallback setzen; der Bot bringt ihn pingfrei in die Teamkanäle und speichert Message-IDs sowie Korrekturen für die Orga-Spur.

## #273 — Scrim-Match-Abfragen werden im Rust-Dashboard gespeichert

Problem: Leos Scrim-Planung brauchte eine verbindliche Match-Abfrage als Grundlage, aber im produktiven Rust-Dashboard gab es dafür noch keinen Speichervertrag. Teams, Slots, Fristen und Vorlagen konnten deshalb nicht zentral als Abfrage-Batch angelegt werden.

Änderung: Das Dashboard hat jetzt Endpunkte für Match-Abfrage-Defaults und zum Anlegen von Abfrage-Batches. Gespeichert werden Vorlage, Frist, 2-5 strukturierte Slot-Optionen sowie Team A gegen Team B oder ein offener Gegner. Ein Team kann in einem aktiven Abfragezeitraum nicht doppelt verplant werden.

Aktuelles Verhalten: Die zentrale DB nimmt Scrim-Match-Abfragen mit 48-Stunden-Standardfrist und Wochenende-abends-Preset auf; der Web-Service läuft mit der neuen Rust-Version.

## #272 — Coaching-Abschluss reagiert sofort

Problem: Beim Klick auf „Coaching abgeschlossen“ konnte Discord melden, dass der Bot nicht rechtzeitig reagiert hat. War die Session intern schon abgeschlossen, blieb die sichtbare Anfrage trotzdem noch im alten Button-Zustand hängen.

Änderung: Der Klick bestätigt jetzt sofort und zieht die Abschlussarbeit im Hintergrund nach. Erkennt der Bot eine bereits abgeschlossene Session, setzt er die sichtbare Anfrage erneut auf abgeschlossen und entfernt die Buttons.

Aktuelles Verhalten: Coaches bekommen keine Discord-Timeout-Meldung mehr; das Coaching-Embed zeigt nach dem Klick zuverlässig den abgeschlossenen Zustand.

## #271 — Python-Altstand liegt nur noch im Archiv

Problem: Nach dem Rust-Cutover lag der alte Python-Stand weiter im Hauptrepo. Dadurch war nicht klar, was noch produktiv ist und was nur noch als Referenz dient.

Änderung: Der alte Stand wurde im Backup festgehalten und aus dem laufenden Repo entfernt. Die Dienste laden ihre Geheimnisse jetzt über den Rust-Weg.

Aktuelles Verhalten: Das Hauptrepo zeigt den aktuellen Rust-Betrieb, während der entfernte Altstand im Backup nachvollziehbar bleibt.

## #270 — Scrim-Aufnahmen landen direkt im Teamkanal

Problem: Scrim-Teams mussten ihre Sprachaufnahme bisher selbst organisieren, lokal speichern und anschließend manuell verteilen.

Änderung: Teammitglieder und Coaches können die Aufnahme in den vier festen Team-Sprachkanälen mit `/record start` und `/record stop` steuern. Ein Einwilligungshinweis macht den Start sichtbar; leere Kanäle und das 60-Minuten-Limit beenden die Aufnahme automatisch.

Aktuelles Verhalten: Nach dem Stop erscheint die MP3 im zugehörigen Team-Textkanal; scheitert die Bereitstellung, meldet der Bot stattdessen den Fehler. Zwei Teams können gleichzeitig aufnehmen; bei fehlender Kapazität gibt der Bot einen klaren Hinweis.

## #269 — Interne Discord-Nachrichten lassen sich fristgerecht entfernen

Problem: Interne Dienste konnten Review-Nachrichten über den zentralen Bot versenden und bearbeiten, aber nicht einzeln löschen. Eine eigene Aufbewahrungsfrist ließ sich deshalb nicht zuverlässig auch auf die Discord-Kopie anwenden.

Änderung: Der zentrale Discord-Zugang nimmt jetzt authentifizierte Löschaufträge für eine konkrete Kanal- und Nachrichten-ID an. Derselbe Auftrag ist wiederholbar; eine bereits fehlende Nachricht gilt als erledigt.

Aktuelles Verhalten: Interne Dienste können ihre abgelaufenen Discord-Kopien entfernen, ohne selbst einen Discord-Token zu erhalten.

## #268 — Verworfenes aus dem Tagesplan bleibt sichtbar

Problem: Wenn das Zweitgehirn einen geplanten Punkt wegen fehlender Belege aussortierte, verschwand er aus der Ansicht. Damit war schwer nachzuvollziehen, warum ein Lauf weniger Vorschläge lieferte als erwartet.

Änderung: Aussortierte Punkte werden mit ihrem Grund dauerhaft zum jeweiligen Planlauf gespeichert und im Dashboard getrennt von den angenommenen Punkten angezeigt.

Aktuelles Verhalten: Der Tagesplan bleibt streng belegt, zeigt aber transparent, welche Vorschläge verworfen wurden und weshalb.

## #267 — Sprachkanäle alle an einem Ort

Problem: Der Knopf zum Erstellen eigener Sprachkanäle saß in einer eigenen Kategorie ganz woanders, während die fertigen Lanes weiter unten bei Chill auftauchten. Wer den Server nicht auswendig kennt, hat den Einstieg schlicht nicht gefunden.

Änderung: Verwaltung, Erstellen-Knopf und Off Topic Voice sitzen jetzt oben in der Chill-Kategorie, direkt über den Lanes. Die Reihenfolge untereinander bleibt genau wie gewohnt.

Aktuelles Verhalten: Alles rund um Sprachkanäle steht jetzt in einer einzigen Kategorie, von oben nach unten: verwalten, erstellen, Off Topic Voice, danach die laufenden Lanes. Off Topic Voice bekommt außerdem wieder automatisch einen zweiten Kanal dazu, sobald zwei Leute drin sitzen.

## #266 — Neue Server-Tour beim Onboarding

Problem: Neue Mitglieder mussten sich alles selbst zusammensuchen. Wie die Sprachkanäle funktionieren, wo man Fragen stellt oder Hilfe bekommt, das stand zwar irgendwo, aber niemand hat es einem gezeigt.

Änderung: Beim Beitritt gibt es jetzt die Frage „Willst du Starthilfe?". Dort kannst du dir den echten Rang einrichten lassen, eine kleine Server-Tour per DM bekommen, oder beides. Die Tour erklärt Schritt für Schritt die wichtigsten Ecken des Servers, und nach jedem Schritt kannst du direkt Fragen stellen, die der Bot beantwortet.

Aktuelles Verhalten: Wer die Tour anhakt, bekommt nach dem Start eine DM und klickt sich in seinem Tempo durch sechs kurze Stationen, von den Sprachkanälen bis zu den Streamern. Wer nichts anhakt, merkt keinen Unterschied.

## #265 — Gelöschte Profile bleiben gelöscht

Problem: Wer seine Daten löschen ließ, konnte durch einen Nebenweg wieder angelegt werden. Der Bot liest regelmäßig Server-Protokolle mit und legte dabei Namen erneut an, ohne zu prüfen, ob die Person eine Löschung beantragt hatte.

Änderung: Die zentrale Stelle, an der Profile geschrieben werden, prüft jetzt zuerst den Löschvermerk. Liegt einer vor, wird nichts geschrieben. Das gilt für alle Wege, die auf Profile schreiben, nicht nur für den Auslöser.

Aktuelles Verhalten: Eine beantragte Löschung hält. Kein Hintergrundprozess kann ein gelöschtes Profil wiederbeleben.

## #264 — Verlorene Steam-Verknüpfungen finden zurück

Problem: Im Februar hat der Bot bei einem fehlerhaften Abgleich angenommen, über zweihundert Mitglieder hätten den Server verlassen, obwohl sie da waren. Er hat ihre Steam-Verknüpfung gelöst und sie von seiner Freundesliste entfernt. Über hundert von ihnen liefen seitdem ohne Rang-Rolle herum, ohne zu wissen warum.

Änderung: Der Bot hält jetzt fest, warum eine Verknüpfung gelöst wurde. Hat er sie selbst gekappt, gilt das Mitglied als Rückkehrer und bekommt beim nächsten Besuch in einem Sprachkanal automatisch eine neue Freundschaftsanfrage.

Aktuelles Verhalten: Wer damals betroffen war, wird wieder angefragt und hat seinen Rang nach dem Bestätigen zurück. Angefragt wird nur, wer im Sprachkanal aktiv ist, und frühestens alle 30 Tage erneut. Wer die Verknüpfung selbst über den Knopf im Panel entfernt, bleibt entfernt.

## #263 — Steam-Panel bekommt einen Knopf zum Entfernen

Problem: Das Steam-Panel zeigte nur den Weg hinein. Wer seine Verknüpfung wieder loswerden wollte, musste einen Slash-Befehl kennen, den kaum jemand kennt.

Änderung: Das Panel hat jetzt einen vierten, roten Knopf zum Entfernen der Verknüpfung. Er fragt vorher nach und zeigt, was entfernt wird.

Aktuelles Verhalten: Verknüpfen und Entfernen liegen an derselben Stelle. Beim Entfernen löst der Bot auch die Steam-Freundschaft auf, nicht nur den Eintrag bei uns.

## #262 — Sprachkanäle vererben keine Moderationsrechte mehr

Problem: Beim Anlegen eines Sprachkanals hat der Bot die Rechte der übergeordneten Kategorie eins zu eins übernommen. Stand dort ein Häkchen zu viel, konnte in diesen Kanälen jedes Mitglied andere stummschalten oder herumschieben, ganz ohne Mod-Rolle. Genau so wurden am 17. Juni Leute stummgeschaltet, die es bis heute waren.

Änderung: Rechte wie Stummschalten, Verschieben, Kicken oder Bannen werden beim Übernehmen jetzt grundsätzlich herausgefiltert. Entzogene Rechte bleiben unangetastet, und jede herausgefilterte Berechtigung landet als Warnung im Log.

Aktuelles Verhalten: Moderieren kann nur noch, wer die passende Rolle hat. Ein falsch gesetztes Häkchen an einer Kategorie kann diese Rechte nicht mehr an neue Sprachkanäle weiterreichen.

## #261 — Ein Logout beendet beide Admin-Sitzungen

Problem: Discord- und Twitch-Admin-Dashboard teilten zwar einen Cookie, konnten die dahinterliegende Sitzung aber getrennt weiterführen.

Änderung: Der zentrale Login-Dienst kann eine gemeinsame Admin-Sitzung jetzt auf den geschützten internen Auftrag des Twitch-Dashboards widerrufen.

Aktuelles Verhalten: Ein Logout entfernt die gemeinsame Sitzung zentral, sodass derselbe Cookie nicht in einem der beiden Dashboards weiterverwendet werden kann.

---

## #260 — Turniervorschläge bleiben in Mod-Hand

Problem: Die Turnier-Automatik konnte einen Wochenplan selbst live schalten und öffentlich ankündigen. Änderung: Im Mod-Kanal gibt es jetzt einen vorher gespeicherten KI-Vorschlag mit J, N samt Pflichtgrund und Änderungswunsch; parallele Änderungen laufen nacheinander und alte Buttons finden automatisch die aktive Version. Aktuelles Verhalten: Die interne Übergabe ist auf Loopback plus Token begrenzt; erst zwei unterschiedliche Mods legen das Turnier an, danach erscheint die Vorlage in derselben Karte. Scheitert dieses Edit, bleibt es offen und wird bei einem erneuten J nochmals versucht.

---

## #259 — Voice-Trennungen bleiben dauerhaft aktiv

Problem: Andere Rechte-Updates des Bots konnten eine laufende feste Voice-Trennung aus einem Kanal wieder entfernen.

Änderung: Aktive Trennungen werden jetzt bei jedem vollständigen Rechte-Update erneut eingemischt und mit Besitzer-Bans gemeinsam verarbeitet.

Aktuelles Verhalten: Die beiden betroffenen Personen können in keinem Sprachkanal zusammenkommen; Besitzerrechte, Rang-Updates und Bot-Neustarts umgehen die Sperre nicht.

---

## #258 — Gelöschte Daten bleiben gelöscht

Problem: Wer seine Daten über den Datenschutz-Befehl gelöscht hat, bekam einen Teil davon still zurück. Mehrere Hintergrundprozesse haben weitergeschrieben, sobald die Person wieder im Sprachkanal saß: Aktivitätsmuster, Mitspieler-Verbindungen, Mitspielersuche, Sprachstatistik und Umfrage-Einladungen.

Änderung: Alle diese Schreibvorgänge prüfen den Widerspruch jetzt genau in dem Moment, in dem sie schreiben, statt vorher. Übersprungene Schreibvorgänge werden protokolliert.

Aktuelles Verhalten: Eine Löschung hält. Wer widersprochen hat, taucht in keiner dieser Auswertungen wieder auf, auch nicht, wenn er weiter auf dem Server aktiv ist.

---

## #257 — Feste Voice-Trennungen gelten überall

Problem: Eine feste Trennung konnte in anderen Sprachkanälen oder über die Rechte eines TempVoice-Owners umgangen werden.

Änderung: Der Bot sperrt den jeweils belegten Sprachkanal jetzt serverweit für die andere betroffene Person und nimmt die Sperre beim Wechsel oder Verlassen wieder zurück.

Aktuelles Verhalten: Beide Seiten haben dieselben Regeln, andere Sprachkanäle bleiben nutzbar und niemand wird nachträglich getrennt oder verschoben.

---

## #256 — Der Concierge redet wieder wie ein Mensch

Problem: Auf Fragen hat der Concierge zuletzt wörtliche Doku-Abschnitte gepostet. Inhaltlich korrekt, aber es klang nach Handbuch statt nach Gespräch, und bei Smalltalk kam immer derselbe Standardsatz.

Änderung: Der Concierge formuliert seine Antworten jetzt wieder selbst, in seinem eigenen Ton. Die Fakten kommen weiterhin ausschließlich aus unserer geprüften Server-Doku, er erfindet also nichts dazu. Weiß die Doku etwas nicht, sagt er das ehrlich und schickt dich zu einem echten Menschen.

Aktuelles Verhalten: Fragen zum Server beantwortet er frei formuliert auf Basis der Doku, Smalltalk beantwortet er natürlich statt mit Textbaustein. Falls die freie Stimme mal klemmt, fällt er automatisch auf den wörtlichen Doku-Text zurück.

---

## #255 — Mitspielersuche einfach in den Channel schreiben

Problem: Die Mitspielersuche über Forum-Posts hat kaum jemand genutzt. Wer Leute suchte, fand keine, und die Suche verlief im Sand.

Änderung: Künftig reicht eine normale Nachricht im Suche-Channel („suche 2 für Ranked heute Abend"). Der Bot liest Rang, Uhrzeit, Anzahl und Modus selbst heraus und sucht passende Leute, die zuletzt aktiv waren und zu der Zeit üblicherweise online sind. Fehlt die Uhrzeit, fragt er einmal kurz nach.

Aktuelles Verhalten: Die Funktion ist eingebaut, aber noch nicht angeschaltet. Die Einladungen an passende Mitspieler starten, sobald der Zustellweg über den Concierge steht, mit hartem Limit, damit niemand zugespamt wird.

---

## #254 — Schnellere Auswahl passender Doku-Passagen

Problem: Die reine Auswahl fester Doku-Passagen lief unnötig in lange Reasoning-Timeouts und bekam Kandidaten, die der Wissensdienst anschließend sicher ablehnen musste.

Änderung: Für diesen Auswahlpfad ist Reasoning abgeschaltet, und sicher ungültige Kandidaten werden bereits vor der Modellanfrage entfernt.

Aktuelles Verhalten: Antworten bleiben bei einer Modellanfrage, die auf sieben Sekunden begrenzt ist, und bestehen weiterhin nur aus festen öffentlichen Passagen.

---

## #253 — Eindeutige Antwortwege für Support-Fragen

Problem: Im Support konnten sich mehrere Antwortwege überschneiden, sodass !brain doppelt oder ohne öffentliche Doku-Grundlage antwortete.

Änderung: Brain reagiert nur noch in einem ausdrücklich freigegebenen Bot-Spam-Kanal; FAQ und Concierge haben in privaten Bereichen jeweils eine exklusive Zuständigkeit.

Aktuelles Verhalten: Pro Ort ist genau ein Antwortweg zuständig, sodass Support-Fragen nicht mehr parallel von mehreren Bots beantwortet werden.

---

## #252 — Antworten kommen nur aus festen Doku-Passagen

Problem: Kopierte Textausschnitte des Sprachmodells konnten trotz Prüfung an Satz- und Überschriftsgrenzen unnötig abgelehnt werden.

Änderung: Das Modell wählt jetzt nur noch nummerierte öffentliche Absätze, Listen oder Tabellenzeilen; den Antworttext und die Quellen setzt der Wissensdienst selbst aus diesen festen Passagen zusammen.

Aktuelles Verhalten: Fragen zu Discord und den Bots erhalten ausschließlich belegte Doku-Passagen, während unbekannte, doppelte, zu lange oder themenfremde Auswahlen vollständig verworfen werden.

---

## #251 — Frageformen blockieren keine passende Steam-Antwort

Problem: Richtige Steam-Passagen wurden bei „Welche“-Fragen abgelehnt, wenn die Frageform erst später im gefundenen Dokument stand.

Änderung: Die rein grammatischen Formen von „welche“ zählen nicht mehr als thematischer Beleg; Steam, Verwaltung und der Bezug zu den eigenen Konten bleiben verbindlich.

Aktuelles Verhalten: Fragen zur eigenen Steam-Verwaltung werden aus der passenden öffentlichen Passage beantwortet, themenfremde Texte weiterhin abgelehnt.

---

## #250 — „Kann“ blockiert keine passende FAQ-Antwort

Problem: Richtige FAQ-Passagen wurden bei Fragen mit „kann“ abgelehnt, wenn das Wort erst später im gefundenen Dokument stand.

Änderung: „kann“ zählt nicht mehr als thematischer Beleg; die übrigen Inhaltsanker und die vollständige Belegprüfung bleiben unverändert.

Aktuelles Verhalten: Fragen zum Stellen einer Server- oder Bot-Frage werden aus der passenden öffentlichen Passage beantwortet, themenfremde Texte weiterhin abgelehnt.

---

## #249 — FAQ-Chat wird eindeutig erkannt

Problem: Eine Frage zum privaten FAQ-Chat wurde trotz exakter öffentlicher Passage abgelehnt, weil „Fragechat“ und „FAQ-Chat“ unterschiedlich verglichen wurden.

Änderung: Die enge Kompaktform wird wie andere Bot-Namen in ihre Bestandteile zerlegt; beliebige Wörter sowie Twitch- und Voice-Kontext bleiben getrennt.

Aktuelles Verhalten: Fragen zum Öffnen über „Frage stellen“ oder /faq werden aus der öffentlichen Doku beantwortet.

---

## #248 — „Kurz:“ als Strukturgrenze erkannt

Problem: Der Support-Agent fand die richtige Hilfeseite, lehnte aber einen vollständigen Antwortsatz direkt nach dem sichtbaren Label „Kurz:“ ab.

Änderung: Das feste Einleitungslabel wird jetzt als Strukturgrenze erkannt; beliebige Doppelpunkt-Texte bleiben unberührt.

Aktuelles Verhalten: Zusammen mit der selbsttragenden öffentlichen Doku beantwortet der Support-Agent die Frage zum Community-Team zuverlässig.

---

## #247 — Absätze nach Überschriften werden erkannt

Der Support-Agent fand zwar die richtige Hilfeseite, verwarf aber den ersten vollständigen Absatz nach einer Überschrift ohne Satzzeichen.

Der Validator erkennt diese strukturelle Grenze jetzt korrekt.

Der folgende Absatz kann als Antwort dienen; die Überschrift allein weiterhin nicht.

---

## #246 — Concierge antwortet aus geprüftem Supportwissen

**Ausgangslage:** Fragen zu Discord und den Bots konnte der Concierge nicht belastbar beantworten, und es war unklar, was er dabei speichert oder an Menschen weitergibt.

**Was wurde geändert:** In DMs, im festen Fragenkanal und in privaten FAQ-Chats antwortet er nur noch aus dem veröffentlichten Supportwissen; unsichere oder undokumentierte Fälle gehen an Menschen, und aus Tickets entstehen höchstens interne Prüfkandidaten statt direkter Bot-Antworten oder Diagnosen.

**Wie es jetzt läuft:** „stopp“ beendet nur in einer echten DM an den Concierge neue Concierge-Verlaufsspeicherung und ungefragte Concierge-Kontakte, direkte Fragen beantwortet er danach ohne Verlauf; „vergiss mich“ löscht nur seine Concierge-Daten, während der Datenschutz-Befehl der umfassendere Weg bleibt. Einen Patenwunsch bestätigt er erst nach der internen Übergabe und nicht doppelt; unklare Discord-Zustände kennzeichnet er sichtbar, statt möglicherweise vorhandene Antworten oder Kanäle blind zu löschen, und Debug, Neustarts oder andere Live-Aktionen führt er nicht aus.

---

## #245 — Spam-Korrektur-Buttons überleben jetzt jeden Neustart

**Problem:** Die Lern-Buttons unter Twitch-Spam-Meldungen wurden nach einem Bot-Neustart tot und meldeten nur noch „Dieser Lernfall ist nicht mehr aktiv".

**Änderung:** Der Twitch-Bot lernt jetzt selbst und meldet, was er gelernt hat. Die Buttons korrigieren nur noch: „Als harmlos korrigieren" macht ein gelerntes Muster rückgängig, „Als Spam korrigieren" lernt eines nach. Alles Nötige steckt im Button selbst, es gibt keinen internen Zwischenspeicher mehr.

**Aktuelles Verhalten:** Korrekturen funktionieren auch Tage später noch, egal wie oft der Bot neu gestartet wurde. Alte Buttons aus früheren Meldungen antworten mit einem freundlichen Hinweis statt mit einem Fehler.

---

## #244 — Neue Sprachkanäle heißen sofort so, wie ihr sie haben wollt

**Ausgangslage:** Der Bot hat jeden neuen Kanal erst mit einem Standardnamen angelegt und euren gespeicherten Wunschnamen ignoriert. Den musstet ihr danach selbst über das Panel anwenden. Discord erlaubt aber nur zwei Umbenennungen pro zehn Minuten, also war eine davon schon weg, bevor ihr überhaupt etwas gemacht habt.

**Änderung:** Habt ihr eine Voreinstellung gespeichert, wird euer Name direkt bei der Erstellung gesetzt. Außerdem hat das Anwenden einer Voreinstellung bisher zwei Umbenennungen statt einer verbraucht, das ist jetzt behoben.

**Aktuelles Verhalten:** Der Kanal trägt von der ersten Sekunde an euren Namen. Eure beiden Umbenennungen gehören wieder euch, nicht dem Bot.

## #243 — Einladungs-Panel entfernt, Admins laden per Befehl ein

**Ausgangslage:** Das Panel für Playtest-Einladungen war der einzige Weg ins Spiel und lief komplett ohne menschlichen Kontakt. Wer im Trichter feststeckte, bekam davon nichts mit.

**Änderung:** Das Panel und die zugehörigen Knöpfe sind entfernt. Stattdessen gibt es einen Einladungs-Befehl, den nur Admins sehen und benutzen können. Die Antwort darauf sieht ebenfalls nur der Admin.

**Aktuelles Verhalten:** Wer eine Einladung möchte, postet seinen Steam-Freundescode und wird von einem Admin persönlich eingeladen. Alte Panel-Nachrichten reagieren nicht mehr, sie werden entfernt.

## #242 — Router erklärt sich beim ersten Mal per DM

**Ausgangslage:** Wer zum ersten Mal in den Deadlock Router ging, wurde nicht automatisch in eine Lane sortiert und wusste oft nicht warum. Der Router braucht erst einen gewählten Modus, und das war vielen nicht klar.

**Was wurde geändert:** Beim ersten Router-Besuch ohne gespeicherten Modus schickt der Bot einmalig eine DM, die erklärt wie der Router funktioniert. Den Modus setzt man direkt in der DM und lässt sich mit einem Klick auf Fertig sofort seine eigene Lane bauen.

**Wie es jetzt läuft:** Modus einmal wählen, Fertig drücken, ab dann sortiert der Router bei jedem Join automatisch ein. Name, Limit und Rang passt man direkt in der DM oder jederzeit im Panel an.

## #241 — Freundescode-Tipp nur noch für neue Mitglieder

**Ausgangslage:** In der Invite-Lounge entschied allein der Text darüber, ob der Bot den Freundescode-Hinweis schickt. Wer jemandem beim Invite helfen wollte und dabei "einladen" schrieb, bekam den Tipp ebenfalls, obwohl er seit Monaten auf dem Server ist.

**Was wurde geändert:** Der Hinweis geht nur noch an Leute, die seit weniger als sieben Tagen auf dem Server sind. Lässt sich das Beitrittsdatum nicht bestimmen, bleibt der Bot still.

**Wie es jetzt läuft:** Neue Mitglieder werden weiterhin an den Steam-Freundescode erinnert, alle anderen können in der Invite-Lounge ungestört antworten und helfen.

## #240 — Coaching-Rolle kommt wieder direkt nach Website-Anfrage

**Ausgangslage:** Neue Coaching-Anfragen über die Website wurden zwar im Coaching-Bereich sichtbar, aber die anfragende Person bekam die aktive Coaching-Rolle nicht mehr direkt. Dadurch fehlten Schreibrechte.

**Was wurde geändert:** Nach einer erfolgreich geposteten Anfrage wird die aktive Coaching-Rolle wieder vergeben und mit Ablaufzeit gespeichert.

**Wie es jetzt läuft:** Wer Coaching über die Website startet, kann direkt im Coaching-Bereich schreiben; die Rolle läuft wie bisher automatisch ab.

## #239 — Concierge meldet sich nur noch auf Ansprache

**Ausgangslage:** Solange am Concierge noch gebaut wird, soll er niemanden von sich aus anschreiben, auch nicht mit der Begrüßung beim Beitritt.

**Was wurde geändert:** Alle selbst gestarteten DMs sind vorerst aus, inklusive der Willkommensnachricht. Er antwortet weiterhin ganz normal, sobald ihm jemand direkt schreibt.

**Wie es jetzt läuft:** Wer eine Frage hat, schreibt ihn einfach an und bekommt Antwort. Von allein bleibt er still, bis er wieder freigeschaltet wird.

## #238 — Concierge drängt sich nicht mehr auf

**Ausgangslage:** Der Concierge meldete sich nach kurzer Zeit von selbst mit einer Glückwunsch-Nachricht, obwohl das gerade noch nicht gewollt ist. In der Roomtour war der Deadlock Router nur fett geschrieben statt anklickbar, und der Vorstellungs-Vorschlag kippte manchmal in eine sinnlose Liste aus lauter Verboten.

**Was wurde geändert:** Die selbständigen Nachfass-Nachrichten sind vorerst abgeschaltet, der Concierge reagiert nur noch. Der Deadlock Router ist jetzt ein klickbarer Kanal. Der Vorstellungstext wird enger geführt und ein kaputter Vorschlag durch einen sauberen Standardtext ersetzt.

**Wie es jetzt läuft:** Er begrüßt dich und antwortet, wenn du schreibst, fängt aber nicht mehr von allein an. Der Router lässt sich direkt aus der Tour antippen, und die vorgeschlagene Vorstellung liest sich wie von dir.

## #237 — Concierge verweist auf den richtigen Fragekanal

**Ausgangslage:** Einige sichtbare Concierge-, FAQ- und Regelwerk-Texte verwiesen allgemeine Bot- und Serverfragen noch in den Community-Fragekanal. Support-Tickets und Invite-Fragen waren dadurch nicht sauber getrennt.

**Was wurde geändert:** Server-, Bot- und Concierge-Fragen zeigen jetzt auf den dafür vorgesehenen Fragekanal. Support und Moderation bleiben im Ticketkanal, Invite-Fragen bleiben im Community-Fragekanal.

**Wie es jetzt läuft:** Der Concierge hilft nur aus der Doku. Wenn er nichts Sicheres weiß, verweist er auf den Server-/Bot-Fragekanal; im Ticket antwortet er nur, wenn er wirklich helfen kann.

## #236 — Tickets verlinken wieder den Ticketkanal

**Ausgangslage:** Mehrere sichtbare Hinweise auf Tickets führten auf den Bereich für Server-Unterstützung, obwohl dort Boosts, Spenden und Unterstützer-Extras hingehören.

**Was wurde geändert:** Ticket-, FAQ-, Regelwerk- und Schnellstart-Verweise zeigen jetzt auf den echten Ticketkanal. Der Bereich für Server-Unterstützung wird wieder als Spenden-/Boost-Bereich beschrieben.

**Wie es jetzt läuft:** Wer Hilfe oder Moderation braucht, landet beim Ticketkanal. Wer den Server finanziell unterstützen will, bleibt im separaten Unterstützerbereich.

## #235 — Concierge bleibt im Wissenspfad

**Ausgangslage:** Wenn die öffentliche Doku nichts fand, konnte der Concierge wieder in eine freie KI-Antwort fallen. Dabei wurden Prompt-Tests, Links, knapper Smalltalk oder Alias-Schreibweisen wie „steambot" nicht sauber genug behandelt.

**Was wurde geändert:** Der Concierge antwortet jetzt nur noch lokal, aus der öffentlichen Wissensbasis oder aus vorhandenem Brain-Wissen. Ohne belastbaren Kontext gibt es den klaren Hinweis auf menschliche Hilfe. Die Wissenssuche versteht außerdem typische Nutzer-Schreibweisen wie „steambot", „twitchbot", „heroes" und „verknuepfen". Sichtbare Antworten werden wieder mit echten Umlauten formuliert. Fragen nach der Heldenanzahl werden direkt aus den öffentlichen Heldenseiten gezählt.

**Wie es jetzt läuft:** Prompt-Injection, Technikfragen und fremde Aufgaben bleiben kurze Concierge-Konter. Normale Serverfragen werden zuerst lokal gesucht, und Lücken fallen kontrolliert aus statt frei erfunden zu werden.

## #234 — Concierge nutzt öffentliche Doku zuerst

**Ausgangslage:** Der Concierge konnte bei Architektur-, Modell- oder Code-Fragen zu lange am KI-Verlauf hängen. Öffentliche Serverfragen wurden außerdem nicht immer direkt aus der Wissensbasis beantwortet.

**Was wurde geändert:** Der Concierge blockt interne Technik- und Code-Aufgaben lokal und fragt bei allen normalen Fragen zuerst die öffentliche Wissensbasis. Findet sie eine sichere Antwort, wird diese direkt genutzt.

**Wie es jetzt läuft:** Fragen aus der öffentlichen Doku werden schneller und ohne Chat-Verlauf beantwortet. Interne Bot-Details, Terminal- und Code-Aufgaben bleiben beim kurzen Concierge-Konter.

## #233 — Voice-Texte sagen Sprachkanal statt TempVoice

**Ausgangslage:** Einige Hinweise im Sprachkanal-Panel sprachen noch von TempVoice oder Lane. Gerade bei Ablehnungen war dadurch nicht sofort klar, dass man dafür einfach in einem Sprachkanal sein muss.

**Was wurde geändert:** Die sichtbaren Panel-Titel, Fußzeilen und Ablehnungen verwenden jetzt Sprachkanal als Begriff. Der Hinweis beim Rang-Gate und bei anderen Aktionen sagt direkt, dass man dafür in einem Sprachkanal sein muss.

**Wie es jetzt läuft:** Nutzer sehen im Voice-Panel durchgehend den Sprachkanal-Begriff und bekommen bei falschem Kontext eine klare Ansage.

## #232 — FAQ-Helfer ignoriert Nachrichten ohne Server-Kontext

**Ausgangslage:** Einzelne Nachrichten ohne Server-Zuordnung konnten im FAQ-Ticket-Helfer einen internen Fehler auslösen.

**Was wurde geändert:** Der FAQ-Helfer verarbeitet Ticket-Autohilfe nur noch, wenn Discord einen echten Server mitliefert.

**Wie es jetzt läuft:** Nachrichten ohne Server-Kontext werden ignoriert statt einen Hintergrundtask zu crashen.

## #231 — Concierge startet nur bei echtem Onboarding

**Ausgangslage:** Wenn bestehenden Mitgliedern später Rollen gegeben wurden, konnte der Concierge sie wie neue Servermitglieder behandeln. Bei gesperrten DMs entstand dann ein privater Fallback-Kanal.

**Was wurde geändert:** Der Bot wertet Discord-Onboarding jetzt nur noch als Auslöser, wenn das Onboarding-Flag wirklich neu dazukommt. Reine Rollenänderungen zählen nicht mehr.

**Wie es jetzt läuft:** Coaching- oder andere Rollen können bestehenden Mitgliedern gegeben werden, ohne dass der Concierge neu loslegt.

## #230 — Voice-Panel nutzt wieder das Lane-Erstellen-Bild

**Ausgangslage:** Das neu erzeugte Voice-Panel zeigte im Bereich „Lane erstellen" wieder das Deadlock-Router-Bild, obwohl dort der Lane-Erstellen-Divider hingehört.

**Was wurde geändert:** Der Panel-Aufbau verwendet für den ersten Bereich wieder das Lane-Erstellen-Bild und lädt es zusammen mit dem Lane-Verwalten-Bild aus.

**Wie es jetzt läuft:** Neue oder aktualisierte Voice-Panels zeigen oben „Lane erstellen" und darunter „Lane verwalten" mit den passenden Bildern.

## #229 — Voice-Lanes bleiben beim Namen ruhig

**Ausgangslage:** Chill-Lanes konnten nach Beitritten, Wechseln oder Spielstatus-Updates wieder automatisch auf generische Namen wie „Lane 1" springen.

**Was wurde geändert:** Automatische Umbenennungen durch Status-Updates, Join/Leave-Events und Auto-Owner-Wechsel sind abgeschaltet. Manuelle Änderungen, Voreinstellungen und bewusstes Owner-Übernehmen bleiben aktiv.

**Wie es jetzt läuft:** Ein Lane-Name ändert sich nur noch durch eine Nutzeraktion wie Umbenennen, Preset/Modus anwenden oder Owner-Claim; Match-/Lobby-Erkennung läuft weiter intern, schreibt aber nicht mehr in den Kanalnamen.

## #228 — Voice-Panel erkennt vorhandene neue Panels wieder

**Ausgangslage:** Im Sprachkanal-Verwalten-Kanal konnte beim Bot-Neustart ein zweites neues Panel entstehen, obwohl schon eins da war.

**Was wurde geändert:** Der Bot liest die vorhandenen Panel-Nachrichten jetzt direkt so aus, wie Discord sie liefert, und erkennt dadurch auch die neuen Komponenten-Panels zuverlässig.

**Wie es jetzt läuft:** Beim Neustart wird das vorhandene Panel aktualisiert statt daneben noch einmal neu gepostet.

## #227 — Coaching-Abschlüsse schließen Anfrage und Website-Stand

**Ausgangslage:** Abgeschlossene Coachings konnten im Anfrage-Post weiter wie laufende oder offene Anfragen wirken. Dadurch passte auch der Stand, den die Website aus dem Bot-Spiegel bekam, nicht zuverlässig zum echten Abschluss.

**Was wurde geändert:** Wenn eine Coaching-Session endet oder abgebrochen wird, aktualisiert der Bot jetzt auch den ursprünglichen Anfrage-Post sichtbar auf abgeschlossen bzw. abgebrochen und entfernt die Aktionsknöpfe.

**Wie es jetzt läuft:** Coachings wirken nach dem Ende im Discord abgeschlossen, und der Website-Abgleich bekommt beim Abschluss den abgeschlossenen Status mit.

## #226 — Voice-Panel wird kompakter und Owner-Claim übernimmt Presets

**Ausgangslage:** Im Voice-Panel war die ausführliche Anleitung weiterhin direkt in der Nachricht sichtbar, und die Modus-Knöpfe waren farbig. Beim Owner-Claim blieb die Lane außerdem auf alten/default Werten statt auf dem Standard-Preset des neuen Owners.

**Was wurde geändert:** Das Panel zeigt nur noch den Anleitungsknopf; die Details öffnen sich dort mit dem Anleitung-Bild. Casual, Ranked und Street Brawl sind neutral grau. Wenn jemand eine Lane übernimmt, wird direkt das gespeicherte Standard-Preset dieses neuen Owners auf die Lane angewendet.

**Wie es jetzt läuft:** Der Kanal bleibt schlank, die Anleitung erscheint nur bei Bedarf, und übernommene Lanes wechseln Name, Limit und Modus auf den neuen Owner.

## #225 — Voice-Panel zeigt Anleitung und Rang-Gate sauber

**Ausgangslage:** Im Sprachkanal-Verwalten-Kanal lagen altes und neues Panel nebeneinander. Das übersichtlichere Panel hatte die bessere Button-Reihenfolge, aber das neue Rang-Gate fehlte dort, und die Anleitung war als eigene Nachricht zu präsent.

**Was wurde geändert:** Das bestehende Panel bekommt oben neben den Voreinstellungen eine ausführliche Anleitung per Knopf und unten beim Verwalten den Rang-Gate-Knopf. Die Anleitung öffnet sich im neuen goldenen Komponenten-Look mit Bild und erklärt die Voice-Lanes und Buttons im Detail.

**Wie es jetzt läuft:** Der Kanal kann wieder auf das kompakte Panel reduziert werden: Anleitung nur bei Bedarf, Rang-Gate direkt im Verwalten-Bereich.

## #224 — Der Bot antwortet dir jetzt richtig per DM

**Ausgangslage:** Wer dem Bot eine DM geschrieben hat, bekam oft nur ein Standard-Menü mit Buttons zurück. Für echte Fragen musste man in den Server ausweichen.

**Was wurde geändert:** DMs an den Bot beantwortet jetzt der neue Concierge direkt und natürlich. Er kennt sich mit dem Server aus, merkt sich euer Gespräch und wenn er etwas nicht sicher weiß, schickt er dich zu den Leuten, die es wissen. Das alte Button-Menü ist raus. Wenn du nicht willst, dass er sich etwas merkt, sag ihm einfach stopp oder vergiss mich.

**Wie es jetzt läuft:** Schreib dem Bot einfach eine DM wie einem Menschen. Neue Mitglieder begrüßt er ab jetzt auch von selbst und hilft beim Ankommen.

## #223 — Voice: Aufräumen greift jetzt immer + neue ⚙️-Ansicht

**Ausgangslage:** Leere Lanes mit eigenem Namen (z. B. „Team Kekse") blieben manchmal als Geisterkanäle stehen — der Aufräumer hat sie am Namen nicht mehr als Bot-Lane erkannt. Und die ⚙️ Voreinstellungen kamen als schlichte Textwand daher.

**Was wurde geändert:** Der Aufräumer erkennt Bot-Lanes jetzt an ihrem Platz in der Voice-Kategorie statt am Namen — egal wie du deine Lane nennst, sie wird nach dem Verlassen abgebaut. Die zwei festen Kanäle in der Kategorie bleiben davon unberührt. Die ⚙️ Voreinstellungen stecken jetzt im gleichen goldenen Look wie das Voice-Panel, mit klarer Übersicht zu Modus, Name, Limit und Rang.

**Wie es jetzt läuft:** Lane leer = Lane weg, ausnahmslos — und beim Bot-Start werden liegengebliebene Geisterkanäle gleich mit entsorgt.

## #222 — Brain-Ingestion-Migration ist im Hauptzweig

**Ausgangslage:** Die zentrale Brain-Postgres-Migration fuer die fehlenden Ingestion-Tabellen war produktiv angewandt, lag aber nur auf einem separaten Branch.

**Was wurde geändert:** Der Hauptzweig enthaelt jetzt die idempotente Migration fuer die ergaenzenden `brain.*`-Tabellen.

**Wie es jetzt läuft:** Neue Deploys haben die Schema-Provenance im Repo; auf bereits migrierten Systemen laeuft die Migration ohne neue Datenbewegung durch.

## #221 — Voice-Umbau: Alle Lanes in einer Kategorie, Ranked offen

**Ausgangslage:** Ranked-Voice hatte eine eigene Kategorie mit Dauer-Schloss für alle Unverifizierten — das hat viele vom Joinen abgehalten, und drei getrennte Voice-Kategorien machten es unübersichtlich.

**Was wurde geändert:** Alle Lanes (Ranked, Casual, Street Brawl) stehen jetzt in einer gemeinsamen, offenen Kategorie — oben Ranked nach Rang sortiert, darunter Casual, unten Street Brawl. Ranked-Lanes kann jetzt jeder ohne Verifizierung erstellen und joinen. Neu ist das 🔓 Rang-Gate: Der Lane-Owner kann seine Ranked-Lane exklusiv für verifizierte Ränge in einem Fenster machen (Standard: eigener Rang ±1,5 Ränge) — erst dann gibt es ein Schloss, und nur an dieser Lane. Wer schon drin ist, fliegt dabei nie raus.

**Wie es jetzt läuft:** Der Name sagt, was in einer Lane läuft (*Ranked Phantom 3*, *Chill Lane 2 · Oracle*), die Sortierung führt dich zu Runden auf deinem Niveau, und ob eine Lane offen oder rang-exklusiv ist, entscheiden die Spieler selbst.

## #220 — Datenschutz-Hinweis im Regelwerk per Knopf

**Ausgangslage:** Wie der Bot Nachrichten per KI prüft, was dabei gespeichert wird und welche Rechte du hast, stand bisher nirgends kompakt im Server.

**Was wurde geändert:** Das Regelwerk hat einen neuen Knopf „🔒 Datenschutz". Ein Tipp darauf zeigt dir – nur für dich sichtbar – kurz und klar: KI-Moderation, Speicherung samt 90-Tage-Löschung und deine Rechte inklusive `/datenschutz` und Kontakt.

**Wie es jetzt läuft:** Der Hinweis ist jederzeit im Regelwerk abrufbar, ohne den Chat vollzumüllen.

## #219 — Moderationsdaten werden nach 90 Tagen gelöscht

**Ausgangslage:** Bei einem Regelverstoß speichert der Bot den gemeldeten Nachrichtentext zur Nachvollziehbarkeit. Bisher blieb dieser Inhalt unbegrenzt liegen.

**Was wurde geändert:** Ein täglicher Job entfernt den gespeicherten Nachrichteninhalt aus Moderationsfällen automatisch nach 90 Tagen; die anonyme Fall-Statistik (Kategorie, Aktion, Zeitpunkt) bleibt erhalten.

**Wie es jetzt läuft:** Inhalte verschwinden nach 90 Tagen von selbst. Wer sofort alles löschen will, nutzt weiterhin `/datenschutz`.

## #218 — Fireworks-Moderation antwortet strikt als JSON

**Ausgangslage:** Fireworks erkannte grenzwertige Moderationsfälle, konnte aber trotz Prompt mit Fließtext antworten. Der Bot wertete das als Parse-Fehler und ignorierte den Fall.

**Was wurde geändert:** Der Fireworks-Aufruf nutzt jetzt den JSON-Modus der Chat-API.

**Wie es jetzt läuft:** Textanalyse-Antworten sind stabil parsebar; die nachgelagerte GPT-nano-Prüfung bleibt unverändert.

## #217 — Twitch-Spam-Lernen läuft im Live-Bot

**Ausgangslage:** Die Lernknöpfe für verdächtige Twitch-Spam-Alerts waren vorbereitet, aber der laufende Rust-Discord-Bot konnte die Klicks noch nicht selbst an den Twitch-Bot weitergeben.

**Was wurde geändert:** Der Live-Bot hängt die Lernknöpfe jetzt an passende Alerts, prüft die Mod-Rechte beim Klick und schreibt die Entscheidung über die interne Twitch-Schnittstelle in die Spam- oder Harmlos-Lernliste.

**Wie es jetzt läuft:** Mods können neue Spam-/Scam-Maschen direkt aus dem Discord-Alert als positiv oder negativ lernen lassen. Nach einem erfolgreichen Klick werden die Knöpfe deaktiviert.

## #216 — Discord-Moderation nutzt Fireworks statt Minimax

**Ausgangslage:** Die Textanalyse der Discord-Moderation lief noch über Minimax, obwohl dafür ein Fireworks-Schlüssel bereitsteht.

**Was wurde geändert:** Die Textanalyse läuft jetzt über Fireworks mit DeepSeek V4 Flash. Die zusätzliche Prüfung über GPT nano bleibt unverändert aktiv.

**Wie es jetzt läuft:** Neue Discord-Moderationsfälle nutzen Fireworks für den ersten Textblick und weiterhin GPT nano für Bildanalyse und Verifikation.

## #215 — Twitch-Spam-Alerts haben Lernknöpfe

**Ausgangslage:** Verdächtige Twitch-Spam-Meldungen kamen im Discord an, aber Mods konnten aus der Meldung heraus nicht zurückmelden, ob das Muster wirklich Spam oder harmlos war.

**Was wurde geändert:** Solche Meldungen bekommen jetzt zwei Lernknöpfe. Berechtigte Mods können den Fall als Spam oder als harmlos bestätigen; der Bot schreibt das über die interne Twitch-Schnittstelle in die vorhandenen Lernlisten.

**Wie es jetzt läuft:** Neue Spam-/Scam-Maschen lassen sich direkt aus dem Discord-Alert trainieren. Nach erfolgreichem Klick werden die Knöpfe deaktiviert, damit derselbe Fall nicht mehrfach entschieden wird.

## #214 — Moderation löscht Takeover-Bilder kanalübergreifend

**Ausgangslage:** Beim Account-Takeover-Alarm erkannte der Bot zwar Bilder in mehreren Kanälen, löschte aber nur die auslösende Nachricht. Im Mod-Fall standen die Bilder außerdem als Original-Links; nach dem Löschen konnten diese Links ins Leere laufen.

**Was wurde geändert:** Der automatische Vollzug löscht jetzt alle Nachrichten, die zum erkannten Takeover-Muster gehören. Die Bildbeweise werden vor dem Löschen geladen und als echte Anhänge an den Mod-Post gehängt.

**Wie es jetzt läuft:** Offtopic, Coaching-Lane und andere betroffene Kanäle werden gemeinsam bereinigt. Mods sehen die Bilder dauerhaft im Fall-Post, nicht nur als kaputte Original-Links.

## #213 — Twitch-Live-Posts im neuen Layout möglich

**Ausgangslage:** Die Ankündigungen, wenn jemand auf Twitch live geht, steckten im alten, starren Nachrichten-Format fest.

**Was wurde geändert:** Der Bot kann solche Posts jetzt im neuen, aufgeräumten Discord-Layout ausliefern. Alle anderen Bot-Nachrichten — Moderation, Hinweise, DMs — bleiben genau wie vorher.

**Wie es jetzt läuft:** Twitch-Live- und Offline-Posts erscheinen im neuen Look; der Rest ist unverändert.

## #212 — Moderation: keine 100%-Warnung mehr auf ganz normale Nachrichten

**Ausgangslage:** Wer in kurzer Zeit in zwei Kanälen was gepostet hat — etwa ein Bild in team-1 und eine Nachricht in team-2 — konnte einen Moderationsfall auslösen, der dann auch noch stur „Verifikation 100%" behauptete. Dabei hatte gar keine KI den Inhalt angeschaut. Das traf völlig harmlose, alteingesessene Accounts.

**Was wurde geändert:** Das Verhaltensmuster (schnelles Posten über mehrere Kanäle) ist jetzt nur noch ein Vorfilter — es sagt „hier lohnt sich ein Blick", entscheidet aber nichts mehr allein. Erst schaut die KI wirklich auf den Inhalt. Ist alles harmlos, passiert nichts. Ist tatsächlich was dran, kommt der Fall mit der echten Trefferquote statt einer erfundenen 100%.

**Wie es jetzt läuft:** Normale Aktivität löst keinen Fehlalarm mehr aus. Die Prozentzahl im Fall entspricht ab jetzt dem, was die KI wirklich gesehen hat.

## #211 — Sprachkanal-Panel entschlackt + Bann mit Mitglieder-Suche

**Ausgangslage:** Das Panel hatte 16 Buttons — einiges doppelt, einiges nur für Spezialfälle. Und bannen ging nur, solange der Störer noch mit dir in der Lane saß.

**Was wurde geändert:** Anleitung und Mitspieler-Suche kommen jetzt ohne eigene Buttons aus — Gesuche und 🔔 Benachrichtigungen laufen direkt über den Forum-Post, der im Panel verlinkt ist; ⚙️ Voreinstellungen gibt's nur noch einmal oben. Beim Bann öffnet sich eine Mitglieder-Suche: Name eintippen, auswählen, fertig — auch nachträglich, wenn der Störer längst weg ist oder du gerade in keinem Sprachkanal bist.

**Wie es jetzt läuft:** Deutlich weniger Knöpfe bei gleichen Funktionen. Deine Bann-Liste gilt wie gehabt für alle deine Lanes, bis du sie wieder aufhebst.

## #210 — Voice-Lanes: aufgeräumtes Panel, gemerkter Modus, Voreinstellungen

**Ausgangslage:** Im Sprachkanal-Verwalten-Kanal war alles ein Riesen-Block, beim Router-Join musste jedes Mal neu gewählt werden, und Lane-Einstellungen gingen nur, wenn man schon in einer Lane saß.

**Was wurde geändert:** Der Kanal ist neu sortiert — Anleitung oben (Details per Button, nur für dich sichtbar), dann Mitspieler finden, Lane erstellen, Lane verwalten; dieselben Panels hängen jetzt auch angepinnt im Router-Chat und als Post im Mitspieler-Suche-Forum. Deine erste Modus-Wahl wird als Standard gespeichert: Ab dann landest du beim Router-Join sofort in deiner eigenen Lane. Über ⚙️ Voreinstellungen legst du Modus, Lane-Name und Limit jederzeit fest — auch ohne in einer Voice zu sein.

**Wie es jetzt läuft:** Einmal wählen, ab dann geht's direkt in deine Lane — Standard ändern jederzeit über ⚙️. Und die 30-Sekunden-Wartezeit beim Lane-Erstellen ist weg.

## #209 — Invite-Fragen ziehen nach frag-die-community

**Ausgangslage:** Für „Ich brauche einen Deadlock-Invite" gab es einen eigenen Kanal — ein Kanal mehr, den Neue erst finden mussten, obwohl die Community-Fragen sowieso woanders laufen.

**Was wurde geändert:** Der Invite-Kanal ist ins Archiv gewandert. Invite-Fragen stellst du jetzt einfach in frag-die-community — der Bot-Tipp mit dem Steam-Freundescode kommt dort genauso, und Regelwerk, FAQ und Willkommens-Hub zeigen alle auf den neuen Weg.

**Wie es jetzt läuft:** Ein Kanal weniger. Egal ob Frage oder Invite-Wunsch: frag-die-community ist die eine Anlaufstelle — nett fragen, Freundescode dazu, jemand lädt dich ein.

## #208 — Regelwerk zum Anklicken, Server-FAQ & Unterstützer-Info im neuen Look

**Ausgangslage:** Das Regelwerk war eine lange Textwand, die Server-FAQ gab's seit dem Umbau gar nicht mehr, und die Unterstützer-Infos lagen als alte Textnachricht im Support-Kanal.

**Was wurde geändert:** Das Regelwerk hat jetzt Buttons — Kurzfassung oben, Details (Verhalten, Ton & Trash-Talk, Moderation, wichtige Kanäle) holst du dir per Klick, und die Antwort siehst nur du. Im Support-Kanal gibt's eine neue FAQ mit den 15 häufigsten Fragen als Auswahlmenü — von „Wie bekomme ich einen Invite?" bis „Wo finde ich Mitspieler?". Die Unterstützer-Infos (Boost & Spende) sind im gleichen Stil neu aufgesetzt.

**Wie es jetzt läuft:** Frage auswählen oder Button klicken → Antwort kommt sofort und nur für dich. Kein Scrollen durch Textwände mehr.

## #206 — Mitspieler-Suche: „Wann?" + Benachrichtigung bei Treffer

**Ausgangslage:** In der Suche stand nur, wer spielt und mit welchem Rang — nicht, wann du überhaupt Zeit hast. Und wer gerade selbst kein Gesuch postet, verpasst passende Lobbys komplett.

**Was wurde geändert:** Im Gesuch gibt's jetzt ein grobes „Wann?" (jetzt / heute Abend / Wochenende / flexibel). Neu ist außerdem der Button „Benachrichtige mich": du trägst Modus, Rang und Zeitfenster ein und bekommst eine DM, sobald ein passendes Gesuch auftaucht.

**Wie es jetzt läuft:** Die Benachrichtigung ist rein freiwillig, einmalig und läuft von selbst ab. Ob sie feuert, richtet sich auch danach, wann du üblicherweise online bist — kein Ping zur Unzeit.

## #205 — Mitspieler-Suche: Rang per Klick statt Tippen

**Ausgangslage:** Im Gesuch-Formular musstest du deinen Rang als Text eintippen („z. B. Archon bis Phantom"). Wer den genauen Rang-Namen nicht kennt, tippt schnell was Falsches — und ungenaue Eingaben brachen auch die automatischen Filter-Tags.

**Was wurde geändert:** Rang und Plätze wählst du jetzt per Dropdown. Nach dem Modus kommt ein „Von"- und ein „Bis"-Menü mit allen Rängen (samt Rang-Emojis) plus die freien Plätze — kein Tippen mehr, keine Tippfehler. „Von = Bis" heißt genau ein Rang, „Rang egal" lässt's offen.

**Wie es jetzt läuft:** Modus wählen → Von/Bis-Rang und Plätze anklicken → **Suche veröffentlichen**. Die Filter-Tags kommen wie gehabt automatisch aus deiner Auswahl.

## #204 — Ranked-Lanes & -Gesuche erkennen deinen Rang wieder

**Ausgangslage:** Wer eine Ranked-Lane aufmachen oder ein Ranked-Gesuch in der Mitspieler-Suche erstellen wollte, wurde abgewiesen („Für Ranked brauchst du einen verifizierten Rang") — auch wenn das Steam-Konto längst verknüpft war. Der Check suchte nach alten Rang-Rollen, die es nach dem Umbau des Rang-Systems gar nicht mehr gibt.

**Was wurde geändert:** Der Ranked-Check erkennt jetzt die aktuelle Steam-Verifiziert-Rolle, die beim Verknüpfen vergeben wird. Wer verifiziert ist, kommt wieder durch — im Router-Panel genauso wie in der Mitspieler-Suche.

**Wie es jetzt läuft:** Steam verknüpft = Ranked-Lane und Ranked-Gesuch gehen wieder. Ohne Verknüpfung führt der Weg wie gehabt in den Rang-Kanal.

## #203 — Voice-Lanes: die alten ➕-Kanäle sind ins Archiv gewandert

**Ausgangslage:** Mit dem Router-Panel gibt es seit Kurzem einen Weg, per Klick eine eigene Voice-Lane zu öffnen. Die alten ➕-Kanäle („Lane eröffnen", „Ranked", „Street Brawl") liefen übergangsweise parallel weiter.

**Was wurde geändert:** Der neue Weg hat sich bewährt — die drei ➕-Kanäle sind jetzt ins Archiv verschoben und gesperrt. Eine Lane machst du ab sofort nur noch über das Router-Panel auf, oder indem du dich in den **Deadlock Router** setzt.

**Wie es jetzt läuft:** Modus im Panel klicken (oder in den Router-Kanal springen) — der Rest ist wie gehabt. Lanes, die gerade offen sind, laufen ganz normal weiter.

## #202 — Mitspieler-Suche: Filtern nach Modus, Rang und Status

**Ausgangslage:** Im neuen Forum wuchsen die Gesuche, aber es gab keinen schnellen Weg, nur die relevanten zu sehen — etwa „nur Ranked auf meinem Level, das gerade aktiv ist".

**Was wurde geändert:** Jedes Gesuch bekommt jetzt automatisch Tags — abgeleitet aus dem, was du im Formular eh angibst: Modus (Casual/Ranked/Street Brawl), grober Rang-Bereich (Einsteiger/Fortgeschritten/Erfahren/Elite) und ein Status, der live mitläuft (Jetzt aktiv, solange eine Lane offen ist / Sucht noch, wenn nicht). Du tippst nichts extra, der Bot setzt die Tags selbst. Oben im Forum kannst du danach filtern.

**Wie es jetzt läuft:** Gesuch wie gewohnt erstellen — die Tags kommen von allein. Über die Tag-Leiste filterst du auf genau das, was du suchst.

## #201 — Onboarding zeigt dir den Weg zum echten Rang

**Ausgangslage:** Beim Server-Beitritt erfuhr niemand, dass sich der echte Deadlock-Rang automatisch verknüpfen lässt — die Anleitung im Rang-Kanal musste man zufällig finden. Die offizielle Verknüpfungs-Rolle direkt im Onboarding zu vergeben, erlaubt Discord Bots grundsätzlich nicht.

**Was wurde geändert:** Das Onboarding hat jetzt eine vierte, optionale Frage: ob du deinen echten Rang automatisch bekommen willst. Wer sie ankreuzt, bekommt nach dem Start eine kurze DM von uns mit dem direkten Sprung zur Drei-Schritte-Anleitung im Rang-Kanal — die Erinnerungs-Markierung räumen wir danach automatisch wieder ab.

**Wie es jetzt läuft:** Frage ankreuzen, Onboarding abschließen, DM aufmachen, drei Schritte durchklicken — ab dann hält der Server deinen Rang von selbst aktuell. Wer keine DMs zulässt, findet den Weg weiterhin direkt im Rang-Kanal.

## #200 — Mitspieler-Suche ist jetzt ein lebendiges Forum

**Ausgangslage:** Die Mitspieler-Suche war ein Textkanal — Gesuche verschwanden im Verlauf, und man wusste nie, ob ein „wer noch Ranked?" von vor zehn Minuten noch aktuell ist.

**Was wurde geändert:** `🎯mitspieler-suche` ist jetzt ein Forum. Du erstellst dein Gesuch per kurzem Formular (Modus, Rang-Bereich, freie Plätze) — entweder direkt im Forum oder mit einem Klick aus deiner eigenen Voice-Lane heraus. Das Besondere: Der Post zeigt live, wie viele Plätze in deiner Lane noch frei sind, ein „Beitreten"-Knopf zieht andere direkt zu dir rein, und sobald die Lane voll ist oder sich auflöst, schließt sich der Post von selbst. Der alte Textkanal ist ins Archiv gewandert.

**Wie es jetzt läuft:** Formular ausfüllen, optional gleich eine Lane aufmachen, und der Post hält sich von allein aktuell — kein Friedhof aus veralteten Gesuchen mehr.

## #199 — Steam verknüpfen: ein klarer Weg in drei Schritten

**Ausgangslage:** Im Rang-Kanal erklärten eine ältere Textnachricht und ein einfaches Embed die Steam-Verknüpfung — funktional, aber unübersichtlich: viel Fettdruck, gleich aussehende Textblöcke und ein verwirrendes Nebeneinander von Bot-Verknüpfung und der offiziellen Discord-Verknüpfung „Steam Verifiziert✅".

**Was wurde geändert:** Die Anleitung ist jetzt ein einziger Weg in drei Schritten, jeder mit eigenem Banner im Server-Look: Steam anmelden, Freunde werden, Rang aktiv. Der Text startet mit dem, was du davon hast (Rang-Rolle, Ranked-Zugang, Mitspieler-Suche), nutzt unsere Server-Emojis und ist deutlich ruhiger formatiert. Das offizielle Discord-Siegel „Steam Verifiziert✅" ist nur noch ein kurzer Bonus-Hinweis statt eines eigenen Schritts. Auch die Erinnerung per DM an Voice-Stammgäste ohne Verknüpfung erzählt jetzt dieselbe Geschichte statt der alten Text-Wand.

**Wie es jetzt läuft:** Drei Schritte durchklicken, danach hält der Server deinen Rang von selbst aktuell.

## #198 — Voice-Lanes per Klick: das neue Router-Panel

**Ausgangslage:** Eigene Voice-Lanes gab es bisher nur über die ➕-Kanäle, verwaltet wurde in separaten Verwaltungs-Kanälen, und im Router-Kanal erklärten zwei alte Nachrichten einen Vorlieben-Flow, den kaum jemand genutzt hat.

**Was wurde geändert:** Im Router-Kanal steht jetzt ein einziges Panel im Server-Look mit Bannern: Drei Modus-Buttons (Casual, Ranked, Street Brawl) erstellen dir sofort eine eigene Lane in der passenden Kategorie und ziehen dich automatisch rüber — für Ranked brauchst du wie gewohnt einen verifizierten Rang. Direkt darunter verwaltest du deine Lane per Button (Owner übernehmen, Limit, Umbenennen, Kick/Ban/Unban, Modus wechseln), und eine ausführliche Anleitung erklärt alles im Detail. Gegen Klick-Spam gibt es eine kurze Abklingzeit, und wer schon eine eigene Lane hat, wird auf Modus wechseln und Umbenennen verwiesen statt eine zweite zu bekommen.

**Wie es jetzt läuft:** In einen Sprachkanal setzen (am einfachsten in den Deadlock Router), Modus klicken, fertig. Die ➕-Kanäle laufen übergangsweise parallel weiter, bis sich der neue Flow bewährt hat.

## #197 — 🧭willkommen ist da: der Server auf einen Blick

**Ausgangslage:** Nach dem Struktur-Umbau (#195) fehlte noch der versprochene Startpunkt — ein Ort, der Neuen wie alten Hasen zeigt, wo was ist.

**Was wurde geändert:** 🧭willkommen ist jetzt befüllt: Banner im Look unserer Website, jede Kategorie mit eigenem Header und kurzen Beschreibungen aller Kanäle, das Community-Team inklusive unseres Bots, Links zu Website, Twitch und Coaching — und ein Schnellstart mit den drei wichtigsten Klicks.

**Wie es jetzt läuft:** Einfach reinschauen und durchklicken — jeder Kanal-Link bringt dich direkt hin, der Button am Ende wieder nach oben. Und wenn sich am Server etwas ändert, ziehen die Texte in Sekunden nach, ohne dass jemand neue Nachrichten posten muss.
## #196 — Text-Moderation reagiert entspannter auf Gaming-Trash-Talk

**Ausgangslage:** Der Moderationsbot hat einzelne Sprüche über Helden- oder Spielergruppen zu schnell als Belästigung vorgeschlagen. Selbst wenn die erste Einschätzung nur schwach war, reichte eine zweite Bestätigung schon für eine Mod-Karte.

**Was wurde geändert:** Solche weichen Textfälle brauchen jetzt zwei klare, hohe Einschätzungen, bevor sie überhaupt bei den Mods landen. Außerdem wird dem Prüfer ausdrücklich gesagt, dass Helden-, Rollen-, Rank- und Spielergruppen-Spott im Spielkontext normaler Trash-Talk oder Ragebait ist.

**Wie es jetzt läuft:** Aussagen wie „Haze-Spieler benutzen nicht viel von ihrem Gehirn" lösen keinen Review-Fall mehr aus, solange daraus kein klarer persönlicher Angriff oder echtes Hass-/Belästigungsmuster wird. Harte Fälle wie Scam, CSAM oder explizite Inhalte bleiben weiter streng.

## #195 — Neue Server-Struktur: klare Bereiche statt Kanal-Wildwuchs

**Ausgangslage:** Der Server war über die Zeit auf 17 Kategorien gewuchert — Sammelbecken wie „Sonstiges", leere Alt-Kategorien und uneinheitliche Namen machten es gerade Neuen schwer, sich zurechtzufinden.

**Was wurde geändert:** Der Server ist jetzt in klare Bereiche sortiert — Information, Neuigkeiten, Community, Deadlock, Coaching — mit einheitlichen Emojis und Trenner-Optik. Einige Kanäle sind umgezogen oder umbenannt (die Mitspieler-Suche heißt jetzt 🎯mitspieler-suche, das Ventil 😡rage-room); alle Verläufe sind erhalten geblieben.

**Wie es jetzt läuft:** Neu dazu sind 🧭willkommen als künftiger Startpunkt mit Server-Überblick und 🗓️scrim-planung fürs Coaching — beide werden in den nächsten Tagen befüllt. Der Umbau lief vollautomatisch über unsere Server-Verwaltung, mit Rechte-Prüfung und Backup vorab.

## #194 — Rangverlauf fürs Aktivitäts-Dashboard, Sichtbarkeit bestimmst du

**Ausgangslage:** Wir speichern die Rangdaten verknüpfter Accounts schon lange, aber es gab keinen Weg, sie auf der Website zu sehen — und keine Kontrolle darüber, wer sie sehen dürfte.

**Was wurde geändert:** Das Aktivitäts-Dashboard kann jetzt deinen Rangverlauf als Kurve zeigen und führt ein Rang-Leaderboard mit Top-Aufsteigern und Top-Rängen. Jeder entscheidet selbst per Schalter, ob der eigene Verlauf privat bleibt (Standard), nur für Server-Mitglieder sichtbar ist oder öffentlich.

**Wie es jetzt läuft:** Ohne dein Zutun sieht niemand deine Kurve — auch nicht per Direktlink, und die Server-Antwort verrät nicht mal, ob es dich gibt. Die Mitglieder-Stufe prüft echte Server-Zugehörigkeit; wer den Server verlässt oder gebannt wird, fällt sofort raus.

## #193 — FAQ-Bot mit frischem Wissen + Invite-Lounge mit Bot-Aufpasser

**Ausgangslage:** Die Wissensbasis unseres FAQ-Bots war an vielen Stellen veraltet — alte Kanalnamen, ein Invite-Ablauf, den es so nicht mehr gibt, und sogar falsche Preise beim Streamer-Dashboard. Wer den Bot gefragt hat, bekam teils Anleitungen für einen Server von gestern.

**Was wurde geändert:** Wir haben die komplette Server-Doku gegen den echten Stand abgeglichen und neu geschrieben — inklusive einer neuen Ehrlichkeits-Seite, auf der steht, was es bei uns bewusst NICHT gibt (damit der Bot dort nicht rät). Kanäle werden jetzt überall rename-fest verlinkt. Außerdem: Der /betainvite-Befehl ist weg — im Invite-Kanal fragst du einfach nett nach einem Invite und postest deinen Steam-Freundescode, ein Community-Mitglied lädt dich ein.

**Wie es jetzt läuft:** Der FAQ-Bot antwortet ab sofort mit aktuellem Wissen. Und in der Invite-Lounge passt der Bot mit auf: Fragst du nach einem Invite und vergisst den Freundescode, erinnert er dich einmal freundlich daran — mehr macht er dort nicht.

## #192 — Neuer Einstieg: 3 Fragen statt 10-Schritte-Assistent, Regelwerk neu, tote Kanäle ins Archiv

**Ausgangslage:** Der Einstieg auf den Server war ein Hindernislauf — erst Discord-Fragen, dann noch ein 10-Schritte-Assistent im Regelkanal, der fast 200 verwaiste Threads hinterlassen hat. Dazu ein Haufen Kanäle, in denen seit Monaten nichts mehr passiert.

**Was wurde geändert:** Der Einstieg läuft jetzt komplett über das native Discord-Onboarding mit drei kurzen Fragen: Wo stehst du gerade (spielst du schon, brauchst du einen Invite, oder bist du ganz neu), welche Pings willst du, und wo stehst du im Rang. Der alte Assistent ist Geschichte, das Regelwerk wurde neu geschrieben und ist jetzt ein reines Nachschlagewerk. Tote Kanäle wandern ins Archiv — gelöscht wird nichts; movement, deadlock-art, mods und food ziehen zusammen in die neue kreativ-ecke. Neue Mitglieder bekommen außerdem einen Server Guide mit den ersten Schritten.

**Wie es jetzt läuft:** Wer neu joint, beantwortet die drei Fragen und ist drin — wer angibt, noch keinen Invite zu haben, wird direkt zum Invite-Kanal geführt, und wer ganz neu ist, dem nehmen wir uns besonders an. Für Bestandsmitglieder ändert sich nichts, außer dass es aufgeräumter aussieht.

## #191 — Scam-Bilder bleiben auch in der Mod-Karte sichtbar

**Ausgangslage:** Beim Nachtesten mit echten Discord-Bildern war die Erkennung jetzt korrekt, aber ein Folgefehler blieb: Wenn Discord ein Bild mit ungenauem Dateityp meldet, konnte es in der Moderationskarte nur als normaler Anhang statt direkt als Beweisbild auftauchen.

**Was wurde geändert:** Die Darstellung nutzt jetzt dieselbe robuste Bild-Erkennung wie der Scan selbst. Die aktuellen Scam-Beispiele aus den Discord-Downloads sind als Regressionstest abgedeckt.

**Wie es jetzt läuft:** Scam-Bilder werden erkannt, geprüft und in der Mod-Karte sichtbar eingebettet, auch wenn Discord den Dateityp ungenau liefert.

## #190 — Bild-Scam rutscht bei frischen Accounts nicht mehr durch

**Ausgangslage:** Ein Scam-Account konnte erneut Bilder posten, ohne dass der Moderator griff. Ursache war eine Schutzlücke beim Einordnen neuer Nachrichten: Wenn Discord ein Mitglied noch nicht im lokalen Cache hatte, wurde die Moderation aus Vorsicht komplett übersprungen. Zusätzlich konnten Bildanhänge mit ungenauem Dateityp übersehen werden.

**Was wurde geändert:** Der Bot nutzt jetzt die Mitgliedsdaten, die Discord direkt mit der Nachricht mitschickt, wenn der Cache noch nicht vollständig ist. Bildanhänge werden außerdem nicht mehr nur über den gemeldeten Dateityp erkannt, sondern auch zuverlässig über die Dateiendung.

**Wie es jetzt läuft:** Frische oder gerade erst gecachte Accounts werden wieder geprüft; Scam-Bilder gehen an die Bildprüfung und können wieder automatisch entfernt bzw. ans Mod-Team gemeldet werden.

## #189 — Automatische Moderation ist zurück: ein System, doppelt geprüft, Mods entscheiden

**Ausgangslage:** Die alte Auto-Moderation hatte Nachrichten teils eigenständig gelöscht — ohne Freigabe, schwer nachvollziehbar und zu vorschnell; wir hatten sie deshalb pausiert (#186). Dazu lief ein zweiter, getrennter Schutz gegen gekaperte Accounts — das fühlte sich wie mehrere uneinheitliche Systeme an.

**Was wurde geändert:** Beides ist jetzt ein einziges System. Jeder Textverdacht wird von einer zweiten KI gegengeprüft, bevor überhaupt etwas passiert; Bilder laufen über eine eigene Prüfung. Von allein und ohne Rückfrage handelt der Bot nur noch bei klarem Betrug oder schweren Inhalten — alles andere kommt als Vorschlag mit Knöpfen (Übernehmen · Bannen · Verwerfen) zu den Mods. Jeder Fall erscheint als kompakte, nachvollziehbare Karte, jede automatische Aktion lässt sich per Knopf zurücknehmen.

**Wie es jetzt läuft:** Kein stilles Löschen mehr. Was der Bot allein macht, ist eng auf die klaren Fälle begrenzt und immer dokumentiert — im Zweifel entscheidet ein Mensch.

## #188 — Server-Aufräumen: Kanäle umbenannt, Rechte entrümpelt, Invite-Kanal offen

**Ausgangslage:** Über die Jahre hatten sich auf dem Server über 500 einzelne Rechte-Einstellungen angesammelt — vieles davon Leichen, Redundanzen oder Regeln, die niemand mehr erklären konnte. Dazu Kanalnamen, die nicht sagten, was drin ist, und ein Invite-Kanal, in den man ohne Extra-Rolle nicht mal schreiben konnte.

**Was wurde geändert:** Wir haben die kompletten Server-Rechte einmal von Grund auf neu sortiert und auf einen klaren Satz Regeln eingedampft — der Bot überwacht die jetzt automatisch und meldet Abweichungen. Vier Kanäle heißen jetzt so, wie sie gemeint sind: regelwerk, deadlock-rang, deadlock-invite und server-support. Vor dem Umbau wurde ein vollständiger Wiederherstellungspunkt gesichert und geprobt.

**Wie es jetzt läuft:** #deadlock-invite ist für alle offen — Steam in #deadlock-rang verknüpfen, der Invite kommt automatisch, und wer schneller persönlich einladen will, darf das weiterhin. Für alle anderen sollte sich nichts verschlechtern; falls doch irgendwo Rechte fehlen, kurz ein Ticket aufmachen.

## #187 — Steam-Verknüpfung läuft jetzt komplett auf der neuen zentralen Datenbank

**Ausgangslage:** Bei der Umstellung auf die neue zentrale Datenbank hatte der Bot User-Profile nirgends aktiv nachgetragen — das fiel erst auf, als eine neue Steam-Verknüpfung dagegen lief. Der Steam-Bot lief deshalb übergangsweise noch auf der alten, separaten Datenbank weiter.

**Was wurde geändert:** Der Bot trägt jetzt bei jeder normalen Aktivität (Nachricht, Befehl, Server-Beitritt) automatisch nach, wer gerade da ist — mit eingebauter Bremse, damit das keine unnötige Last erzeugt. Erst danach wurde der Steam-Bot komplett auf die neue Datenbank umgestellt.

**Wie es jetzt läuft:** Neue Steam-Verknüpfungen funktionieren zuverlässig für jeden User, auch für welche, die vorher noch nie mit dem Bot interagiert haben. Steam-Bot und Hauptbot teilen sich jetzt dieselbe Datenbank.

## #186 — Automatische Text-Moderation vorübergehend pausiert

**Ausgangslage:** Die automatische Moderation hat Textnachrichten teils eigenständig gelöscht und mit einer Auszeit belegt — ohne dass ein Mod das vorher freigeben oder hinterher leicht nachvollziehen konnte, und stellenweise zu vorschnell.

**Was wurde geändert:** Der automatische Text-Scan ist abgeschaltet. Der Schutz gegen Spam-Wellen und übernommene Accounts (die Bilder-Fluten) läuft unverändert weiter.

**Wie es jetzt läuft:** Der Bot entfernt aktuell keine Nachrichten mehr allein wegen einer KI-Textbewertung. Eine überarbeitete Version kommt, die jeden Verdacht doppelt prüft und den Mods eine nachvollziehbare Meldung mit Entscheidungsknöpfen gibt.

## #185 — Bot spricht durchgehend Deutsch, Logs werden ehrlicher

**Ausgangslage:** Ein paar Bot-Antworten und Moderations-Meldungen waren noch auf Englisch (Reste der Technik-Umstellung), und die Abschieds-Umfrage tauchte in unseren Logs immer als „fehlgeschlagen" auf — obwohl Discord solche Nachrichten nach einem Austritt schlicht nicht mehr zustellen lässt.

**Was wurde geändert:** Alle sichtbaren Texte sind jetzt durchgehend deutsch (inklusive der KI-Begründungen in Moderations-Meldungen), und die Logs unterscheiden sauber zwischen „wirklich kaputt" und „von Discord erwartbar blockiert".

**Wie es jetzt läuft:** Wer mit dem Bot zu tun hat, liest Deutsch. Und wenn im Maschinenraum etwas wirklich schiefgeht, sehen wir es jetzt sofort, statt es im Dauerrauschen falscher Alarme zu übersehen.

## #184 — Neues Onboarding: das Konzept steht

**Ausgangslage:** Unser Onboarding war über die Zeit zu einem Flickenteppich aus fünf halbfertigen Systemen gewachsen — immer weniger Neue sind wirklich angekommen, und an den wichtigsten Stellen (Invite bekommen, erste Frage stellen, Anschluss finden) hat es am häufigsten gehakt.

**Was wurde geändert:** Wir haben den kompletten Ist-Zustand vermessen (echte Zahlen, echte Chats, echte Stolperstellen), Forschung ausgewertet, was neue Mitglieder wirklich aktiviert, und daraus ein vollständiges Neubau-Konzept erarbeitet und hart gegengeprüft: ein einziger klarer Einstieg, danach echte Menschen statt Automaten-Wände, ein Bot-Begleiter für Fragen, ein ehrlicher Invite-Weg und aufgeräumte Kanäle und Rollen.

**Wie es jetzt läuft:** Der Umbau passiert in Etappen und wird vor jedem sichtbaren Schritt angekündigt. Für euch ändert sich erst etwas, wenn wir es ankündigen — Feedback ist ab jetzt ausdrücklich erwünscht.

## #183 — Server-Statistiken werden dauerhaft archiviert

**Ausgangslage:** Discord zeigt Server-Statistiken (Neuzugänge, Aktivität, Retention) nur für die letzten 120 Tage und löscht ältere Daten unwiderruflich. Für langfristige Auswertungen — etwa wie gut neue Mitglieder ankommen — fehlte damit die Historie.

**Was wurde geändert:** Die exportierten Statistiken werden jetzt dauerhaft und versioniert im Projekt abgelegt, mit fester Ablage-Routine für künftige Exporte.

**Wie es jetzt läuft:** Wir können Trends über beliebig lange Zeiträume vergleichen und Entscheidungen (z.B. zum neuen Onboarding) an echten Langzeitdaten messen statt am 120-Tage-Fenster.

## #182 — Patch-Erkenntnisse bekommen eine eigene Brain-Schicht

**Ausgangslage:** Die zentrale Brain-Timeline kann Patch-Events, Forum-Claims und Current-State speichern, aber kuratierte Auswertungen aus der Patch-Historie hatten noch keinen eigenen Platz. Solche Hinweise sind wichtig, um alte Werte, Renames oder Reworks nicht versehentlich als aktuelle Wahrheit zu behandeln.

**Was wurde geändert:** Das `brain`-Schema bekommt eine Insight-Tabelle mit stabilen Hashes, Entity-Bezug, Currentness, Vertrauensstufe, Quellen-Event-IDs, Links und strukturiertem Payload. Damit können Analyse-Agenten verdichtete Patch-Erkenntnisse speichern, ohne Rohdaten oder Current-State zu überschreiben.

**Wie es jetzt läuft:** Das Brain kann künftig zwischen Rohereignis, aktuellem Snapshot und kuratierter Warn-/Erkenntnisschicht unterscheiden. Alte Patchinfos bleiben nachlesbar, werden aber explizit als historisch, ersetzt oder aktuell markiert.

## #181 — Brain bekommt einen zentralen Postgres-Bereich

**Ausgangslage:** Das Deadlock-Brain sammelte Patch-, Forum- und Wissensdaten noch in einer eigenen SQLite-Datenbank. Für eine echte Historie mit Timeline, alten Patchständen, Reworks und aktuellem Gewinnerstand ist das als zentrale Wissensquelle zu schwach.

**Was wurde geändert:** Die zentrale Datenbank bekommt ein eigenes `brain`-Schema für Quellen, Snapshots, Entitäten, Patch-Events, Forum-Claims, vereinheitlichte Wissens-Events und den aktuellen Entitätszustand. Historische Daten können damit erhalten bleiben, während aktuelle Quellen sauber darüber priorisiert werden.

**Wie es jetzt läuft:** Postgres ist vorbereitet, Brain-Daten als Zeitlinie aufzunehmen: alte Werte bleiben nachvollziehbar, neue Werte können als Current-State gewinnen, und entfernte oder überarbeitete Heroes, Items und Fähigkeiten müssen nicht mehr verloren gehen.

## #180 — Dashboard prüft Coaching-Sperren wieder in der zentralen Datenbank

**Ausgangslage:** Nach dem Merge auf die zentrale Datenbank hing die interne Dashboard-Abfrage für Coaching-Sperren noch an der alten lokalen Datenbank-Logik. Dadurch brach der Release-Build des Web-Dienstes.

**Was wurde geändert:** Die Sperren-Abfrage nutzt jetzt dieselbe zentrale Datenbank wie der Bot und die Tests legen ihre Sperren direkt dort an.

**Wie es jetzt läuft:** Der Web-Dienst baut wieder als Release-Binary und die Website kann aktive Coaching-Sperren weiter über die interne Dashboard-Route prüfen.

## #179 — Zentrale Datenbank kann live aus den Alt-Daten befüllt werden

**Ausgangslage:** Die zentrale Datenbank hatte das fertige Schema, aber der operative Schritt zum Befüllen aus den bestehenden Datenbanken war noch kein eigener, wiederholbarer Lauf.

**Was wurde geändert:** Es gibt jetzt einen zentralen Sync-Lauf, der die bekannten Alt-Datenbanken zuerst konsistent snapshotet und danach über das geprüfte Ledger in die zentrale Datenbank lädt. Der Lauf meldet Zieltabellen, Zeilensummen und bekannte Orphan-Fälle, ohne Zugangsdaten auszugeben.

**Wie es jetzt läuft:** Die zentrale Datenbank ist mit den aktuellen Bot-, Website- und Turnier-Daten befüllt. Ein späterer Cutover kann denselben Lauf direkt vor dem Umschalten wiederholen und hat dabei einen SQLite-Snapshot als Rollback-Anker.

## #178 — Bot-Build nutzt die zentrale Moderations-Datenbank

**Ausgangslage:** Der Rust-Bot konnte auf dem aktuellen Branch nicht als Release-Binary gebaut werden, weil zwei Moderations-Bausteine noch die alte lokale Datenbank-Verbindung bekamen. Die Moderation erwartet inzwischen die zentrale Datenbank-Verbindung.

**Was wurde geändert:** Security-Guard und AI-Moderation werden beim Start mit der zentralen Datenbank-Verbindung verdrahtet. Das passt zur bereits umgestellten Moderationsschicht.

**Wie es jetzt läuft:** Der Rust-Bot baut wieder als Release-Binary und kann nach Voice-Änderungen sauber neu deployed werden.

## #177 — Rang-Lanes schreiben Rechte gebündelt

**Ausgangslage:** Beim Mindest-Rang und beim Laden eines Presets wurden Rang-Rollen einzeln angefasst und niedrigere Rollen aktiv gesperrt. Das passte nicht zu den Comp-Lanes, die über erlaubte Rang-Rollen funktionieren: Bei `Phantom+` müssen Phantom, Ascendant und Eternus rein dürfen, nicht die unteren Rollen als Deny-Liste landen.

**Was wurde geändert:** Rang-Rollen werden jetzt gesammelt und in einem Batch auf den Kanal geschrieben. Erlaubte Rang-Rollen bekommen `Verbinden` erlaubt, alle nicht erlaubten Rang-Rollen werden aus den Kanalrechten entfernt statt auf `deny` gesetzt. Auch andere Voice-Overwrite-Änderungen laufen über denselben gebündelten Kanal-Update-Pfad.

**Wie es jetzt läuft:** Ein Preset oder Mindest-Rang wie `Phantom+` öffnet die passenden höheren Rang-Rollen sauber per Allow. Die Deny-Liste wird nicht mehr mit Rang-Rollen vollgeschrieben.

## #176 — `!brain` beantwortet einfache Fragen wieder direkt

**Ausgangslage:** `!brain` behandelte auch kurze Rechen- oder Mechanikfragen wie eine Build-Analyse. Dadurch kamen Antworten mit internen Vertrauenshinweisen, langen Verifikationsabschnitten und zusätzlichem Build-Gelaber, obwohl eigentlich nur ein kurzer Zahlenwert gefragt war.

**Was wurde geändert:** Normale Brain-Fragen bekommen jetzt eigene Antwortregeln: erst die konkrete Antwort, bei Rechnungen nur die kurze Formel plus Ergebnis, keine internen Datenquellen, keine Vertrauens-Legende und keine Build-Tipps ohne Build-Frage. Die ausführlicheren Build-Regeln greifen nur noch bei echten Build-Fragen.

**Wie es jetzt läuft:** Eine Frage wie „ab wie viel Spirit ist der Schaden wieder gleich?" wird knapp beantwortet, statt in eine halbe Faktenprüfung auszuarten. Builds bleiben weiterhin strukturiert, aber ohne sichtbare interne Markierungen.

## #175 — Scam-Guard meldet ehrlich und greift wieder durch

**Ausgangslage:** Der Scam-Guard erkannte Bild-Scam korrekt, lief live aber im Shadow-Modus. Dadurch standen in der Mod-Meldung „Ban failed", „Deleted: 0" und ein Timeout-Button, obwohl keine automatische Aktion ausgeführt wurde. Außerdem konnten schnelle Folgeposts desselben Accounts mehrere fast gleiche Mod-Meldungen auslösen, und bei bildlosen Textvorschauen war nicht klar genug sichtbar, in welchen Kanälen der Scam stand.

**Was wurde geändert:** Der Guard läuft standardmäßig wieder im Durchsetzungsmodus; Shadow muss jetzt bewusst gesetzt werden. Nach einem Fall gibt es einen kurzen User-Cooldown gegen Alert-Spam. Mod-Meldungen zeigen Fundorte mit Kanal und Sprunglink, unterscheiden Shadow sauber von echten Fehlversuchen und zeigen den Timeout-Aufheben-Button nur noch bei echten Timeout-Fällen. Fehler beim Bannen, Timeouten oder Löschen landen mit Discord-Fehler im Log.

**Wie es jetzt läuft:** Neue Treffer werden automatisch ausgeführt statt nur gemeldet. Mods sehen sofort, wo gepostet wurde, ob Löschung wirklich versucht wurde und welche manuelle Aktion noch sinnvoll ist.

## #174 — Rollen-Entzug erkennt fehlende Discord-Mitglieder sauber

**Ausgangslage:** Wenn Steam beim Aufräumen die Verified-Rolle entfernen wollte, Discord den Nutzer aber nicht mehr als Server-Mitglied kannte, meldete der Broker daraus fälschlich einen internen Fehler. Steam retryte deshalb alle paar Stunden, obwohl der Fall eigentlich abgeschlossen war.

**Was wurde geändert:** Der Discord-Adapter erkennt `Unknown Member` beim Rollen-Entzug jetzt als „Mitglied nicht gefunden" und gibt diese Information sauber an den Broker weiter. Der Broker antwortet dadurch mit 404 statt 502.

**Wie es jetzt läuft:** Steam kann solche Cleanup-Einträge bereinigen, statt sie immer wieder gegen den Broker laufen zu lassen. Echte Discord-/Broker-Fehler bleiben weiterhin Fehler.

## #173 — Coaching-Sperre ist für die Website direkt prüfbar

**Ausgangslage:** Der Bot hatte die No-Show-Sperre bereits als Wahrheit in seiner Datenbank und blockierte gebannte Coaching-Anfragen beim Discord-Spiegeln. Die Website konnte diese Wahrheit am Formular-Eingang aber nicht selbst prüfen, weil ihre Coaching-Daten in einer eigenen Datenbank liegen.

**Was wurde geändert:** Das Dashboard bietet jetzt eine interne, token-geschützte Coaching-Abfrage für genau diesen Fall: Discord-ID rein, aktive Sperre samt Ablauf zurück. Abgelaufene Sperren zählen nicht, und die Route bleibt lokal/intern geschützt wie die bestehenden Service-zu-Service-Endpunkte.

**Wie es jetzt läuft:** Die Website kann vor dem Speichern beim Bot nachfragen. Damit bleibt die Bot-Datenbank die Quelle der Sperre, ohne dass die Website direkt in fremde Tabellen greifen muss.

## #172 — Coaching: gesperrte Anfragen werden abgewiesen, kein Doppel-Claim mehr

**Ausgangslage:** Drei Lücken rund ums Coaching. Wer wegen eines verpassten Termins gesperrt war, konnte über die Website trotzdem eine neue Anfrage anstoßen. Klickten zwei Coaches im selben Moment auf „Übernehmen", konnten sich beide als zuständig wähnen. Und die Moderations-Buttons unter einer Review prüften die falsche Berechtigung — jemand mit „Rollen verwalten", aber ohne Moderationsrechte, hätte handeln können.

**Was wurde geändert:** Eine gesperrte Person bekommt ihre Coaching-Anfrage jetzt schon beim Reinkommen abgewiesen, mit klarer Rückmeldung statt stillem Ins-Leere-Laufen. Das Übernehmen ist eindeutig: nur ein Coach bekommt den Zuschlag, der zweite Klick läuft sauber ins „schon vergeben". Und die Moderations-Buttons prüfen jetzt die passende Berechtigung (Mitglieder moderieren bzw. bannen), bevor überhaupt etwas passiert.

**Wie es jetzt läuft:** Das Coaching ist an den Entscheidungspunkten dicht — gesperrt heißt gesperrt, übernommen heißt von genau einem, und moderieren darf nur, wer auch moderieren darf.

## #171 — Coaching-Anfragen von der Website landen jetzt in Discord

**Ausgangslage:** Wer auf der Website eine Coaching-Anfrage stellt, war bisher nur dort sichtbar — im Discord, wo die Coaches sich koordinieren, tauchte die Anfrage gar nicht auf. Das hieß: doppelt hinschauen, Sachen von Hand rüberkopieren, und leicht mal eine Anfrage übersehen.

**Was wurde geändert:** Eine neue Website-Anfrage erscheint jetzt automatisch als schickes Embed im Coaching-Anfrage-Kanal — mit Rang, Hero, Verfügbarkeit und den Punkten, an denen jemand arbeiten will, plus einem Button „Auf der Website öffnen", der direkt zur Anfrage führt. Coaches übernehmen sie wie gewohnt per Klick (Claim bzw. faire Reihum-Zuteilung), und die Übernahme wird zurück auf die Website gespiegelt, sodass die Anfrage dort als vergeben auftaucht — ohne Doppel-Einträge.

**Wie es jetzt läuft:** Website und Discord ziehen beim Coaching am selben Strang: eine Anfrage, ein Ort zum Übernehmen, kein Hin- und Herkopieren mehr. Bestehende Anfragen bleiben unberührt — gespiegelt wird ab jetzt, was neu reinkommt.

## #170 — `!brain`: aufgeräumte Antworten + „denkt nach"-Anzeige

**Ausgangslage:** Die frischen `!brain`-Antworten kamen als dichte Textwand — die Stichpunkte klebten alle in einer Zeile, weil die Zeilenumbrüche intern beim Zusammenbauen verloren gingen. Und nach dem Absenden stand man ein paar Sekunden vor dem Nichts, bis die Antwort plötzlich da war.

**Was wurde geändert:** Antworten behalten jetzt ihre Struktur — jeder Stichpunkt steht auf einer eigenen Zeile, Abschnitte bekommen fette Überschriften, das Ganze ist gut scannbar. Während der Bot nachdenkt, erscheint sofort eine kleine animierte „💭 . . ."-Nachricht; sobald die Antwort fertig ist, wird genau diese Nachricht durch das Ergebnis ersetzt.

**Wie es jetzt läuft:** `!brain` fühlt sich reaktiv an — man sieht sofort, dass der Bot arbeitet — und liefert eine übersichtliche, gegliederte Antwort statt einer Textwand.

## #169 — `!brain`: schönere Antworten und immer ein Build

**Ausgangslage:** Der frisch ausgelieferte `!brain`-Befehl hatte zwei Schwächen. Die Antwort kam als unstrukturierte Textwand inklusive roher Markdown-Zeichen, und bei dünner Faktenlage druckte der Bot oft einen langen „dazu hab ich zu wenig gesicherte Daten"-Absatz, statt etwas Brauchbares zu liefern.

**Was wurde geändert:** Antworten kommen jetzt als sauberes Embed — Titel ist deine Frage, mit Farbakzent und Fußzeile. Und der Bot verweigert nicht mehr: Wo geprüfte Fakten vorliegen, nutzt er sie und markiert sie mit ✅; wo sie fehlen, ergänzt er einen sinnvollen Build aus allgemeinem Deadlock-Wissen und kennzeichnet diese Teile ehrlich mit ℹ️ (allgemeine Einschätzung, nicht aus geprüften Daten). Echte Nicht-Deadlock-Fragen werden weiterhin freundlich abgewiesen, und der Bot pingt dabei niemanden.

**Wie es jetzt läuft:** `!brain` gibt immer eine konkrete, gut lesbare Antwort — geprüftes Wissen klar getrennt von allgemeiner Einschätzung, statt einer Absage oder einer Textwand.

## #168 — Sprachkanal verschieben passt die Einstellungen an

**Ausgangslage:** Wer einen TempVoice-Kanal in eine andere Kategorie gezogen hat, blieb auf den alten Einstellungen sitzen: Eine Street-Brawl-Lane behielt z. B. ihr 4er-Limit, auch wenn sie nach Ranked oder Chill verschoben wurde — und nach einem Bot-Neustart konnten sogar die alten Regeln weiterwirken.

**Was wurde geändert:** Beim Verschieben übernimmt die Lane jetzt sauber die Einstellungen der Zielkategorie — Platzanzahl, Rang-Regeln und Benennung stellen sich auf die neue Kategorie um. Der Neustart-Abgleich bleibt davon unberührt: bestehende Lanes behalten ihre bekannte Zuordnung wie bisher.

**Wie es jetzt läuft:** Lane verschieben heißt jetzt auch wirklich „Lane passt sich an" — eine nach Ranked gezogene Lane verhält sich wie eine Ranked-Lane, eine nach Chill gezogene wie eine Chill-Lane.

## #167 — Reaction-Rollen, faireres Coaching und der große Rust-Nachzug

**Ausgangslage:** Eine ganze Welle fertiger Arbeit lag bereit, aber noch nicht im laufenden Bot: ein neues Reaction-Rollen-System, eine faire Coach-Verteilung und der Rest der Rust-Portierung — Voice/TempVoice, Moderation, Stats und Spielersuche, Onboarding. Zusätzlich steckte im Voice-Code noch ein heikler Stolperstein: Beim Bot-Start hätte er unter ungünstigen Umständen aktive Sprachkanäle für „leer" halten und aufräumen können, bevor der Bot überhaupt richtig verbunden war.

**Was wurde geändert:** Wir haben alles zusammengeführt und live gebracht. Reaction-Rollen sind neu — ihr klickt unter der passenden Nachricht auf die Reaktion und bekommt die Rolle automatisch, beim ersten Mal mit einer kurzen Erklär-DM; verwaltet wird das komplett im Dashboard. Die Coach-Verteilung läuft jetzt als echtes Round-Robin, also reihum, statt dass einzelne dauernd zuerst drankamen; wer freigibt oder eine Reservierung verfallen lässt, verzerrt die Reihenfolge nicht mehr, Coaches können sich aus der Auto-Verteilung ausklinken, und die Zugriffsrolle gilt jetzt sieben Tage. Den Voice-Stolperstein haben wir entschärft: Aufräumen und Abgleich starten erst, wenn der Bot seinen Kanal-Cache wirklich geladen hat, und ein unbekannter Kanal wird nie mehr vorschnell als „weg" behandelt.

**Wie es jetzt läuft:** Reaction-Rollen, faires Coaching und die übrige Rust-Fassung von Voice, Moderation, Stats und Onboarding laufen jetzt alle im selben Bot. Und der Neustart ist sicher — es kann nichts mehr passieren, das versehentlich aktive Sprachkanäle wegräumt.

## #166 — Frag das Brain: neuer `!brain`-Befehl

**Ausgangslage:** Wir sammeln seit Monaten in einer eigenen Wissensbasis — dem „Brain" — geprüftes Deadlock-Wissen: offizielle Spielwerte, die Patch-Historie und aus Creator-Videos extrahierte Aussagen, die wir gegen die echten Spieldaten gegengeprüft haben. Bisher lag das alles nur intern; im Discord kam man nicht dran. Wer eine Deadlock-Frage hatte, musste raten, lange suchen oder ein allgemeines KI-Tool fragen, das Deadlock kaum kennt und gern veraltetes Zeug erzählt.

**Was wurde geändert:** Es gibt jetzt den Befehl `!brain <Frage>`. Dahinter steckt nicht einfach „frag irgendeine KI", sondern der echte Brain-Weg: Die Frage geht zuerst ins Brain, das die passenden Fakten heraussucht und nach Vertrauensgrad sortiert — gesicherte Spieldaten schlagen geprüfte Creator-Aussagen, die wiederum Ungeprüftes schlagen. Erst aus diesem Faktenbündel formuliert die KI die Antwort. Fragen, die nichts mit Deadlock zu tun haben, fängt der Befehl freundlich ab, bevor überhaupt eine KI-Antwort erzeugt wird. Ein kurzer Cooldown pro Person bremst Spam, lange Antworten werden sauber aufgeteilt, und der Bot pingt dabei niemanden — selbst wenn in einer Antwort mal ein `@everyone` auftauchen würde, löst es nichts aus.

**Wie es jetzt läuft:** Jeder im Discord kann etwas wie `!brain wie spiel ich Vindicta?` oder `!brain ist Lash grad stark?` fragen und bekommt eine Antwort, die auf unserem geprüften Deadlock-Wissen fußt statt auf Halbwissen. Findet das Brain zu einer Frage nichts Belastbares, sagt der Bot das ehrlich, statt zu raten.

## #165 — Altes In-Bot-Turnier entfernt

**Ausgangslage:** Im Bot steckte noch ein altes, nie richtig fertig gewordenes Turnier-System: eine Anmelde- und Admin-Seite für Turniere sowie ein Befehl, der per Sprachkanal zwei einigermaßen faire Teams zusammenlosen sollte. Das echte, weitergepflegte Turniersystem läuft inzwischen komplett woanders — als eigenständiges Projekt mit eigener Website. Der alte Kram im Bot war damit toter Ballast, der nur noch Pflegeaufwand und Verwechslungsgefahr brachte.

**Was wurde geändert:** Wir haben das alte Turnier restlos aus dem Bot geräumt — die Anmelde-/Admin-Seite samt ihrer Schnittstellen, den Team-Auslosungs-Befehl mit allen Unterbefehlen, die alte interne Turnier-Webseite und den Turnier-Tab im Verwaltungs-Dashboard. Alles, was das neue Turniersystem wirklich braucht, bleibt unangetastet: die Discord-Login-Brücke, die Vermittlung von Kanälen, Rollen und Voice-Verschiebungen, die Tierlist und die verknüpften Steam-/Rang-Daten.

**Wie es jetzt läuft:** Der Bot ist um eine komplette tote Funktionsschicht leichter, das Turnier läuft sauber im separaten System. Für die Website und das neue Turnier ändert sich nichts — deren Anbindung an den Bot blieb vollständig erhalten. Die alten Turnier-Daten liegen vorerst noch unberührt in der Datenbank; das Aufräumen dort kommt später als eigener Schritt.

## #164 — Rust-Portierung schließt die letzten großen Parität-Lücken

**Ausgangslage:** Neben dem aktiven Python-Bot pflegen wir eine noch nicht produktive Rust-Fassung, die Python später ablösen soll. Eine erneute Gegenprüfung fand die letzten größeren Funktionslücken dieser Portierung: die Owner-Adminbefehle, die Steuer-Panels der temporären Sprachkanäle, die Knöpfe und die Abschluss-Umfrage rund um die Coaching-Anfragen, den kompletten Admin-Dialog der Turnierverwaltung, den KI-gestützten Onboarding-Flow sowie eine zuverlässige Verarbeitung der Build-Veröffentlichungen samt einer internen Schnittstelle.

**Was wurde geändert:** Wir haben diese Bereiche in der Rust-Fassung nachgezogen — verhaltensgleich zum Live-Python-Bot: die Owner-Adminbefehle (Status, Neustart, Befehls-Sync) inklusive Statusanzeige und Einfach-Start-Schutz; die Steuer-Panels und das Einstiegs-Panel der temporären Sprachkanäle; die wieder verdrahteten Coaching-Knöpfe und die Umfrage samt Belohnungsrolle nach dem Voice-Termin; den Turnier-Admindialog (Zeitraum erstellen und beenden, Anmeldungen seitenweise entfernen, Teams anlegen und löschen, alle Anmeldungen leeren); das KI-Onboarding (Startknopf, Drei-Fragen-Dialog, persönliche Tour per KI, Regelbestätigung mit Rollenvergabe, neustartsichere Knöpfe); sowie die saubere Verarbeitung der Build-Ergebnisse mit Bereitschafts-Prüfung und eine bislang fehlende interne Kanal-Auskunft.

**Wie es jetzt läuft:** Die Rust-Fassung verhält sich in diesen Bereichen jetzt deckungsgleich zum Live-Bot. Für Nutzer ändert sich dadurch unmittelbar nichts, da die Rust-Fassung nicht im Live-Betrieb ist; der Schritt hält die Portierung für eine spätere Umstellung vollständig deckungsgleich. Kleinere und mittlere Restpunkte sind als nächste Welle eingeplant.

## #163 — Server-Statistik der Website wieder live, stabilere Discord-Aktionen

**Ausgangslage:** Die öffentliche Server-Statistik auf der Website (Mitglieder, online, im Voice) war kaputt und meldete „keine Daten"; auf dem Dashboard erschienen statt Namen teils nur rohe IDs. Ursache war eine Lücke zwischen dem neuen Website-Backend und dem Bot: Das Backend fragte den Bot minütlich nach den Live-Kennzahlen und nach der Auflösung mehrerer Namen auf einmal, doch der Bot kannte diese beiden Abfragen nicht und antwortete mit „nicht gefunden". Zusätzlich brach gelegentlich eine ausgehende Aktion des Bots (etwa eine Abschieds-Umfrage als Direktnachricht) mit einem internen Fehler ab, weil bestimmte komprimierte Antworten von Discord nicht entschlüsselt werden konnten.

**Was wurde geändert:** Der Bot beantwortet jetzt beide Abfragen des Website-Backends — die aggregierten Live-Kennzahlen der Gilde (inklusive Vanity-Link) und die gebündelte Auflösung mehrerer Nutzer-IDs zu Anzeigenamen. Beide Abfragen laufen ausschließlich lokal und nur lesend. Für die Kompressions-Aushandlung mit Discord wird das fehleranfällige Verfahren nicht mehr angeboten, sodass alle Antworten zuverlässig verarbeitet werden.

**Wie es jetzt läuft:** Die Server-Statistik auf der Website zeigt wieder aktuelle Zahlen, und auf dem Dashboard erscheinen Namen statt roher IDs. Ausgehende Aktionen des Bots — darunter Direktnachrichten — laufen ohne die sporadischen Entschlüsselungs-Abbrüche durch.

## #162 — Rust-Portierung zieht die Bild-Scam-Verbesserung nach

**Ausgangslage:** Die in #161 ausgelieferte Verbesserung der Bild-Scam-Erkennung lag bislang nur in der aktiven Python-Fassung vor; die parallel gepflegte, noch nicht produktive Rust-Portierung kannte weiterhin nur den alten Bildprüf-Weg und das alte Verhalten.

**Was wurde geändert:** Die Rust-Portierung erhielt einen eigenständigen OpenAI-Bildprüf-Client, das auf wenige Minuten verkürzte Zeitfenster für den Mehrkanal-Verdacht, das Anhängen des Beweisbilds an die Mod-Meldung und die ehrliche Status-Angabe zu Löschung und Timeout — verhaltensgleich zur Python-Fassung.

**Wie es jetzt läuft:** Beide Codestände verhalten sich bei der Bild-Scam-Erkennung identisch. Für Nutzer ändert sich durch diesen Schritt nichts unmittelbar, da die Rust-Fassung nicht im Live-Betrieb ist; er hält die Portierung für eine spätere Umstellung deckungsgleich.

## #161 — Bild-Scam-Erkennung: treffsicheres Modell, ehrliche Mod-Meldung

**Ausgangslage:** Die automatische Bild-Scam-Erkennung stufte normale Spielinhalte — Rang-Verläufe, Match-Statistiken, Server-Logos — fälschlich als Casino-/Glücksspiel-Betrug ein und verhängte dadurch unberechtigte Timeouts gegen langjährige Mitglieder. Zusätzlich zeigte die Meldung an das Mod-Team weder das beanstandete Bild noch verlässlich an, ob die Nachricht wirklich gelöscht wurde — der Hinweis „gelöscht" stand fest im Text, auch wenn das Löschen fehlschlug.

**Was wurde geändert:** Die Bildprüfung läuft jetzt über ein deutlich treffsichereres KI-Modell, das an realen Server-Bildern gegengeprüft wurde. Der Auslöser für den Mehrkanal-Verdacht reagiert nur noch auf Bilder, die in kurzem Abstand (wenige Minuten) über mehrere Kanäle auftauchen, statt auf alles innerhalb einer Stunde. Die Mod-Meldung hängt das beanstandete Bild als Beweis an und nennt den tatsächlichen Stand von Löschung und Timeout.

**Wie es jetzt läuft:** Normale Gaming-Screenshots lösen keinen Fehlalarm mehr aus; das schnelle Streuen identischer Scam-Bilder über mehrere Kanäle wird weiterhin erkannt. Das Mod-Team sieht das Beweisbild und eine ehrliche Statusangabe und kann die Strafe gezielt prüfen. Die Strafzumessung bleibt unverändert (Timeout für etablierte, Bann für neue Accounts).

## #160 — AI-Moderation: Kategorie „Racism" abgeschaltet

**Ausgangslage:** Die automatische Inhaltsprüfung schlug für die Kategorie Rassismus auch bei harmlosen oder grenzwertigen Kurznachrichten an und erzeugte dadurch übermäßig viele Fehlmeldungen, die den Moderations-Review unnötig fluteten.

**Was wurde geändert:** Es gibt jetzt eine konfigurierbare Liste abgeschalteter Kategorien; „Racism" steht darin. Die Klassifizierung läuft technisch weiter, das Ergebnis abgeschalteter Kategorien wird aber verworfen, bevor daraus ein Moderationsvorschlag oder eine Aktion entsteht.

**Wie es jetzt läuft:** Für als Rassismus eingestufte Nachrichten werden keine Vorschläge mehr erzeugt — unabhängig von der Confidence. Alle übrigen Kategorien (z. B. Scam, NSFW, Hassrede) arbeiten unverändert weiter. Die Abschaltung lässt sich jederzeit zurücknehmen, indem die Kategorie wieder aus der Liste entfernt wird.

## #159 — Ticket-Kanäle für Beta-Einladungen wieder privat

**Ausgangslage:** Bei Beta-Einladungen erstellte Ticket-Kanäle waren zeitweise für mehr Personen sichtbar als vorgesehen, weil die Zugriffsrechte beim Anlegen nicht mehr eindeutig auf den jeweiligen Empfänger eingeschränkt wurden.

**Was wurde geändert:** Neu erstellte Ticket-Kanäle erhalten beim Anlegen wieder ausdrückliche Sichtbarkeits-Beschränkungen, sodass nur der zugehörige Empfänger und das Team Zugriff haben. Fehlt ausnahmsweise die Empfänger-Zuordnung, bleibt der Kanal trotzdem geschlossen statt offen.

**Wie es jetzt läuft:** Jeder Beta-Ticket-Kanal ist von Beginn an privat. Ohne gültige Zuordnung wird der Kanal sicherheitshalber nicht geöffnet, sondern bleibt verschlossen.

## #158 — Gemeinsame Admin-Session neustartsicher gemacht

**Ausgangslage:** Discord- und Twitch-Admin-Dashboard verwendeten zwar denselben Cookie-Namen, der zentrale Rust-Dienst hielt seine Sessions aber nur im Arbeitsspeicher. Nach einem Neustart war das Cookie wertlos. Zusätzlich konnte ein älteres Cookie mit demselben Namen das gültige Cookie überdecken und erneut eine Login-Schleife auslösen.

**Was wurde geändert:** Zentrale Admin-Sessions werden mit ihrer 14-Tage-Laufzeit in der gemeinsamen Datenbank gespeichert und beim Start wieder geladen. Bei Anfragen mit mehreren gleichnamigen Alt-Cookies werden alle Kandidaten geprüft und der tatsächlich gültige Sessionwert verwendet.

**Wie es jetzt läuft:** Ein Discord-Login stellt weiterhin genau ein gemeinsames Cookie aus. Dieses Cookie gilt für beide Admin-Dashboards, überlebt Neustarts und wird gleitend verlängert. Alte, ungültige Cookie-Dubletten blockieren eine gültige Session nicht mehr.

## #157 — Admin-Login über Haupt- und Admin-Domain stabilisiert

**Ausgangslage:** Der gemeinsame Discord-Rücksprung lief auf der Hauptdomain, während beide Admin-Oberflächen anschließend auf die Admin-Subdomain wechselten. Die neue Rust-Implementierung setzte das Login-Cookie jedoch nur für die Hauptdomain. Auf der Admin-Subdomain fehlte die Session deshalb sofort wieder und der Login begann erneut.

**Was wurde geändert:** Die Rust-Implementierung übernimmt jetzt das bestehende Python-Verhalten vollständig: Das Session-Cookie wird für die gemeinsame Community-Domain ausgestellt und das Rücksprungziel wird aus der fest konfigurierten öffentlichen Admin-Adresse gebildet statt aus dem eingehenden Host-Header.

**Wie es jetzt läuft:** Discord meldet weiterhin ausschließlich an den registrierten Rücksprung auf der Hauptdomain zurück. Danach bleibt dieselbe Session auf der Admin-Subdomain gültig, sodass Discord- und Twitch-Admin-Dashboard ohne Auth-Schleife auf der richtigen Domain öffnen.

## #156 — member-access-Endpunkt im Python-Master-Broker hinzugefügt

**Ausgangslage:** Nach dem Rollback des Discord-Bots auf Python (#154) fehlte im Python-Master-Broker der Endpunkt `/internal/master/v1/discord/member-access`. Das Rust-Dashboard (`dl-web`) benötigt diesen jedoch beim Login zur Überprüfung der Admin-Rechte und Rollen, was zu einer Endlosschleife (Auth Loop) führte.

**Was wurde geändert:** Der Route-Eintrag für `/internal/master/v1/discord/member-access` und der zugehörige Handler `_handle_member_access` wurden in `service/master_broker.py` implementiert.

**Wie es jetzt läuft:** Wenn das Rust-Dashboard die Mitgliedsrechte abfragt, liefert der Python-Broker die Daten aus dem Discord-Cache (Rollen, Administrator-Rechte und Anzeigename) analog zur früheren Rust-Broker-Implementierung aus. Die Authentifizierungsschleife ist dadurch behoben.

## #155 — Scam-Schutz: Fehlentscheidung per Discord-Button zurücknehmen

**Ausgangslage:** Der Twitch-Scam-Schutz meldet seine Aktionen (automatischer Bann oder Moderationsvorschlag) nach Discord. Eine falsch getroffene Entscheidung ließ sich von dort aber nicht direkt korrigieren — die Rücknahme ging nur über einen Chat-Befehl im jeweiligen Twitch-Kanal.

**Was wurde geändert:** Die Discord-Meldungen tragen jetzt einen „Rückgängig"-Knopf. Ein Klick nimmt die gemeldete Entscheidung zurück.

**Wie es jetzt läuft:** Beim Klick ruft der Discord-Bot die interne Twitch-Schnittstelle auf; dort wird — sofern ein Bann gesetzt war — entbannt und die Entscheidung als Fehlalarm vermerkt. Dieser Fehlalarm fließt in den selbstlernenden Schutz ein, der daraus lernt und ähnliche Fälle künftig seltener falsch einstuft. Nach erfolgreicher Rücknahme wird der Knopf deaktiviert und als „Zurückgenommen" angezeigt; bei einem Fehlschlag bleibt er nutzbar und es kommt ein kurzer Hinweis. Der Knopf wirkt nur bis zum nächsten Neustart des Discord-Bots — die dauerhaften Rücknahme-Wege bleiben der Chat-Befehl und (künftig) das Dashboard.

## #154 — Discord-Bot zurück auf Python

**Ausgangslage:** Der Rust-Bot (`dl-bot`) deckte rund 50 % der Python-Funktionalität ab; der Rest war nicht portiert und sollte vorerst nicht nachgezogen werden.

**Was wurde geändert:** `deadlock-bot.service` startet jetzt wieder über `run_bot_with_infisical.sh` statt `run_dl_bot_service.sh` — der Python-Bot (`main_bot.py`) ist damit der aktive Gateway-Owner.

**Wie es jetzt läuft:** Alle 44 Cogs werden geladen; der vollständige Python-Funktionsumfang ist wieder verfügbar. Der Rust-Bot (`deadlock-bot-rust.service`) bleibt installiert, aber inaktiv.

## #153 — Zwei weitere Statistik-Befehle zurück: Member-Events & Ping-Check

**Problem:** Beim Wiederherstellen der Statistik-Befehle (#152) standen noch zwei weitere Befehle aus den alten Cogs aus, die ebenfalls seit dem Rust-Umbau fehlten: die persönliche Event-Historie eines Mitglieds und der Ping-Eignungs-Check.

**Was wurde geändert:** Zwei zusätzliche Prefix-Befehle sind zurück:
- `!memberevents` / `!mevents [@User] [Anzahl]` — listet die letzten Server-Events eines Mitglieds (Beitritt, Verlassen, Bann, Entbannung).
- `!checkping [@User]` — prüft, ob ein Mitglied gepingt werden dürfte, und nennt den Grund.

**Wie es jetzt funktioniert:** `!memberevents` zeigt die neuesten Events zuerst (Standard 10, maximal 50), jeweils mit Symbol, Typ und Zeitpunkt; ohne Daten kommt ein kurzer Hinweis. `!checkping` wertet dieselben Kriterien wie früher aus (Aktivität der letzten zwei Wochen, typische Online-Zeiten ± zwei Stunden, passender Wochentag, Ping-Rate-Limit) und meldet „kann/kann nicht gepingt werden" samt Begründung. Beide stehen allen offen. Hinweis: Der Versand-Teil des Ping-Systems (aktives Anschreiben inaktiver Mitglieder) ist weiterhin nicht portiert — `!checkping` ist reine Anzeige; die Rate-Limit-Begründung greift daher praktisch nicht.

**Betroffen:** Alle Mitglieder (beide Befehle wieder abrufbar).

## #152 — Statistik-Befehle sind zurück

**Problem:** Seit dem Rust-Umbau fehlten sämtliche Statistik-Befehle. Voice- und Text-Aktivität wurde im Hintergrund weiter gesammelt, war für Mitglieder aber nirgends abrufbar — weder die persönlichen Werte noch die Bestenlisten. Die Annahme, ein Web-Dashboard ersetze das, trug nicht: Die zugehörige Web-Anzeige läuft derzeit nicht, die Statistik-Anzeige war damit faktisch tot.

**Was wurde geändert:** Sieben Befehle sind zurück und verhalten sich wie früher:
- `!vstats [@User]` — persönliche Voice-Statistik (Gesamtzeit, Punkte, laufende Session, Grace-Rolle)
- `!vleaderboard` / `!vlb` / `!voicetop` — Voice-Bestenliste (Top 10)
- `!useranalysis` / `!ua` / `!analyze [@User]` — Aktivitäts-Analyse (Server-History, Voice, Nachrichten, Top-Mitspieler, Muster)
- `!myactivity [@User]` — eigenes Aktivitätsmuster der letzten zwei Wochen
- `!tleaderboard` / `!tlb` / `!texttop` — Text-Bestenliste (Top 10)
- `!messagestats` / `!msgstats [@User]` — Nachrichten-Statistik
- `!serverstats` — Server-Gesamtübersicht (nur Admins)

**Wie es jetzt funktioniert:** Die Befehle stehen allen offen; nur die Server-Übersicht verlangt Admin-Rechte (Nicht-Berechtigte werden still ignoriert). Bestenlisten zeigen die Top 10 nach Punkten, mit Sekunden bzw. Nachrichten als Gleichstand-Kriterium, und im Fußzeilen-Text die eigene Platzierung. Die persönliche Voice-Statistik rechnet eine gerade laufende Voice-Session live dazu. Die Voice-Abfragen sind gegen Spam begrenzt (höchstens fünf Anfragen pro 30 Sekunden je Nutzer; danach ein kurzer Hinweis mit Restzeit). Namen, Rollen und Server-Name kommen aus dem Gateway-Cache; Zahlen sind wie im Original formatiert (Bestenlisten mit Punkt-Tausendertrennung, Nachrichten-/Server-Werte mit Komma).

**Betroffen:** Alle Mitglieder (Statistiken und Bestenlisten wieder abrufbar) sowie Admins (Server-Übersicht).

## #151 — Regelwerk-Panel lässt sich wieder posten

**Problem:** Seit dem Rust-Umbau fehlte der Admin-Befehl, mit dem das Regelwerk-Panel im Regel-Kanal gesetzt wird. Dieses Panel mit dem „Hier starten"-Button ist der Einstiegspunkt ins geführte Onboarding — ohne den Befehl ließ es sich nach Änderungen nicht neu posten.

**Was wurde geändert:** Der Admin-Befehl ist zurück. Er postet das Regelwerk-Embed samt „Hier starten"-Button — Text, Farbe und Button wortgleich zur bisherigen Fassung.

**Wie es jetzt funktioniert:** Ein Admin ruft den Befehl auf; das Panel landet im fest hinterlegten Regel-Kanal — genau wie im Original, das immer diesen Kanal bedient, unabhängig davon, wo der Befehl ausgelöst wird. Der „Hier starten"-Button öffnet weiterhin den privaten Onboarding-Thread. In einer DM (außerhalb eines Servers) lehnt der Befehl mit einem kurzen Hinweis ab.

**Betroffen:** Admins (Panel wieder setzbar) und neue Mitglieder (Onboarding-Einstieg bleibt erreichbar).

## #150 — Bild-Moderation: reine Bild-Nachrichten werden wieder geprüft

**Problem:** Die KI-Moderation bewertete bisher nur den Text einer Nachricht. Eine Nachricht ganz ohne Text — nur mit einem Bild-Anhang — wurde komplett übersprungen. Damit rutschte rein bildbasierter Scam (gefälschte Krypto-Auszahlungen, Casino-/Wett-Promos, Promo-Codes) und bildbasiertes NSFW durch die automatische Erkennung. Auch der Account-Takeover-Alarm (verdächtige Bild-Flut über mehrere Kanäle in Sekunden) lieferte den Mods keinerlei Bild-Einschätzung mehr, sondern nur den deterministischen Muster-Treffer.

**Was wurde geändert:** Nachrichten mit Bild-Anhängen gehen jetzt durch die Bild-Erkennung (Vision). Die Moderation schickt zusätzlich zum Text bis zu vier Bilder an das KI-Modell und wertet dieselbe Klassifikation aus wie beim Text. Eine Nachricht wird nur noch dann ignoriert, wenn sie weder Text noch Bilder hat. Beim Account-Takeover holt der Schutz zusätzlich ein begleitendes Bild-Urteil ein und hängt es als Kontext an die Mod-Meldung.

**Wie es jetzt funktioniert:** Beim Eingang einer Nachricht werden die Bild-Anhang-URLs mitgeführt (gefiltert auf echte Bild-Anhänge). Hat die Nachricht Bilder, läuft sie über den Bild-Pfad: Text plus die ersten vier Bilder gehen an das Modell, das mit demselben Schema antwortet (Urteil, Kategorie, Konfidenz) — und es gelten exakt dieselben Schwellen wie beim Text (automatisches Löschen erst ab sehr hoher Konfidenz, Mod-Vorschlag ab der bekannten Vorschlags-Schwelle). Es wird also nicht schärfer gehandelt als zuvor, nur die bisher blinde Bild-Spur ist abgedeckt. Der Takeover-Pfad bleibt deterministisch und reversibel; das KI-Bild-Urteil dort ist reines Mod-Kontext-Label und entscheidet die Quarantäne nicht mit. Greift das Modell mal nicht, fällt die Bewertung still aus statt zu blockieren.

**Betroffen:** Alle Mitglieder (bildbasierter Scam/NSFW wird wieder automatisch erfasst) und das Mod-Team (bessere Takeover-Meldungen).

## #149 — Verbund-Broker kann „ist dieser Nutzer noch auf dem Server?" beantworten

**Problem:** Der Steam-Bot räumt die Steam-Verknüpfung eines Mitglieds auf, wenn es den Discord-Server verlässt (Freundschaft beenden, Verknüpfung archivieren). Verpasst er das Austritts-Ereignis (z. B. weil er gerade neu startet), gibt es einen stündlichen Nachlauf, der verwaiste Verknüpfungen aufspürt. Dafür muss er aber zuverlässig wissen, ob ein Nutzer **wirklich** weg ist — und genau diese Auskunft konnte der zentrale Master-Broker bisher nicht geben. Es ließ sich nur „aus dem Gedächtnis" (Cache) raten, ohne sicher zwischen „bestätigt weg", „noch da" und „weiß ich gerade nicht" zu unterscheiden.

**Was wurde geändert:** Der Broker bekommt eine neue, nur lokal erreichbare Abfrage, die für eine Server-/Nutzer-Kombination einen klaren Status liefert: **anwesend**, **abwesend** oder **unbekannt**. Sie schaut erst ins schnelle Gedächtnis des Bots und fragt bei einem Treffer-Fehlschlag live bei Discord nach.

**Wie es jetzt funktioniert:** Ist der Nutzer im Mitglieder-Gedächtnis, lautet die Antwort sofort „anwesend". Fehlt er dort, fragt der Bot live nach: antwortet Discord mit „gibt es nicht" (404), gilt der Nutzer als **abwesend**; bei jedem anderen Fehler (z. B. Rate-Limit) lautet die Antwort bewusst **unbekannt** statt „weg" — so kann eine vorübergehende Störung niemals fälschlich eine Aufräum-Aktion auslösen. Die Abfrage ist rein lesend, nur über die lokale Schleife erreichbar und ändert keine bestehende Funktion.

**Betroffen:** Interne Infrastruktur (Grundlage für den korrekten Steam-Verknüpfungs-Nachlauf des Steam-Bots) — keine direkte Nutzeraktion.

## #148 — Coaching-Status erreicht die Website wieder

**Problem:** Die Coaching-Plattform auf der Website zeigt den Stand der Coaching-Anfragen und -Sessions (offen, reserviert, übernommen, abgesagt, abgeschlossen). Nach der Umstellung der Bot-Anbindung wurde dieser Stand **nicht mehr an die Website gemeldet** — der Bot verarbeitete die Anfragen zwar weiter, spiegelte jeden Zustandswechsel aber nicht mehr an die Website-Schnittstelle. Ergebnis: die Website-Ansicht wäre nach dem Cutover stehengeblieben. Außerdem wurde eine Änderung der Coach-Rollen-Zugehörigkeit nur alle 10 Minuten zur Website synchronisiert, nicht zeitnah.

**Was wurde geändert:** Der Bot spiegelt jeden Coaching-Lebenszyklus-Schritt wieder an die Website (Anfrage erstellt/analysiert, für alle geöffnet, von einem Coach übernommen, abgesagt, Session abgeschlossen) — best-effort im Hintergrund, mit demselben Datensatz-Format wie zuvor. Zusätzlich löst das **Hinzukommen** eines Coaches (Coach-Rolle vergeben) jetzt einen zeitnahen Roster-Abgleich aus (mit kurzer Sammelpause, damit mehrere Änderungen zu einem Lauf gebündelt werden) statt erst beim 10-Minuten-Timer.

**Wie es jetzt funktioniert:** Sobald sich an einer Coaching-Anfrage etwas ändert, ist der neue Stand kurz darauf auch auf der Website sichtbar; neue Coaches erscheinen zeitnah im Website-Roster. (Das Entfernen einer Coach-Rolle wird weiterhin vom regelmäßigen 10-Minuten-Abgleich erfasst.)

**Betroffen:** Nutzer der Coaching-Plattform auf der Website und das Coach-Team.

## #147 — Erinnerungs-DMs abbestellbar + Mod-Tag- & Rang-Befehle zurück

**Problem:** Nach der Umstellung fehlten mehrere Befehle, die es vorher gab. Am wichtigsten: Wer die automatischen **„Wir vermissen dich"-Erinnerungs-DMs** (an länger inaktive Stamm-Mitglieder) nicht (mehr) wollte, hatte **keine Möglichkeit mehr, sie abzustellen** — der Bot verschickte sie weiter, aber der Aus-Schalter war weg. Außerdem fehlten dem Mod-Team die **`/mod-tag`**-Befehle (Mitglieder mit Tags wie „Ragebaiter" markieren/entfernen/auflisten) und die **`!rrang`**-Verwaltung des Rang-Voice-Systems (u. a. das Rang-System pro Sprachkanal an-/ausschalten) sowie der Admin-Test `!nudgesend`.

**Was wurde geändert:** Neu sind die Slash-Befehle **`/retention-optout`** (keine Erinnerungs-DMs mehr) und **`/retention-optin`** (wieder aktivieren) — der gewählte Zustand wird gespeichert, und der Versand-Mechanismus überspringt abgemeldete Nutzer bereits. Für Mods: **`/mod-tag set|remove|list`** (an die bestehende Tag-Logik angebunden, serverseitig auf „Nachrichten verwalten" beschränkt) und die **`!rrang`**-Befehlsgruppe (`toggle` schaltet das Rang-System je Kanal um und merkt sich das dauerhaft; dazu Status-/Anker-/Rollen-/Kanal-Übersichten und ein manuelles „aktualisieren"). `!nudgesend`/`!t30` schickt die Steam-Verknüpfungs-Erinnerung testweise an ein Ziel (respektiert Opt-out und ausgenommene Rollen).

**Wie es jetzt funktioniert:** Jeder kann seine Erinnerungs-DMs selbst per `/retention-optout` aus- und per `/retention-optin` wieder anschalten. Mods vergeben/entfernen/listen Mod-Tags direkt per Slash-Befehl und steuern das Rang-Voice-System wieder über `!rrang`.

**Betroffen:** Alle, die Erinnerungs-DMs steuern wollen, sowie das Mod-Team.

## #146 — Verbund-Broker liefert Discord-Nachrichten-IDs wieder als Text

**Problem:** Über den zentralen Master-Broker posten andere Bots (u. a. der Twitch-Bot) ihre Discord-Nachrichten und bekommen danach eine Bestätigung mit der ID der erstellten Nachricht zurück. Beim Umbau des Brokers von Python auf das neue System wurde diese ID versehentlich als reine Zahl ausgeliefert statt — wie zuvor und wie es die Discord-API selbst tut — als Text. Discord-IDs sind so groß, dass viele Systeme sie als Zahl nicht mehr verlustfrei darstellen; aufrufende Bots erwarten deshalb Text. Der Twitch-Bot verwarf die Antwort dadurch als unlesbar, merkte sich die erstellte Nachricht nie und postete „ist live"-Pings mehrfach (siehe Twitch-Bot #221).

**Was wurde geändert:** Der Broker liefert die Nachrichten-ID in der Post-Bestätigung wieder als Text — zurück zum ursprünglichen Vertrag.

**Wie es jetzt funktioniert:** Nach erfolgreichem Posten wandelt der Broker die intern als Zahl geführte ID vor dem Versenden in Text um. Damit passt die Antwort wieder zu dem, was alle aufrufenden Bots erwarten, und große IDs gehen nicht verloren. Andere Felder der Antwort (z. B. die Kanal-ID) bleiben unverändert.

**Betroffen:** Alle Bots, die Discord-Nachrichten über den Master-Broker posten — sichtbar wurde es zuerst an den doppelten Live-Pings des Twitch-Bots.

## #145 — Turnier-Automatik & Moderations-Einspruch zurück (weitere Cutover-Lücken)

**Problem:** Mehrere Funktionen, die es vor der Umstellung auf das neue System gab, fehlten danach. Turnier: (1) Der **Auto-Balance** — ein Hintergrund-Vorgang, der angemeldete Solo-Spieler regelmäßig nach Rang automatisch auf Teams verteilt und Teilteams auffüllt — lief gar nicht mehr. (2) Das **Anmelde-Panel** ließ sich nicht neu in einen Kanal posten (bestehende Panels funktionierten, ein verlorenes konnte aber nicht ersetzt werden). (3) `!balance start` mit mehr als 12 Leuten im Voice schnitt **stillschweigend die rangschwächsten ab**, statt das zu sagen. Moderation: (4) Wer automatisch gebannt/stummgeschaltet wurde, bekam **keine Einspruchsmöglichkeit** mehr; (5) die öffentliche „Scam erkannt"-Notiz erschien nur in *einem* der betroffenen Kanäle; (6) im Mod-Log fehlten Kontextdaten.

**Was wurde geändert:** Der Auto-Balance läuft wieder als 5-Minuten-Vorgang, zeilengenau nach der alten Logik (volle Teams bleiben unangetastet, der Rest wird nach Rang-Score per Snake-Draft auf neue „Team A/B/…"-Teams verteilt). Ein neuer Admin-Befehl **`/turnierpanel`** postet das Anmelde-Panel mit den Solo-/Team-Knöpfen neu in den aktuellen Kanal. `!balance start` bricht bei >12 Spielern jetzt mit klarer Anweisung ab (Teilnehmer gezielt per `!balance manual @…` wählen) statt Leute zu verlieren. Die Auto-Moderations-DM an Betroffene trägt wieder einen **„Einspruch"-Knopf** → kurzes Begründungs-Formular → der Einspruch landet im Mod-Kanal. Die Scam-Notiz geht in **jeden** betroffenen Kanal (einmal pro Kanal), und das Mod-Log zeigt wieder Account-Alter, Zeit seit Beitritt, Aktivitätsfenster, Signale und die ausgeführten Aktionen — plus einen „Entbannen"-Knopf, wenn tatsächlich gebannt wurde.

**Wie es jetzt funktioniert:** Anmeldungen werden im Hintergrund automatisch zu Teams; Admins posten das Panel bei Bedarf neu; große Voice-Runden bekommen eine klare Ansage statt stillem Datenverlust; gebannte Nutzer können direkt aus der DM Einspruch einlegen; und das Team sieht im Log alle Eckdaten eines Vorfalls auf einen Blick.

**Betroffen:** Turnier-Teilnehmer und -Admins, das Mod-Team und alle von der Auto-Moderation Betroffenen.

## #144 — Master-Broker kann jetzt Discord-Rollen anlegen

**Ausgangslage:** Andere Bots im Verbund (z. B. der Twitch-Bot) haben keinen eigenen Discord-Zugang — sie lassen alle Discord-Aktionen über den zentralen Master-Broker laufen. Der konnte Mitglieder zu bestehenden Rollen hinzufügen, aber keine neue Rolle erstellen. Für die automatische Anlage der Twitch-„ist live"-Ping-Rolle fehlte genau das.

**Was geändert wurde:** Der Broker hat einen neuen internen Endpunkt, der eine neue, erwähnbare Rolle in der Guild anlegt und ihre ID zurückgibt.

**Wie es funktioniert:** Der aufrufende Bot schickt Guild, Rollenname und einen Audit-Grund. Der Broker sucht zuerst (best effort) eine gleichnamige Rolle, um Duplikate zu vermeiden, und legt nur sonst eine neue an. Der Endpunkt ist wie alle Broker-Aktionen nur lokal erreichbar (Loopback + interner Token) und gegen doppelte Aufrufe abgesichert. Voraussetzung ist, dass der Bot in der Guild die Berechtigung „Rollen verwalten" hat — fehlt sie, meldet Discord einen Fehler, der sauber durchgereicht wird.

## #143 — Moderation greift wieder durch + TempVoice-Eingabefenster reagieren

**Problem:** Zwei Cutover-Regressionen. (1) Die Moderations-Review-Knöpfe (Annehmen / Bannen / Ablehnen), mit denen das Team einen automatisch erkannten Scam-/Spam-Fall bestätigt, **führten gar keine Aktion mehr aus** — sie setzten nur intern einen Status, löschten aber die Nachricht nicht, timeouteten/bannten den Account nicht und schrieben kein Aktions-Log. Ein bestätigter Fall blieb faktisch folgenlos. Zusätzlich behandelte der automatische Account-Takeover-Schutz (Account postet in Sekunden Bilder über mehrere Kanäle = typisches Hack-Muster) den Fall als **harten Ban** statt als reversible Maßnahme. (2) Im TempVoice-Panel reagierten die Eingabefenster **Limit setzen**, **Lane umbenennen** und **Preset speichern** nicht — egal was man eintippte, kam nur „Bitte … eingeben".

**Was wurde geändert:** Die Review-Knöpfe lösen jetzt die echten Aktionen aus: *Annehmen* löscht die Nachricht und setzt einen 24-Stunden-Timeout, *Bannen* bannt den Account (inkl. Löschen der letzten Nachrichten) und *Ablehnen* öffnet ein Pflichtfeld für die Begründung; jede Aktion landet im Mod-Log und ist gegen Doppelklick abgesichert. Der Takeover-Schutz nimmt statt des Dauerbans einen **reversiblen 24-Stunden-Timeout** plus eine Hinweis-DM an den Betroffenen (möglicher Hack, Passwort ändern, 2FA, beim Team melden) — so trifft ein Fehlalarm niemanden dauerhaft. Bei den TempVoice-Eingabefenstern lag's daran, dass der eingetippte Text aus dem falschen internen Feld gelesen wurde (das neue System liefert Modal-Eingaben anders aus als das alte) — jetzt aus dem richtigen.

**Wie es jetzt funktioniert:** Mod klickt „Annehmen" → Nachricht weg + 24h-Timeout + Log; „Bannen" → Ban + Log; „Ablehnen" → Grund eingeben → Fall geschlossen + Log. Takeover-Verdacht → 24h-Timeout + Aufklärungs-DM statt Ban. Limit/Umbenennen/Preset im Voice-Panel übernehmen die Eingabe wieder.

**Betroffen:** Das Mod-Team (Review-Knöpfe), alle von Auto-Moderation/Takeover-Schutz Betroffenen, und alle, die im TempVoice-Panel Limit/Umbenennen/Preset nutzen.

## #142 — TempVoice-Panel: tote Knöpfe reagieren wieder (Lurker, Tag-Filter, Mindest-Rang, Modus)

**Problem:** Seit der Umstellung der Bot-Anbindung auf das neue System antwortete ein Teil der Knöpfe im TempVoice-Bedienpanel nur noch mit „Diese Funktion ist im neuen System noch nicht freigeschaltet" — also gar nicht. Betroffen waren: **Lurker** (still/ohne Limit-Slot beitreten), **Tag-Filter** (Lane nach Mindest-Alter/Tonfall/Ragebaiter filtern), der zweistufige **Mindest-Rang** (Haupt-Rang → Sub-Rang) und **Modus wechseln** (Casual/Ranked/Street Brawl/Off Topic). Die Knöpfe waren zwar da und nahmen Klicks an, dahinter lag aber kein Handler — die eigentliche Logik war schon umgezogen, wurde aber von keinem Knopf aufgerufen.

**Was wurde geändert:** Die fünf Knopf-/Auswahl-Flows sind jetzt mit der vorhandenen Backend-Logik verdrahtet. Weil das neue System Knöpfe zustandslos verarbeitet (jeder Klick ist für sich, anders als das alte, das ein Bedien-Fenster im Speicher hielt), wurden zwei Abläufe angepasst: Beim Tag-Filter übernimmt jede Auswahl sofort (kein separater „Speichern"-Schritt mehr), und beim Mindest-Rang wird der zuerst gewählte Haupt-Rang gemerkt, bis der Sub-Rang (1–6) folgt, und dann zu z. B. „Phantom 3" kombiniert. Zusätzlich behoben: Der Mindest-Rang lehnte intern jeden Wert mit Sub-Rang als „unbekannt" ab, weil er gegen die falsche Rang-Tabelle prüfte — jetzt wird gegen die Punktetabelle geprüft, die Sub-Ränge kennt.

**Wie es jetzt funktioniert:** Lurker-Knopf schaltet den eigenen Lurker-Status um (Rolle + Nickname + Limit-Anpassung). Tag-Filter öffnet drei Auswahlmenüs, die je sofort greifen und blockierte Mitglieder aus der Lane halten. Mindest-Rang setzt nach Haupt- und Sub-Rang-Wahl die Zutrittsschranke (nur in Comp-/Ranked-Lanes, und man kann keinen Rang über dem eigenen setzen). Modus wechseln verschiebt die Lane in die passende Kategorie (Ranked verlangt einen verifizierten Rang). Owner-Knopf wie Region/Limit/Kick/Ban waren nie betroffen.

**Betroffen:** Alle, die im TempVoice-Panel Lurker, Tag-Filter, Mindest-Rang oder Modus-Wechsel nutzen.

## #141 — Beta-Einladung: langsame Button-Klicks geben endlich wieder Antwort

**Problem:** Seit die Discord-Anbindung des Bots auf das neue (Rust-)System umgestellt wurde, kam bei Button-Klicks, deren Verarbeitung länger als ~2 Sekunden dauert, **gar keine** sichtbare Antwort mehr. Der Bot setzt bei solchen langsamen Aktionen korrekt eine kurze „lädt…"-Anzeige, aber die eigentliche Antwort danach verfiel still. Besonders betroffen war der Beta-Einladungs-Flow: der Schritt „Freundschaft prüfen" spricht immer mit Steam (Freundschafts-Check plus Einladungs-Anfrage an den Spiel-Koordinator) und braucht damit fast immer mehr als 2 Sekunden — der User klickte und sah nichts, egal ob die Einladung rausgegangen wäre oder Steam sie ablehnt (z. B. weil das Steam-Konto noch eingeschränkt/Limited ist).

**Was wurde geändert:** Die Nachreich-Nachricht nach der „lädt…"-Anzeige adressiert Discord über die Anwendungs-Identität des Bots. Die REST-Verbindung des Bots — eine eigene Verbindung getrennt von der Gateway-Verbindung — hatte diese Identität nie gesetzt bekommen, weshalb Discord jede solche Nachreich-Nachricht ablehnte, bevor sie überhaupt rausging. Der Bot setzt diese Identität jetzt einmalig beim Start.

**Wie es jetzt funktioniert:** Langsamer Button-Klick → kurze „lädt…"-Anzeige → die echte Antwort wird zuverlässig nachgereicht: Einladung verschickt, bereits im Besitz, oder der Hinweis „dein Steam-Konto ist noch eingeschränkt — sobald 5 $ ausgegeben sind, holt der Bot die Einladung automatisch nach". Schnelle Klicks (unter ~2 Sekunden) waren nie betroffen, weil deren Antwort auf einem anderen Weg rausgeht, der die Identität nicht braucht.

**Betroffen:** Alle, die die Beta-Einladung bestätigen, sowie weitere Aktionen, die länger als ~2 Sekunden brauchen.

## #140 — „Rang prüfen"-Button: genug Zeit für die Antwort

**Problem:** Der „Rang prüfen"-Button im Steam-Panel lieferte kein Ergebnis. Neben der eigentlichen Abruf-Logik (im Steam-Dienst nachgezogen) gab es ein Timing-Problem auf Bot-Seite: Button-Klicks räumten dem Steam-Dienst nur wenige Sekunden für die Antwort ein. Ein echter Rang-Abruf (Profilkarte über den Spiel-Datendienst) dauert aber länger — der Bot hätte die Antwort also abgeschnitten, bevor sie fertig war.

**Was wurde geändert:** Für genau diesen Button wartet der Bot jetzt deutlich länger auf die Antwort des Steam-Dienstes statt der kurzen Standard-Schranke. Wie bei langsamen Befehlen üblich wird die Interaktion nach kurzer Zeit auf „lädt…" gesetzt, damit der Klick nicht verfällt, während im Hintergrund der Rang geholt wird.

**Wie es jetzt funktioniert:** Klick auf „Rang prüfen" → kurze Warteanzeige → das Ergebnis wird nachgereicht, sobald der Rang ermittelt ist. Andere Panel-Knöpfe (z. B. „Steam verknüpfen") behalten ihr schnelles Standard-Verhalten.

**Betroffen:** Nutzer des „Rang prüfen"-Buttons im Steam-Panel.

## #139 — „Verified"-Rolle: Schluss mit dem Flackern (nur noch ein Verantwortlicher)

**Problem:** Seit der Steam-Teil in einen eigenen Dienst ausgelagert wurde, verwalteten **zwei** Systeme gleichzeitig die „Verified"-Rolle: der alte bot-interne Abgleich (stündlich) und der neue Steam-Dienst. Beide lasen zwar denselben Datenstand, entschieden aber unabhängig voneinander über Vergeben und Entziehen. Trafen ihre Läufe ungünstig aufeinander, konnte die Rolle kurzzeitig flackern (vergeben → entzogen → wieder vergeben) oder uneinheitlich gesetzt sein.

**Was wurde geändert:** Der alte bot-interne Verified-Abgleich wurde abgeschaltet. Zuständig für die „Verified"-Rolle ist jetzt ausschließlich der Steam-Dienst.

**Wie es jetzt funktioniert:** Die Rolle kommt unmittelbar, sobald die Steam-Freundschaft mit dem Bot bestätigt ist, und wird danach von genau einer Stelle konsistent gepflegt — regelmäßiger Abgleich plus Entzug, wenn die Freundschaft wegfällt. Da kein zweites System mehr gegensteuert, gibt es kein Flackern und keine widersprüchlichen Zustände mehr.

**Betroffen:** Alle mit „Verified"-Rolle, besonders wer das Kommen und Gehen der Rolle bemerkt hat.

## #138 — /checkrank & weitere Steam-Befehle antworten wieder

**Problem:** `/checkrank` (und weitere Steam-Befehle, die nur Text oder ein Info-Embed ohne Knöpfe zurückgeben — etwa die Rang-Einzelabfrage oder die Verknüpfungs-Statusanzeige) brachen seit dem Umbau der Steam-Anbindung kommentarlos ab: Der Befehl lief intern auf einen Fehler und schickte deshalb gar keine Antwort. Für den Nutzer sah es so aus, als würde der Befehl einfach nichts tun.

**Was wurde geändert:** Beim Zusammenbauen der Antwort wurde für den Fall „diese Antwort hat keine Knöpfe" der falsche Platzhalter verwendet. Die aktuelle Discord-Bibliotheksversion akzeptiert dafür keinen leeren Wert mehr, sondern verlangt einen eigenen „kein-Element"-Platzhalter — sonst wirft sie einen Fehler, bevor die Nachricht rausgeht. Genau dieser Platzhalter wird jetzt gesetzt.

**Wie es jetzt funktioniert:** Antworten ohne Knöpfe (reiner Text / Embed) werden wieder korrekt verschickt. Befehle mit Knöpfen (z. B. das Verknüpfungs-Panel) waren nie betroffen, weil dort ohnehin ein echtes Bedienelement mitgeschickt wurde. `/checkrank` und Co. liefern damit wieder ihr Ergebnis.

**Betroffen:** Alle, die `/checkrank`, die Rang-Einzelabfrage oder die Steam-Statusbefehle nutzen.

## #137 — Server-Statistik: „Vor Tracking" sauber von „Unbekannt" getrennt

**Ausgangslage:** Im Mitgliederquellen-Donut stand „Unbekannt" bei ~30 % (621 von 2020 Beitritten) — auffällig hoch. Die Analyse zeigt: das sind fast ausschließlich Beitritte von *bevor* der Bot überhaupt erfassen konnte, woher jemand kam. Die Quellen-Erkennung (welche Einladung wurde benutzt) ging erst am 18.02.2026 live; alles davor wurde pauschal „unbekannt" gestempelt. Diese Quelle lässt sich rückwirkend nicht rekonstruieren — sie wurde damals nirgends festgehalten, und Discord verrät selbst im Nachhinein nicht, über welche Einladung jemand kam.

**Was wurde geändert:** Diese Alt-Beitritte bekommen eine eigene, ehrliche Kategorie „Vor Tracking", statt im „Unbekannt"-Topf zu liegen. „Unbekannt" ist damit echten, jüngeren Erkennungs-Fehlschlägen vorbehalten — aktuell praktisch null, weil die Erkennung seit dem Frühjahr sauber läuft.

**Wie es funktioniert:** Ein quellenloser Beitritt zählt als „Vor Tracking", wenn er entweder nachträglich verbucht wurde oder zeitlich vor dem Erkennungs-Start (18.02.2026, 16:22 Uhr) liegt. Bewusst zeit- statt nur markergebunden: ein *künftiger* echter Ausfall der Erkennung bleibt sichtbar „Unbekannt" und wird nicht stillschweigend als „Vor Tracking" weggebügelt. In Zahlen: „Unbekannt" 621 → 0, „Vor Tracking" 621 (30,7 %); alle anderen Töpfe unverändert. Greift mit der neuen (Rust-)Auswertung, der Donut zeigt es, sobald diese aktiv ist.

**Betroffen:** Betrachter der Server-Statistik im Admin-Bereich.

## #136 — Support-Bot prüft im Ticket den echten Twitch-Status statt zu raten

**Ausgangslage:** Seit #132 hilft der Ticket-Bot gezielt bei Problemen. Aber bei „der Bot kommt nicht in meinen Stream" oder „ich habe autorisiert, aber es steht auf inaktiv" konnte er nur allgemeine Schritte nennen — den tatsächlichen Autorisierungs-Stand des Fragenden kannte er nicht und musste raten.

**Was wurde geändert:** Der Ticket-Bot kann bei einem eigenen technischen Problem jetzt den echten Twitch-Status des Fragenden nachsehen (verbunden? fehlende Berechtigungen? Neu-Autorisierung nötig? aktiv?) und bei Bedarf redigierte Log-Zeilen zum Fragenden heranziehen. Das ist streng auf den Fragenden selbst beschränkt — auf seine vom Server ermittelte Identität, nie auf einen fremden Account — rein lesend, und es gibt niemals interne oder geheime Daten (Tokens, Pfade) aus. Vor dem Posten prüft eine zweite, unabhängige Sicherheitsstufe jede Antwort und blockiert im Zweifel.

**Wie es funktioniert:** Erkennt der Bot ein eigenes Tech-Problem, fragt er über eine abgesicherte interne Schnittstelle den Status ab und übersetzt ihn in einen konkreten nächsten Schritt (z. B. „dir fehlen Berechtigungen — verbinde den Bot über die Verwaltungsseite neu" statt einer allgemeinen Anleitung). Lässt sich der Status nicht ermitteln, fällt er auf die dokumentierten Selbsthilfe-Schritte zurück, statt etwas zu erfinden. Bei zwischenmenschlichem Streit oder Moderationsfällen schweigt er weiterhin. Jede Antwort und jede bewusste Schweige-Entscheidung wird protokolliert, damit sich die Qualität nachvollziehen lässt.

## #135 — Rust-Neuaufbau: Twitch-Beitritts-Brücke + rückwirkende Neu-Einsortierung

**Ausgangslage:** Mit #131 war die Quellen-Klassifikation der Server-Statistik in Rust nachgebaut und um die fehlende Twitch-Korrektur ergänzt — aber dem Twitch-Teil fehlten noch die *Daten*. Die Zuordnung Einladungscode→Streamer liegt in der Datenbank des Twitch-Bots; die Tabelle, aus der die Deadlock-Auswertung sie lesen würde, war in der gemeinsamen Datenbank leer und wurde nie befüllt. Folge: Beitritte über Streamer-Einladungen zählten weiter als „Bot-Einladung" oder „unbekannt" statt als „Twitch". Besonders unauffällig, weil diese Einladungen vom Bot selbst erzeugt werden — dadurch rutschten sie in den Topf „Bot-Einladung".

**Was wurde geändert:** Ein kleines Rust-Programm schließt die Lücke in zwei Schritten und läuft alle 6 Stunden automatisch. Erstens holt es die vollständige Streamer↔Einladung-Zuordnung über eine neue interne Schnittstelle des Twitch-Bots und spiegelt sie in die gemeinsame Datenbank. Zweitens geht es alle bisherigen Beitritte durch und sortiert *genau die* nachträglich auf „Twitch" um, deren Einladung sich einem Streamer zuordnen lässt, die aber unter einem anderen Topf gespeichert waren.

**Wie es funktioniert:** Die Umsortierung ist bewusst chirurgisch: Sie verschiebt nur *nach* „Twitch", nie weg davon — dadurch ist sie wiederholbar und kann bei jedem Lauf gefahrlos erneut greifen. Konkret wanderten 20 Alt-Beitritte von „Bot-Einladung" auf „Twitch" (es waren durchweg vom Bot erzeugte Streamer-Einladungen, deshalb vorher falsch im Bot-Topf); die Server-Statistik zeigt damit 25 statt 5 Twitch-Beitritte, „Bot-Einladung" schrumpft entsprechend. Geprüft wurde erst gegen eine Kopie der echten Datenbank — Probelauf ohne Schreiben, dann echt, dann ein zweiter Lauf, der erwartungsgemäß nichts mehr ändert — danach auf der Live-Datenbank. Der 6-Stunden-Takt hält es frisch: kommt ein neuer Streamer dazu, landet seine Zuordnung beim nächsten Lauf in der Tabelle und seine älteren Beitritte werden automatisch nachgezogen.

**Betroffen:** Betrachter der Server-Statistik im Admin-Bereich (Quellen-Aufschlüsselung neuer Mitglieder). Reine Auswertungs- und Datenkorrektur, keine Änderung am Beitritts-Erlebnis selbst.

## #134 — Bot-Texte: durchgängig echte Umlaute und überall Deutsch

**Ausgangslage:** An vielen nutzersichtbaren Stellen standen Umlaut-Ersatzschreibungen (ae/oe/ue/ss statt echtem ä/ö/ü/ß) — entstanden, weil Texte mal mit, mal ohne echte Umlaute getippt wurden. Zusätzlich waren einzelne Abläufe noch auf Englisch, obwohl der Bot sonst durchgängig Deutsch spricht. Am deutlichsten beim Ban-/Einspruch-Ablauf: Ein gesperrtes Mitglied bekam eine englische Ban-Nachricht mit einem „Appeal"-Button, ein englisches Eingabefeld und als Bestätigung „Your appeal was sent to the moderators." — mitten in einem ansonsten deutschen Bot. Auch die Voice-Tracker-Admin-Befehle und die Moderations-Buttons fürs Team waren teils englisch/gemischt.

**Was geändert wurde:** Alle betroffenen Anzeigetexte nutzen jetzt echte Umlaute, und die englischen Stellen sind ins Deutsche übersetzt. Der komplette Einspruch-Ablauf ist deutsch: die Ban-/Timeout-Nachricht, der „Einspruch"-Button, das Eingabefeld, die Bestätigung („Dein Einspruch wurde an das Mod-Team weitergeleitet.") und das Embed, das beim Mod-Team ankommt. Ebenso eingedeutscht: die Voice-Tracker-Konfiguration (Anzeige plus alle Erfolgs-/Fehlermeldungen), die Moderations-Buttons (Annehmen / Ablehnen / Entbannen) und kleinere Onboarding- und Umfrage-Texte.

**Wie es funktioniert:** Reine Text- und Sprachkorrektur, keine Logikänderung. Die Umlaute wurden gezielt pro Wort im jeweiligen Anzeigetext ersetzt — nicht pauschal, damit englische Begriffe, technische Bezeichner und Befehlsnamen unangetastet bleiben. Bewusst englisch bleiben etablierte Begriffe (z. B. Timeout, Ban) sowie alles, was nur intern in Logs steht und kein Mitglied zu sehen bekommt. Abläufe, Buttons und Reihenfolge sind identisch — es liest sich nur sauberer und einheitlich auf Deutsch.

**Betroffen:** gesperrte Mitglieder (kompletter Einspruch-Ablauf), neue Mitglieder (Onboarding-Hinweise), das Mod-Team (Voice-Konfiguration und Moderations-Buttons) sowie generell alle, die DMs und Embeds des Bots lesen.

## #133 — Coaching-Panel: Fragen kommen in den Channel statt in die DMs

**Ausgangslage:** Im Coaching-Panel stand zwar schon die Regel „keine DMs/Freundschaftsanfragen an die Coaches", trotzdem landeten Follow-up-Fragen rund ums Coaching regelmäßig als DM. Die Antwort blieb dann bei einer Person hängen — andere mit demselben Anliegen hatten nichts davon, und Leute mit hilfreichem Input wurden gar nicht erst erreicht.

**Was wurde geändert:** Das Coaching-Panel hat ein zusätzliches Feld bekommen, das für Fragen zum Coaching auf einen eigenen, öffentlichen Fragen-Channel verweist — statt auf DMs. Außerdem ist die Ablauf-Übersicht entschlackt: der interne Zwischenschritt „KI sortiert die Anfrage vor" wird nicht mehr aufgeführt, sodass nur die für Nutzer relevanten Schritte übrig bleiben.

**Wie es jetzt funktioniert:** Unter dem Ablauf zeigt das Panel jetzt einen anklickbaren Verweis auf den Fragen-Channel. Wer etwas wissen will, fragt dort öffentlich; so sehen alle die Antwort und andere mit demselben Thema lesen direkt mit. Der Ablauf nennt nur noch die drei sichtbaren Schritte — Formular ausfüllen, Coaching-Rolle bekommen, Coach meldet sich. Technisch ist beides nur Text im bestehenden Panel-Embed — der Bot pflegt es beim Start automatisch in die schon gepostete Panel-Nachricht ein, es entsteht also kein zweiter Post.

## #132 — Support-Bot im Ticket: hilft bei Problemen, hält sich aus Streit raus

**Ausgangslage:** Der Support-Bot beantwortet nicht nur im FAQ-Bereich Fragen, sondern macht in neu geöffneten Tickets einen automatischen Erstcheck. Dabei war er falsch eingestellt: Bei echten technischen Problemen („der Bot kommt nicht in meinen Stream", „ich habe autorisiert, aber es steht auf inaktiv") schwieg er oft, weil die passenden Hilfe-Themen in seiner Wissensbasis fehlten — und wenn er antwortete, verwies er gern darauf, „ein Ticket aufzumachen", obwohl der Nutzer längst in einem Ticket saß. Bei zwischenmenschlichem Streit dagegen mischte er sich eher ein, statt das den Menschen zu überlassen.

**Was wurde geändert:** Der Ticket-Bot ist neu eingestellt. Er kümmert sich jetzt gezielt um sach- und problembezogene Anliegen (Fragen, „X funktioniert nicht"-Fälle) und hält sich bei Community-Stress — Streit mit anderen, Beschwerden über Mitglieder, Drama — bewusst raus; das übernehmen Menschen. Bei Forderungen oder Erpressungsversuchen gegen das Team zieht er eine klare Grenze, statt darauf einzugehen; bei frechem Ton bleibt er sachlich und hilft trotzdem weiter. Den unsinnigen Verweis „mach ein Ticket auf" innerhalb eines Tickets gibt es nicht mehr. Zusätzlich wurde die Wissensbasis um eine Sammlung häufiger Probleme samt Selbsthilfe-Schritten ergänzt (Twitch-Bot kommt nicht in den Stream, „autorisiert aber inaktiv", Steam-Verbindung/Rang, Beta-Invite, Coaching).

**Wie es jetzt funktioniert:** Sobald in einem Ticket die erste Nachricht kommt, prüft der Bot still, ob er sicher helfen kann. Ist es eine sachliche Frage oder ein konkretes Problem, antwortet er direkt mit den passenden Schritten; geht es um Streit/Drama oder etwas, das eine menschliche Entscheidung braucht, bleibt er still und überlässt es dem Team. Geht es um Druck oder Erpressung, antwortet er knapp und bestimmt, ohne auf die Forderung einzugehen. Damit die Qualität künftig nachvollziehbar bleibt, hält er jetzt fest, was er geantwortet hat und wann er bewusst geschwiegen hat. Die neuen Selbsthilfe-Themen stammen direkt aus echten Support-Fällen, bei denen vorher die passende Antwort fehlte.

## #131 — Rust-Neuaufbau: Server-Statistik + Fix der Beitritts-Quellen-Erkennung

**Ausgangslage:** Die Server-Statistik im Admin-Bereich zeigt aggregierte Zahlen (Mitglieder-Ereignisse, Nachrichten, Voice-Stunden, aktive Nutzer, 30-Tage-Wachstum) und eine Aufschlüsselung, woher neue Mitglieder kommen (öffentlich/Website/Twitch/persönliche Einladung/Bot/unbekannt). Beim Prüfen fiel auf: Beitritte über Twitch-Streamer-Einladungen wurden kaum als „Twitch" gezählt — sie landeten in „persönlich" oder „unbekannt".

**Ursache (zwei Probleme):** (1) Die Zuordnung Einladungscode→Streamer, die für die Twitch-Erkennung nötig ist, liegt in der Datenbank des Twitch-Bots — die Tabelle, aus der die Deadlock-Auswertung sie lesen würde, existiert in der gemeinsamen Datenbank gar nicht und wird nie befüllt. (2) Selbst mit Daten gab es einen Logik-Fehler: Website-Beitritte wurden nachträglich korrekt umsortiert, für Twitch fehlte genau diese Korrektur — ein einmal als „persönlich" markierter Streamer-Beitritt blieb für immer falsch einsortiert.

**Was wurde geändert:** Die Server-Statistik ist in Rust nachgebaut, inklusive der Quellen-Aufschlüsselung. Die Klassifikation ist als eigene, getestete Logik herausgezogen und um die **fehlende Twitch-Korrektur ergänzt**: Lässt sich eine Einladung einem Streamer zuordnen (und ist es keine Website-Quelle), zählt der Beitritt jetzt zuverlässig als Twitch — analog zur schon vorhandenen Website-Korrektur. Die Live-Vanity-Links kommen über die neue Bot-Auskunft (statt direktem Bot-Cache).

**Wie es jetzt funktioniert:** Die Aggregate treffen dieselben Tabellen mit denselben Zeitfenstern wie zuvor; die Quellen-Klassifikation läuft über die korrigierte Logik. Geprüft gegen eine Kopie der echten Datenbank: die Buckets summieren sauber auf alle Beitritte, und die Korrektur greift schon sichtbar (Website-Beitritte werden zur Laufzeit richtig erkannt). Damit der Twitch-Teil auch *Daten* bekommt, fehlt noch der letzte Schritt — eine Brücke, die die Streamer↔Einladung-Zuordnung aus der Twitch-Datenbank in die gemeinsame Datenbank spiegelt; danach zählt die rückwirkende Neu-Einsortierung die bisher falsch zugeordneten Beitritte korrekt um. Die Klassifikations-Logik dafür steht und ist getestet; sie greift automatisch, sobald die Daten da sind.

## #130 — Rust-Neuaufbau: öffentliche Live-Server-Zahlen

**Ausgangslage:** Die Community-Website zeigt Live-Kennzahlen des Discord-Servers — Mitgliederzahl, gerade online, gerade im Voice. Diese öffentliche Schnittstelle war bei der Patchnotes-Umstellung (#128) bewusst zurückgestellt, weil sie Daten direkt aus dem laufenden Bot braucht (die Website-Anzeige selbst blieb so lange auf Python).

**Was wurde geändert:** Die Schnittstelle ist jetzt in Rust. Da der Web-Prozess selbst keine Discord-Verbindung hat, fragt er die Zahlen beim Bot ab — dafür hat der Bot eine neue, rein lesende interne Auskunft bekommen, die Mitglieder-, Online- und Voice-Zahl (sowie den Vanity-Link) aus seinem Live-Zustand liefert. Die Antwort wird wie bisher 30 Sekunden zwischengespeichert, damit häufige Website-Aufrufe den Bot nicht belasten, und trägt die nötigen Zugriffs- und Cache-Hinweise für den Browser.

**Wie es jetzt funktioniert:** Beim Aufruf liefert der Web-Prozess entweder den noch frischen Zwischenspeicher oder holt die aktuellen Zahlen einmal beim Bot und merkt sie sich für 30 Sekunden. Die Online-Zahl ergibt sich aus den als nicht-offline gemeldeten Mitgliedern, die Voice-Zahl aus den aktuell in Sprachkanälen befindlichen Nutzern — genau wie zuvor. Ist der Bot gerade nicht verbunden, kommt dieselbe „keine Daten verfügbar"-Antwort wie im Original statt eines Fehlers. Geprüft über einen nachgestellten Bot: korrekte Zahlen, Zwischenspeicher und Zugriffs-Hinweise. Damit sind alle öffentlichen Schnittstellen in Rust. Derselbe neue Bot-Auskunftsweg speist als Nächstes die interne Server-Statistik im Admin-Bereich.

## #129 — Rust-Neuaufbau: Turnier-Verwaltung (Admin-Aktionen)

**Ausgangslage:** Im Admin-Dashboard lässt sich ein Turnier verwalten — Anmeldezeitraum anlegen und schließen, Teams anlegen und löschen, Spieler einem Team zuweisen, einzeln entfernen oder alle Anmeldungen leeren. Der Spieler-Anmelde-Flow lief schon in Rust (#114), die Verwaltungs-Aktionen aber noch über Python (das war die offen dokumentierte Lücke aus #114).

**Was wurde geändert:** Diese sieben Verwaltungs-Aktionen sind jetzt in Rust nachgebaut. Sie laufen über denselben Turnier-Speicher wie der bereits portierte Anmelde-Flow (gleiche Tabellen, gleiche Regeln) und sind für Turnier-Moderatoren und volle Admins freigegeben. Jede schreibende Aktion ist durch denselben Anti-Fälschungs-Schutz (CSRF) abgesichert wie die übrigen Admin-Aktionen — ohne gültiges Token wird sie abgewiesen.

**Wie es jetzt funktioniert:** Die Aktionen treffen dieselben Tabellen mit denselben Bedingungen wie zuvor: ein neuer Zeitraum deaktiviert automatisch den vorherigen; ein Team anzulegen ist unempfindlich gegen Groß-/Kleinschreibung (gleicher Name → gleiches Team statt Dublette); ein Team zu löschen hängt vorher seine Mitglieder ab; das Schließen ohne Angabe trifft den aktuell aktiven Zeitraum. Große Discord-IDs werden in der Antwort wie bisher als Text ausgegeben (sonst verlieren Browser bei sehr großen Zahlen Stellen). Geprüft gegen eine Kopie der echten Datenbank: ohne Schutz-Token abgewiesen, mit gültiger Sitzung Zeitraum angelegt (201), Team doppelt angelegt erkannt, aktiver Zeitraum geschlossen, alles geleert. Die reine Anzeige-Seite der Turnier-Übersicht (Teilnehmerliste mit Namen, Turnierbaum) braucht noch Daten aus dem laufenden Bot und folgt separat — bis dahin zeigt das Python-Dashboard sie.

## #128 — Rust-Neuaufbau: öffentliche Patchnotes-Schnittstelle

**Ausgangslage:** Die Community-Website holt sich die übersetzten Deadlock-Patchnotes über eine öffentliche Schnittstelle des Dashboards (ohne Anmeldung). Diese lief bisher über Python.

**Was wurde geändert:** Die Patchnotes-Schnittstelle ist in Rust nachgebaut. Sie liefert alle Patches, für die eine deutsche Übersetzung vorliegt, in der gewohnten Reihenfolge (neueste zuerst) und erkennt je Patch wie bisher, welche Abschnitte enthalten sind (Allgemein, Items, Helden). Die für den Website-Abruf nötigen Zugriffs- und Zwischenspeicher-Hinweise (CORS und Cache) werden mitgeliefert, inklusive der Vorab-Anfrage (OPTIONS), die der Browser vor dem eigentlichen Abruf stellt.

**Wie es jetzt funktioniert:** Die Schnittstelle liest dieselbe Tabelle mit derselben Bedingung und Sortierung wie zuvor und gibt dieselbe Form zurück, damit die Website unverändert weiterläuft. Geprüft gegen eine Kopie der echten Datenbank: alle Patches kommen mit korrekten Feldern und Zugriffs-Hinweisen zurück, die Vorab-Anfrage wird sauber beantwortet. Die zweite öffentliche Schnittstelle — die Live-Mitgliederzahlen des Servers — braucht Daten direkt aus dem laufenden Bot und folgt separat.

## #127 — Rust-Neuaufbau: Heldenkonfiguration speichern (mit Schutz vor Fremd-Absenden)

**Ausgangslage:** Auf der Heldenkonfigurations-Seite lässt sich die globale Ziel-Build-Bezeichnung setzen. Die Anzeige war schon in Rust (#125), das Speichern lief aber noch über Python. Schreibende Aktionen im Admin-Bereich brauchen zudem einen Schutz dagegen, dass eine fremde Webseite im Namen eines angemeldeten Admins heimlich Änderungen auslöst.

**Was wurde geändert:** Das Speichern der globalen Ziel-Build-Bezeichnung ist jetzt in Rust. Dafür wurde der allgemeine Schutz für schreibende Admin-Aktionen mitgebaut: Jede Änderung verlangt ein gültiges Sitzungs-Token *und* ein passendes Anti-Fälschungs-Token (CSRF), das nur die echte Admin-Oberfläche kennt — fehlt es, wird die Aktion abgewiesen. Die Eingabe wird wie bisher geprüft (getrimmt, höchstens 120 Zeichen, keine Steuerzeichen) und jede Änderung im Protokoll vermerkt (wer, von welchem Wert auf welchen).

**Wie es jetzt funktioniert:** Eine Speicher-Anfrage ohne das Anti-Fälschungs-Token wird mit demselben Status abgewiesen wie zuvor; mit gültiger Sitzung und Token wird der Wert normalisiert, gespeichert und zurückgegeben. Geprüft gegen eine Kopie der echten Datenbank: ohne Token abgewiesen, mit Token gespeichert und sofort über die Anzeige bestätigt, falscher Werttyp sauber abgelehnt. Dieser Schutz ist zugleich die Grundlage für die noch ausstehenden Schreib-Bereiche (Turnier-Verwaltung, Bot-Steuerung). Das Bearbeiten einzelner Helden samt Abgleich gegen die externe Build-Quelle folgt weiterhin separat.

## #126 — Rust-Neuaufbau: Austritts-Umfrage-Link (Anzeige)

**Ausgangslage:** Wer den Server verlässt, bekommt per DM einen Link zu einer kurzen Austritts-Umfrage. Beim Öffnen des Links lädt die Seite die zugehörigen Daten (Anzeigename, Nutzergruppe, ggf. vorausgewählter Grund) und zeigt, ob schon abgesendet wurde. Dieser öffentliche Abruf lief bisher über das Python-Dashboard.

**Was wurde geändert:** Der Anzeige-Teil des Umfrage-Links ist in Rust nachgebaut. Der Aufruf braucht keine Anmeldung — er ist allein über das im Link enthaltene Token abgesichert; ungültig aufgebaute oder über 30 Tage alte Token werden wie bisher abgewiesen.

**Wie es jetzt funktioniert:** Das Token wird auf das erlaubte Format geprüft und dann gegen dieselbe Tabelle mit demselben 30-Tage-Fenster nachgeschlagen wie zuvor; die Antwort hat dieselbe Form, damit das bestehende Umfrage-Formular unverändert weiterläuft. Geprüft gegen eine Kopie der echten Datenbank: ein gültiges Token liefert die Daten, ein unbekanntes oder falsch aufgebautes Token wird sauber abgewiesen. Das eigentliche Absenden der Umfrage inklusive Bild-Anhängen folgt separat; bis dahin bleibt dafür das Python-Dashboard zuständig.

## #125 — Rust-Neuaufbau: Dashboard-Heldenkonfiguration (Anzeige)

**Ausgangslage:** Im Admin-Bereich gibt es eine Seite für die Deadlock-Heldenkonfiguration: die global eingestellte Ziel-Build-Bezeichnung und die Liste aller Helden samt ihrer hinterlegten Build-Schnappschüsse. Diese Anzeige lief bisher über das Python-Dashboard.

**Was wurde geändert:** Die Anzeige-Seite ist in Rust nachgebaut — die globale Konfiguration und die vollständige Heldenliste mit ihren Builds. Builds werden je Held gruppiert und in der gewohnten Reihenfolge ausgegeben; hat ein Held keine eigenen Builds, aber einen früher hinterlegten Ursprungs-Build, erscheint wie bisher ein „Legacy"-Vorschlag. Diese Seite ist Vollzugriffs-Mitgliedern vorbehalten — ein neuer Zugriffs-Schutz prüft, dass nur eine angemeldete Sitzung mit vollem Dashboard-Zugriff sie laden kann (sonst Abweisung mit demselben Status wie zuvor).

**Wie es jetzt funktioniert:** Die Abfragen treffen dieselben Tabellen und denselben Konfigurations-Speicher wie das Original, die Antwort hat dieselbe Form, damit die bestehende Oberfläche unverändert weiterläuft. Geprüft ist das gegen eine Kopie der echten Datenbank: ohne gültige Sitzung wird die Seite abgewiesen, mit Vollzugriffs-Sitzung kommen die echten Werte (Konfiguration und alle Helden mit ihren Builds) zurück. Das Bearbeiten und Speichern von Helden samt automatischem Abgleich gegen die externe Build-Quelle ist bewusst noch nicht dabei und folgt separat; bis dahin bleibt dafür das Python-Dashboard zuständig.

## #124 — Rust-Neuaufbau: Dashboard-Auswertungen (Voice, Austritte, Mitspieler-Netz)

**Ausgangslage:** Nach den ersten beiden Auswertungs-Schnittstellen (#123) folgen drei größere: die **Voice-Historie** (Zeiten und Sitzungen über einen Zeitraum, mit Aufschlüsselung nach Stunde/Wochentag/Woche/Monat und einer Detailansicht pro Nutzer), die **Austritts-Umfragen** (Übersicht über Gründe und Antwortquoten) und das **Mitspieler-Netz** (wer mit wem wie oft im Voice war, als Graph). Diese lasen bisher aus dem Python-Dashboard.

**Was wurde geändert:** Alle drei sind jetzt in Rust nachgebaut. Die Voice-Historie liefert die Tages-Summen, die aktivsten Nutzer im Zeitraum und die Verteilung über die gewählte Zeiteinheit — bei Stunden immer volle 24 Balken, bei Wochentagen die sieben deutschen Tagesnamen, sonst die tatsächlichen Wochen/Monate. Wird ein Nutzer angegeben, kommt seine Detailansicht dazu: Zeitraum- und Lebenszeit-Statistik plus die letzten Sitzungen mit den jeweiligen Mitspielern. Die Austritts-Übersicht zählt Einreichungen nach Nutzergruppe, Zustellstatus und Grund, berechnet die Antwortquoten (per DM und über die Web-Umfrage) und listet die neuesten Rückmeldungen. Das Mitspieler-Netz baut aus den gespeicherten Paar-Daten einen Graphen: Personen als Knoten (mit Gewicht und Anzahl Verbindungen), die gemeinsamen Voice-Zeiten als Kanten, wobei Hin- und Rückrichtung zu einer Kante zusammengefasst werden.

**Wie es jetzt funktioniert:** Jede Abfrage trifft dieselben Tabellen mit denselben Bedingungen, Sortierungen, Grenzwerten und Zeitfenstern wie zuvor; Durchschnitts- und Quoten-Werte werden wie im Original ohne zusätzliches Runden ausgegeben, damit nichts an der Nachkommastelle abweicht. Namen zu Discord-IDs kommen bevorzugt aus den bereits in der Datenbank hinterlegten Anzeigenamen, sonst über den Bot; fehlt beides, steht ersatzweise „User <ID>" da — exakt wie bisher. Geprüft ist alles gegen eine Kopie der echten Datenbank (Bucket-Auffüllung, Wochentags-Beschriftung, die Detailansicht mit allen Feldern, die Graph-Kennzahlen, gleiche Fehlertexte). Es fehlen aus dieser Auswertungs-Gruppe noch die laufenden Voice-Sitzungen in Echtzeit, die Verbleibs-Analyse und die Server-Gesamtstatistik — diese brauchen Daten direkt aus dem laufenden Bot und folgen separat.

## #123 — Rust-Neuaufbau: Dashboard-Auswertungen (erste Reads)

**Ausgangslage:** Das Admin-Dashboard zeigt mehrere Auswertungen — wer ist beigetreten/gegangen, wer schreibt wie viel, Voice-Zeiten, Mitspieler-Netz und so weiter. Diese Seiten lasen bisher ausschließlich aus dem Python-Dashboard. Nachdem der Anmelde-Teil in Rust steht (#122), kommt jetzt die Auswertungs-Seite dran — Stück für Stück, weil einige der Abfragen groß sind.

**Was wurde geändert:** Zwei der Auswertungs-Schnittstellen sind in Rust nachgebaut: die **Mitglieder-Ereignisse** (Beitritte/Abgänge/Bans mit Filtern nach Typ und Server, Häufigkeiten je Typ und die Zahl der Beitritte/Abgänge der letzten 7 Tage) und die **Nachrichten-Aktivität** (die aktivsten Schreiber mit Anzahl und Zeitpunkten plus eine Zusammenfassung mit Schnitt pro Nutzer). Dazu kam die nötige Grundlage: ein Schutz, der lesende Auswertungs-Aufrufe nur mit gültiger Anmeldung durchlässt, und eine Namensauflösung, die zu mehreren Discord-IDs auf einen Schlag die Anzeigenamen besorgt — letzteres über den Bot, weil der Rust-Web-Prozess selbst keine Discord-Verbindung hat (neuer rein lesender Sammel-Endpunkt am Bot-Broker).

**Wie es jetzt funktioniert:** Beide Abfragen treffen genau dieselben Tabellen mit denselben Bedingungen, Sortierungen und Grenzwerten wie das Original; die Antworten haben dieselbe Form, damit die bestehende Oberfläche unverändert weiterläuft. Eingabe-Fehler (z. B. eine unsinnige Anzahl-Angabe) werden mit demselben Text und Statuscode abgewiesen wie zuvor. Der Schnitt pro Nutzer wird mit derselben kaufmännischen Rundung berechnet wie in Python (sonst kippen Werte an der Rundungsgrenze). Geprüft ist das gegen eine Kopie der echten Datenbank — gleiche Daten, gleiche Form, gleiche Fehlertexte. Die restlichen Auswertungen (Voice-Statistik und -Verlauf, Verbleib, Austritts-Umfragen, Mitspieler-Netz, Server-Statistik) folgen als nächstes; bis dahin bleibt dafür das Python-Dashboard zuständig.

## #122 — Rust-Neuaufbau: der Dashboard-Login (Auth-Provider)

**Ausgangslage:** Das Master-Dashboard (Port 8766) ist im Hintergrund der Anmelde-Besitzer: Wenn sich jemand auf der Statistik-, Tierlist- oder Turnier-Seite per Discord einloggt, reichen diese Seiten die Anmeldung an das Dashboard durch — es spricht mit Discord, prüft die Rechte und gibt die Identität zurück. Dieser Teil lag bisher nur als Python-Dienst vor (rund 7600 Zeilen) und war das letzte große, noch nicht nach Rust portierte Stück. Solange er fehlt, kann auch keine der schon umgebauten Seiten endgültig auf Rust umgestellt werden.

**Was wurde geändert:** Der Anmelde- und Weiterreich-Teil ist jetzt in Rust nachgebaut — bewusst nicht als ein Riesen-Block, sondern als kleine, einzeln getestete Bausteine: der Discord-OAuth-Ablauf (Weiterleitung zu Discord, Code-gegen-Token-Tausch, Profil-Abruf), der genau einmal einlösbare Anmelde-Vorgang in der Datenbank, der Sitzungs-Speicher mit gleitender Gültigkeit, die Rechteprüfung und die Schutzregeln. Weil der Rust-Web-Prozess selbst keine Discord-Verbindung hat, fragt er Rollen und Admin-Status über den Bot (Master-Broker) ab — dafür hat der Broker einen neuen, rein lesenden Endpunkt bekommen.

**Wie es jetzt funktioniert:** Zwei Wege laufen darüber. (1) **Eigener Admin-Login:** „Login" erzeugt einen kurzlebigen Vorgang und schickt den Browser zu Discord; nach der Bestätigung kommt der Browser mit einem Einmal-Code zurück, der Code wird gegen ein Discord-Token getauscht, das Profil geladen und der Zugriff entschieden — Server-Owner, Administrator-Recht oder Moderator-Rolle bekommen vollen Zugriff, die Community-Moderator-Rolle nur den Turnier-Bereich, alle anderen werden abgewiesen. Bei Erfolg gibt es ein Sitzungs-Cookie (HttpOnly, gleitende Gültigkeit), das bei jedem Aufruf verlängert wird. (2) **Delegierte Anmeldung für andere Seiten:** Eine Seite startet den Vorgang (`initiate`) und bekommt eine Discord-Login-URL plus eine Vorgangs-ID; nach der Rückkehr von Discord legt das Dashboard das Ergebnis an der Vorgangs-ID ab, die Seite holt es mit `consume-result` ab und erhält Discord-ID, Name und Rollen — die Vorgangs-ID ist danach verbraucht. Schutz: Die internen Schnittstellen sind nur lokal und nur mit gültigem Token erreichbar (getrennte Token für Turnier- und Twitch-Aufrufe), und Weiterleitungsziele werden gegen eine feste Allowlist geprüft, damit niemand über einen manipulierten Link auf eine fremde Seite umgeleitet werden kann. Bewiesen ist das mit 35 Einzeltests plus einem Ende-zu-Ende-Durchlauf gegen den laufenden Prozess (Token-Trennung, Allowlist, Vorgangs-Persistenz, Sitzungs-Roundtrip). Noch offen im Dashboard-Umbau und absichtlich getrennt: die Auswertungs-Seiten (Voice-/Mitglieder-/Aktivitäts-Statistik), die Turnier-Verwaltung, die Deadlock-Konfiguration und die Steuerungs-Seite (die im Rust-Modell auf systemd-Neustarts statt Cog-Nachladen umgebaut wird). Bis dahin bleibt das Python-Dashboard der aktive Dienst — der Rust-Teil läuft parallel und wird erst beim gemeinsamen Umstieg scharf geschaltet.

## #121 — Steam-Bridge: Discord-Anzeigename an den Steam-Bot mitsenden

**Ausgangslage:** Wenn jemand im Steam-Einladungs-Funnel auf einen Button klickt, reicht die Bridge die Interaktion an den (Rust-)Steam-Bot weiter. Der Anzeigename des Nutzers wurde dabei nicht mitgeschickt — der Steam-Bot kannte nur die numerische Discord-ID. An Stellen, die den Namen erwarten (z.B. die Supporter-Bestätigung nach einer Ko-fi-Spende), erschien dadurch die nackte ID statt des Namens.

**Was wurde geändert:** Die Bridge legt den Discord-Anzeigenamen (`display_name`) jetzt mit in die weitergeleitete Interaktion (Feld `data.discord_name`), das der Steam-Bot ohnehin ausliest.

**Wie es jetzt funktioniert:** Der Steam-Bot nutzt den echten Anzeigenamen, wo er ihn braucht; fehlt er ausnahmsweise, fällt er weiterhin sauber auf die ID zurück. Rein interne Kopplung zwischen Bridge und Steam-Bot — für Nutzer ändert sich nichts Sichtbares außer korrekten Namen in Bestätigungen.

## #120 — DNS-Resolver: aiodns ergänzt, Warn-Flut im Log beseitigt

**Ausgangslage:** Beim Aufbau jeder HTTP-Verbindung versucht der Bot, einen asynchronen DNS-Resolver mit festen öffentlichen Nameservern (1.1.1.1 / 8.8.8.8 / 9.9.9.9) zu verwenden. Dieser Resolver braucht die Bibliothek `aiodns`, die aber nie in der Umgebung installiert war. Folge: Der Versuch schlug jedes Mal fehl, der Code fiel still auf den Standard-Resolver zurück — und schrieb dabei jedes Mal eine Warnung ins Log. Das passierte rund einmal pro Minute, also etwa 1.600 identische Warnzeilen in 24 Stunden, die echte Fehler im Log zugemüllt haben.

**Was wurde geändert:** `aiodns` (4.0.4) wurde als Abhängigkeit ergänzt und in die Laufzeitumgebung installiert. Am Code selbst nichts geändert — der asynchrone Resolver war bereits vorgesehen, ihm fehlte nur die Bibliothek.

**Wie es jetzt funktioniert:** Der asynchrone Resolver wird jetzt erfolgreich aufgebaut und tatsächlich genutzt; der Fallback-Pfad mit der Warnung läuft nicht mehr an. DNS-Auflösungen laufen damit nicht-blockierend über die festen Nameserver statt über den thread-basierten Standard-Resolver. Verifiziert: nach dem Neustart über 90 Sekunden keine einzige der Warnungen mehr, Bot sauber hochgefahren.

## #119 — Broker: GET /discord/members für Rust-Streamer-Link-Matcher

**Ausgangslage:** Der Streamer-Link-Matcher soll neu in Rust (tb-bot) laufen statt als Python-Cog im Discord-Bot. Dafür braucht der Rust-Code Zugriff auf alle Guild-Member mit Name, globalem Anzeigenamen und Server-Nickname — bisher gab der Broker nur Role-Member-Listen (mit Role-Filter) oder einzelne User-Auflösungen zurück.

**Was wurde geändert:** Neuer read-only Endpoint `GET /internal/master/v1/discord/members` im Master-Broker — liefert alle Nicht-Bot-Member der primären Guild als JSON-Array mit `id`, `name`, `global_name`, `nick`. Loopback-only wie die anderen Diagnose-Routen, kein Token erforderlich. Trigger `guild.chunk()` wenn der Member-Cache noch nicht vollständig befüllt ist.

**Wie es jetzt funktioniert:** Der Rust tb-bot ruft beim Streamer-Abgleich einmalig alle Guild-Member ab und baut daraus intern einen Matching-Index. Die Python-Seite ist damit auf das Bereitstellen der Member-Daten beschränkt — die eigentliche Abgleich-Logik läuft komplett in Rust.

## #118 — Master-Broker: Discord-User-Auflösung für den Twitch-Bot

**Ausgangslage:** Der Twitch-Bot (Rust) übernimmt nach einer Streamer-Autorisierung das Partner-Setup — dazu gehört, den Discord-Anzeigenamen des Streamers nachzuschlagen und ihm die Streamer-Rolle zu geben. Rollen vergeben konnte der Broker schon, einen Discord-User nur anhand seiner ID auflösen (Name/Anzeigename) aber nicht.

**Was wurde geändert:** Neue interne Broker-Route `resolve-user`: nimmt eine Discord-User-ID entgegen und liefert Name, globalen Anzeigenamen und Server-Anzeigenamen zurück. Nicht auffindbare User kommen als „nicht gefunden" zurück statt als Fehler — der Aufrufer behandelt das wie bisher der Python-Pfad (kein Name verfügbar, weiter ohne).

**Wie es jetzt funktioniert:** Streamer autorisiert den Twitch-Bot → der Rust-Prozess fragt den Broker nach dem Anzeigenamen (Cache zuerst, dann Discord-API) und lässt ihn die Streamer-Rolle setzen. Nur intern erreichbar (Loopback + Token), reine Lese-Operation ohne Seiteneffekte.

## #117 — Streamer-Abgleich: neue Namen sichtbar + manueller Discord-Link

**Ausgangslage:** Das Summary-Embed nach einem Abgleich zeigte nur Zahlen — welche Streamer konkret neu dazugekommen sind, war nicht erkennbar. Wer keinen automatischen Discord-Match bekam, verschwand kommentarlos im "Ohne Treffer"-Topf; ein nachträgliches manuelles Verknüpfen war nur über DB-Direktzugriff möglich.

**Was wurde geändert:** Im Summary-Embed erscheint jetzt eine "Neu:"-Zeile mit den Twitch-Logins aller in diesem Lauf erstmals geprüften Streamer (bis zu 10, danach "+X weitere"). Für jeden Streamer ohne Discord-Treffer — egal ob kein Member gefunden oder Score zu niedrig — wird zusätzlich ein eigenes Embed mit einem "Discord eingeben"-Button gepostet. Der Button öffnet ein Modal, in das Name oder numerische ID eingetippt werden kann; der Bot sucht erst nach exaktem Username/Displayname/Nick, fällt bei Nichttreffer auf ID-Suche zurück. Nach erfolgreichem Eintragen wird die Streamer-Rolle vergeben, der State als "linked" markiert und der Button dauerhaft deaktiviert. Persistent Views werden nach Bot-Neustart wiederhergestellt, sodass offene Eingabe-Prompts nicht verloren gehen.

**Wie es jetzt funktioniert:** Neuer Streamer taucht auf → Abgleich läuft → Summary zeigt Namen → wenn kein Match: orangefarbenes Embed mit Name und Button erscheint → Mod klickt, tippt Discord-Name oder ID ein → Bot verknüpft und vergibt Rolle → Embed wird grün und Button verschwindet.

## #116 — Steam-Panels: Restart-fest, editierbar, Zielkanal wählbar

**Ausgangslage:** Beim Umbau auf den Rust-Steam-Bot war die Discord-Brücke schlanker geraten als das Original: Das Steam-Verknüpfen-Panel wurde nach einem Bot-Neustart nicht mehr aufgefrischt, `/publish_steam_panel` konnte keine bestehende Panel-Nachricht editieren (jeder Aufruf erzeugte ein neues Panel), und `/publish_betainvite_panel` konnte nur in den aktuellen Kanal posten.

**Was wurde geändert:**

- **Panel-Restore nach Neustart:** Beim Posten merkt sich der Bot jetzt Kanal- und Nachrichten-ID des Steam-Panels in der Datenbank (gleicher Speicherplatz wie früher — das alte Panel aus der Python-Zeit wurde beim ersten Start direkt übernommen und aufgefrischt). Nach jedem Neustart holt er das aktuelle Embed vom Steam-Bot und aktualisiert die bestehende Nachricht, statt sie veralten zu lassen.
- **`/publish_steam_panel` kann editieren:** Optionaler `message_id`-Parameter zum gezielten Aktualisieren einer bestehenden Nachricht. Ohne Parameter wird zuerst geprüft, ob im selben Kanal schon ein gespeichertes Panel liegt — dann wird editiert statt doppelt gepostet.
- **`/publish_betainvite_panel` mit Zielkanal:** Optionaler `channel`-Parameter postet das Invite-Panel direkt in den gewählten Kanal oder Thread (wie das Python-Original), Bestätigung kommt ephemer.
- **`!steam_status` antwortet wieder im Kanal:** Der Admin-Befehl wartet jetzt lang genug auf die Antwort des Steam-Bots (synchroner Pfad) statt das Ergebnis per DM nachzureichen.

**Wie es jetzt funktioniert:** Panel posten → Referenz wird gespeichert → jeder Neustart frischt die Nachricht automatisch auf; die Buttons bleiben durchgehend klickbar. Panels lassen sich per Parameter editieren oder gezielt in andere Kanäle setzen, ohne Duplikate zu erzeugen.

## #115 — Discord Go-Live-Sync repariert

**Ausgangslage:** Wenn ein Partner-Streamer live ging, versuchte der Rust-Monitoring-Bot ein Discord-Embed über den Master-Broker zu posten — das schlug seit dem Rust-Cutover stumm fehl. Im Log stand `error decoding response body`, die Discord-Ankündigungen blieben einfach aus.

**Ursache:** Der Broker antwortete auf `POST /internal/master/v1/discord/send-rich-message` mit einem JSON-Body, in dem `message_id` eine Ganzzahl war (z. B. `1381674235709513729`). Der Rust-Client erwartet an dieser Stelle einen String. Serde bricht bei diesem Typ-Mismatch die Deserialisierung ab — kein Crash, nur ein stilles WARN und kein Posting.

**Geändert:** Im Master-Broker wird `message_id` in den Erfolgs-Antworten beider Discord-Endpunkte (send + edit) jetzt als String zurückgegeben. Discord-Snowflakes sind als JSON-Zahl ohnehin problematisch (sie überschreiten JavaScripts `Number.MAX_SAFE_INTEGER`), String ist hier das richtige Format.

**Jetzt:** Go-Live-Postings landen wieder zuverlässig in Discord, sobald ein Streamer auf Sendung geht.

## #114 — Rust-Neuaufbau: die Turnier-Anmeldung

**Ausgangslage:** Das Turnier-Panel mit den drei Knöpfen (Anmelden, Abmelden, Mein Status): Anmelden prüft die Turnier-Rolle, den offenen Anmeldezeitraum und die Steam-Verknüpfung, zeigt den verifizierten Rang und bietet die Wahl zwischen Solo- und Team-Anmeldung — mit Team-Auswahlmenü (volle Teams markiert) oder Team-Neuerstellung per Formular.

**Geändert:** Der komplette Spieler-Flow ist in Rust portiert — gleiche Knopf-Kennungen am Panel (das bestehende Panel funktioniert weiter), alle Texte und Prüfungen identisch, gleiche Datenbank über den bereits portierten Turnier-Speicher. Die Auswahlmenüs sind dabei robuster geworden: Sie überleben jetzt Bot-Neustarts (im Original verfielen sie nach 2 Minuten). Der Admin-Bereich (Zeiträume, Team-Verwaltung, Panel-Posten) bleibt als dokumentierte Lücke in Python — gleiche Datenbasis, kein Konflikt.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Zeitraum-Prüfung (offen/zu/abgelaufen/fehlend), Rang-Anzeige, Knopf-Kennungen und die Erfolgs-Embeds sind getestet; Anmeldungen laufen über denselben getesteten Speicher wie das Turnier-Web.

## #113 — Rust-Neuaufbau: das Coaching-System

**Ausgangslage:** Das Herzstück des Coaching-Angebots: Über das Panel füllt man ein 5-Felder-Formular aus (Rang, Held, Verfügbarkeit, Spielzeit, Probleme), eine AI fasst die Anfrage für die Coaches zusammen (und filtert Unsinn-Anfragen komplett aus), und der Post im Coaching-Kanal wird fair rotierend für 24 Stunden einem Coach reserviert — wer am längsten keine Zuweisung hatte, ist dran. Der Coach übernimmt per Knopf: Session wird angelegt, der Spieler bekommt die Coaching-Rolle und eine DM. Bricht der Coach ab, weil sich der Spieler nicht meldet, gibt es eine 7-Tage-Sperre.

**Geändert:** Der komplette Kern ist in Rust portiert: gleiche Knopf-Kennungen (laufende Anfragen überleben den Umstieg), Formular und alle Antwort-Texte wortgleich, gleiche faire Rotations-Formel, gleiche Reservierungs- und Sperr-Regeln, gleiche Tabellen, AI-Zusammenfassung über die MiniMax-Anbindung mit demselben Unsinns-Filter. Drei ehrlich dokumentierte Lücken laufen vorerst in Python weiter: die Spiegelung zur Coaching-Website, der automatische Rollen-Ablauf nach 48 Stunden und die Feedback-Umfrage nach der Session.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Rotations-Formel (drei Fälle inklusive Gleichstand), Text-Kappungen, das Anfrage-Embed mit Reservierungs-Anzeige und alle Knopf-Kennungen sind getestet; die Analyse- und Ablauf-Schleifen folgen dem Original-Takt.

## #112 — Rust-Neuaufbau: der Onboarding-Wizard

**Ausgangslage:** „Hier starten ➜" im Regelkanal öffnet einen privaten Thread und führt neue Mitglieder durch zehn Schritte: Willkommen, Regeln, (für Streamer ein Extra-Schritt), Voice-Lanes, Mitspieler-Suche, Server-Features, die optionalen Ton- und Alters-Tags und zuletzt die Steam-Verknüpfung. Im Rust-System antwortete der Knopf bisher mit einem Umbau-Hinweis.

**Geändert:** Der komplette Wizard ist portiert. Die Schritt-Texte wurden maschinell aus dem Original extrahiert und byte-genau eingebettet — kein Abtipp-Risiko. Gleiche Logik: Streamer-Schritt nur mit Creator-Rolle (Fußzeile zählt dann bis 10 statt 9), Ton/Alter speichern direkt ins Tag-System, der Steam-Schritt erzeugt den Einmal-Link erst beim Klick. Drei ehrliche Verbesserungen bzw. Annäherungen: Die Wizard-Knöpfe überleben jetzt Bot-Neustarts (das Original vergaß laufende Wizards nach einer Stunde), statt des automatischen Verifikations-Beobachters gibt es einen „Ich hab verknüpft ➜"-Knopf mit Live-Prüfung, und das gesonderte Streamer-Einrichtungs-Formular ist noch nicht portiert (der Schritt informiert und verweist weiter).

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Schritt-Navigation (inklusive Streamer-Überspringen), Fußzeilen-Zählung und die Knopf-Beschriftungen aller Sonder-Schritte sind getestet; die zehn Texte stecken als geprüfte Daten-Datei im Modul.

## #111 — Rust-Neuaufbau: der Nachrichten-Zähler

**Ausgangslage:** Der alte Aktivitäts-Analyzer zählt jede Server-Nachricht pro Nutzer mit (Gesamtzahl, erste und letzte Nachricht). Auf diesem Zähler bauen mehrere Funktionen auf — unter anderem die Einstufung der Abschieds-Umfrage (wer „aktiv" war, bekommt andere Fragen). Nach dem Umstieg hätte niemand mehr gezählt, und die Einstufung wäre schleichend falsch geworden.

**Geändert:** Der Rust-Bot führt den Zähler jetzt selbst weiter — gleiche Tabelle, gleiche Zähl-Logik (Hochzählen je Nutzer und Server, Kanal und Zeitstempel aktualisieren), gleicher Datenschutz (Opt-out-Nutzer werden nicht erfasst). Damit ist die zweite stille Daten-Abhängigkeit des Teil-Umstiegs geschlossen.

**Wie es jetzt funktioniert:** Unverändert sichtbar — aber die Daten bleiben nach dem Umstieg korrekt.

## #110 — Rust-Neuaufbau: die Gruppensuche ist durchgängig

**Ausgangslage:** Nach Entscheidungslogik (#103) und Antwort-Bau (#109) fehlte das letzte Anschluss-Stück der Gruppensuche: Nachrichten aus dem Suche-Kanal einsammeln, den Rang bestimmen und das fertige Lobby-Finder-Embed posten.

**Geändert:** Das Anschluss-Stück ist gebaut: Suche-Nachricht erkannt (nur außerhalb einer Lane, 60-Sekunden-Bremse) → Rang aus den Rollen, sonst aus dem Nachrichtentext geparst („Oracle 3", „emi II" — Kurzformen und römische Unterränge wie im Original) → Anfänger ohne Rang werden wie ein Alchemist 1 eingestuft → Lanes aller vier Kategorien aus dem Discord-Cache gescannt (Rang-Durchschnitt aus Rollen, Sonderfall Juice-Kammer zählt fix als Eternus, bekannte Mitspieler markiert) → Embed in den Ausgabe-Kanal. Damit ist die komplette Gruppensuche in Rust durchgängig: Erkennung → Einstufung → Routing → Auswahl → Antwort. Ehrlich dokumentierte Rest-Lücke: Das Admin-Entscheidungsprotokoll wird ins Server-Log geschrieben statt als eigenes Embed gepostet.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Der Text-Rang-Parser ist mit fünf Fällen getestet (inklusive „höchster Rang gewinnt"); insgesamt sichern 14 Tests die LFG-Kette ab.

## #109 — Rust-Neuaufbau: die LFG-Antwort komplett

**Ausgangslage:** Nach der Routing-Entscheidung (#103) fehlte noch die sichtbare Antwort der Gruppensuche: das Lobby-Finder-Embed mit bis zu drei Vorschlägen (Punktesystem: bekannte Mitspieler, Routing-Treffer, Belegung, Rang-Nähe; eng passende Lanes gewinnen allein), die sechs Begrüßungs-Varianten (Anfänger mit Coaching-Tipp, gefundene Lobbys, leere Lage), die Feld-Texte (Belegung, Durchschnittsrang, „etwas über deinem Rang"-Warnung, anwesende Bekannte, Voll-Hinweis) und der „eigene Lobby aufmachen"-Verweis auf den passenden Sammel-Kanal.

**Geändert:** Diese komplette Antwort-Schicht ist als reine, testbare Logik in Rust portiert — Texte wortgleich, Bewertungs-Formel zahlengleich, Auswahl-Regeln identisch (inklusive der Feinheit, dass mit bekanntem Rang genau EIN eng passender Vorschlag gezeigt wird statt dreien). Auch die Anfänger-Erkennung aus dem Nachrichtentext („bin neu", „Anfänger" …) ist dabei.

**Wie es jetzt funktioniert:** Die LFG-Kette ist in Rust jetzt von der Erkennung über die Entscheidung bis zur fertigen Antwort durchgängig vorhanden und getestet; es fehlt nur noch das dünne Anschluss-Stück, das Nachrichten aus dem Suche-Kanal einsammelt und das Embed postet.

## #108 — Rust-Neuaufbau: der FAQ-Chat und der Ticket-Auto-Helfer

**Ausgangslage:** Über das FAQ-Panel bekommt jeder auf Knopfdruck einen privaten Chat-Kanal, in dem ein Bot Fragen zum Server beantwortet — mit der Server-Dokumentation als Wissensbasis, strikten Sicherheitsregeln (keine internen Details, nichts erfinden) und Gesprächs-Gedächtnis für Rückfragen. Chats schließen nach 24 Stunden oder per Knopf. Zusätzlich liest der Ticket-Auto-Helfer die erste Nachricht in neuen Support-Tickets mit: Kann er das Anliegen aus der Doku klar lösen, antwortet er sofort — sonst schweigt er und überlässt das Ticket den Menschen.

**Geändert:** Komplett in Rust portiert: gleiche Knopf-Kennungen, gleiche System-Anweisungen wortgleich (inklusive der Invite- und Coaching-Sonderregeln), gleiche Tabellen, gleiches Gedächtnis-Fenster (letzte 10 Nachrichten), gleiche Schweige-Regel (KEIN_TREFFER-Protokoll). Die Antworten laufen über die bestehende MiniMax-Anbindung. Zwei ehrlich dokumentierte Annäherungen: Der Ticket-Helfer springt auf die erste Nachricht im Ticket-Kanal an statt auf dessen Erstellung (sichtbar gleiches Verhalten), und die optionale Patchnotes-Anreicherung der Antworten fehlt noch.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Prompt-Aufbau, Doku-Sammlung (alphabetisch, nur Markdown) und der komplette Session-Lebenszyklus (anlegen, Verlauf kappen, schließen, Ablauf) sind mit Tests abgesichert.

## #107 — Rust-Neuaufbau: die Clip-Einsendungen

**Ausgangslage:** Im Clip-Kanal steht ein Einsende-Interface: Button drücken, Verwendungserlaubnis bestätigen, dann Link, Credit und Kontext ins Formular — mit Link-Prüfung und 60-Sekunden-Bremse gegen Spam. Die Einsendungen sammeln sich in einem Wochenfenster (Sonntag 0 Uhr bis Samstag 23 Uhr deutscher Zeit); nach Ablauf bekommt der Clip-Kurator genau einmal eine Text-Datei mit allen Einsendungen der Woche per DM.

**Geändert:** Komplett in Rust portiert: gleiche Knopf-Kennungen (das bestehende Interface im Kanal funktioniert nach dem Umstieg weiter), gleiche Formular-Felder und Fehlertexte, gleiche Tabellen, gleiche Fenster-Berechnung inklusive Zeitzonen-Logik, gleiches Dump-Format. Das Interface aktualisiert sich weiter alle 5 Minuten (Fenster-Countdown im Embed), der Fenster-Wächter prüft alle 2 Minuten auf abgelaufene Fenster.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Die Fenster-Berechnung ist gegen konkrete Kalenderdaten getestet (inklusive „Sonntag selbst ist Fensterstart"), Dump-Format und URL-Prüfung ebenso, und ein Datenbank-Test spielt den kompletten Lebenszyklus durch: Fenster anlegen → Einsendung zuordnen → Dump markieren.

## #106 — Rust-Neuaufbau: die Exit-Umfrage

**Ausgangslage:** Wer den Server verlässt, bekommt eine kurze Abschieds-Umfrage per DM — mit Fragen, die zum Nutzer-Typ passen: Kurzbesucher (unter 2 Tagen, nie im Voice, kaum Nachrichten) werden anders gefragt als langjährig Aktive (ab 14 Tagen mit regelmäßigen Sessions, vielen Nachrichten oder über einer Stunde Voice) oder stille Mitglieder. Gebannte und Nutzer mit Datenschutz-Opt-out werden übersprungen, und niemand wird öfter als alle 30 Tage befragt.

**Geändert:** Komplett in Rust portiert: gleiche Typ-Einstufung, gleiche Grund-Auswahllisten und Nachfragen (wortgleich), gleiche Tabelle samt Einmal-Token für den Web-Fragebogen. Die Auswahl-Kennungen der Knöpfe sind unverändert — auch Umfrage-DMs, die noch vom alten Bot verschickt wurden, funktionieren nach dem Umstieg weiter, weil die Antwort über die Datenbank ihrer offenen Umfrage zugeordnet wird.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Verlassen → Sperren-Checks → Einstufung → DM mit Grund-Auswahl → Nachfrage-Formular → Antwort und Zeitpunkt landen in der Datenbank, eine Zusammenfassung im Log-Kanal. Die Einstufungs-Regeln und alle Texte sind mit Tests abgesichert (8 Klassifikations-Fälle).

## #105 — Rust-Neuaufbau: Beitritts-Protokoll und eine wichtige Umstiegs-Korrektur

**Ausgangslage:** Beim geplanten Teil-Umstieg wäre ein stilles Problem entstanden: Der alte Aktivitäts-Analyzer und sein neues Rust-Pendant hätten beide den Mitspieler-Zähler hochgezählt — alle Werte doppelt. Außerdem schreibt bisher nur der alte Analyzer die Beitritts-/Austritts-Ereignisse, auf denen die Join-Quellen-Auswertung und die Bindungs-Funktionen aufbauen.

**Geändert:** Die Umstiegs-Checkliste warnt jetzt ausdrücklich, dass der alte Analyzer beim Umstieg abgeschaltet werden muss, und der Rust-Bot schreibt die Beitritts-/Austritts-Ereignisse selbst weiter (gleiche Tabelle). Ehrlich dokumentierte Übergangs-Lücke: Die Herkunfts-Zuordnung neuer Beitritte (welcher Einladungs-Link) fehlt noch — solche Beitritte zählen als „Unbekannt", bis das Einladungs-Abgleich-Verfahren portiert ist. Die Checkliste wurde außerdem auf den aktuellen Stand gebracht: Alle drei ursprünglichen Umstiegs-Voraussetzungen sind gebaut, es gibt keine Blocker mehr.

**Wie es jetzt funktioniert:** Nach dem Umstieg gehen keine Beitritts-Daten verloren und nichts wird doppelt gezählt.

## #104 — Rust-Neuaufbau: die Website-Invites

**Ausgangslage:** Jede Website-Unterseite (Landing, Streamer, Mitspieler, Coaching, Helden, Guides) hat ihren eigenen permanenten Einladungs-Link in den Server — so lässt sich nachvollziehen, über welche Seite neue Mitglieder kommen. Der Bot prüft beim Start, ob die gespeicherten Codes noch existieren, und erstellt fehlende neu. Dazu gehört die Join-Quellen-Auswertung, die Beitritte nach Herkunft aufschlüsselt (Website-Seite, Vanity-Link, Twitch-Streamer, persönliche Einladungen …).

**Geändert:** Beides in Rust portiert: gleiche Speicher-Verträge (inklusive der Landing/Haupt-Spiegelung), gleicher Start-Lebenszyklus (tote Codes werden ersetzt, lebende bleiben unangetastet), und die komplette Herkunfts-Klassifikation samt Balken-Anzeige wortgleich. Die Auswertung liest dieselben Beitritts-Ereignisse, die der bestehende Python-Analyzer weiter schreibt.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Acht Klassifikations-Fälle und der Speicher-Vertrag (inklusive Spiegelung) sind getestet. Der Start-Check wartet 20 Sekunden nach dem Hochfahren, damit die Discord-Verbindung steht.

## #103 — Rust-Neuaufbau: das LFG-Routing-Herz

**Ausgangslage:** Wenn die Gruppensuche eine Mitspieler-Anfrage erkennt, entscheidet eine Routing-Logik, wohin der Suchende gelotst wird: Street-Brawl- und Ranked-Absicht aus Schlüsselwörtern (Ranked zählt erst ab Emissary), Anfänger primär in die Neue-Spieler-Lanes mit Casual-Rückfall, Rang-Toleranzen je Lane-Typ (±2 Ranked, ±3 Casual), und Lanes mit bekannten Mitspielern gewinnen vor der vollsten passenden.

**Geändert:** Diese Entscheidungslogik ist vollständig in Rust portiert und mit sieben Referenz-Entscheidungen aus dem laufenden Python-Original abgesichert — jedes Szenario (Casual, Ranked, zu niedriger Rang für Ranked, Street Brawl, Anfänger-Rückfall, Mitspieler-Vorrang, alles voll) liefert exakt dieselbe Entscheidung. Der sichtbare Antwort-Teil (Embed mit Empfehlung und Spielervorschlägen im Ausgabe-Kanal) folgt als letzter Schritt auf dieser Basis.

**Wie es jetzt funktioniert:** Noch unverändert über den Python-Bot — die Rust-Seite hält jetzt aber die komplette Entscheidungskette (Erkennung → Bewertung → Routing) getestet vor.

## #102 — Rust-Neuaufbau: der Modus-Wechsel — das Lane-Panel ist komplett

**Ausgangslage:** Der letzte gesperrte Panel-Knopf: Lane-Besitzer können ihre bestehende Lane in einen anderen Modus umziehen (Ranked, Casual, Street Brawl, Off Topic) — die Lane wandert in die Ziel-Kategorie, der Datenbank-Eintrag zieht nach, und der Name passt sich an (Ranked übernimmt den Rang des Besitzers, sonst kehrt der gespeicherte Basisname zurück).

**Geändert:** In Rust angeschlossen mit Auswahlmenü, demselben Ranked-Gate (verifizierter Rang nötig, sonst Hinweis auf den Info-Kanal) und derselben Umzugs-Reihenfolge. Damit sind **alle** Funktionen des Lane-Panels im neuen System verfügbar.

**Wie es jetzt funktioniert:** Knopf → Modus wählen → Lane zieht um. Der Voice-Bereich ist damit funktional vollständig portiert.

## #101 — Rust-Neuaufbau: Anfänger-Lanes, Off-Topic-Duo und die Rang-Sortierung

**Ausgangslage:** Drei Spezial-Systeme rund um die Voice-Lanes fehlten noch im Rust-Port: Die Anfänger-Einsortierung (wer höchstens Arcanist ist — verifiziert oder mit Unverifiziert-Rolle — wird beim Betreten der Sammel-Kanäle in die Neue-Spieler-Kategorie umgeleitet, mit 4-Minuten-Rückkehr-Fenster in den normalen Ablauf), die selbst-wachsenden Lanes („Neue Spieler Lane" legt ab 6 Personen nach, „Off Topic Voice" ab 2 genau eine zweite; Leere werden abgebaut und Nummern rücken nach) und die Rang-Sortierung der Chill-Lanes (Lanes mit Rang-Namen werden nach Haupt- und Unterrang auf ihre Plätze sortiert).

**Geändert:** Alle drei in Rust portiert und an den zentralen Ereignis-Verteiler gehängt: Die Anfänger-Umleitung sitzt als Vorab-Haken im Join-to-create (genau wie das Original — greift kein Haken, läuft alles normal weiter), die Wachstums-Pläne beider Lane-Familien teilen sich eine Logik mit zwei Spielarten und sind mit Referenz-Plänen aus dem Python-Original abgesichert (inklusive der Feinheit, dass besetzte Lanes vor leeren nachrücken), und die Sortierung verschiebt nur, was tatsächlich falsch steht.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Fünf Test-Gruppen decken die Pläne, die Namens-/Index-Erkennung, die Anfänger-Rang-Auflösung und die Sortier-Reihenfolge ab — jeweils gegen Referenz-Ausgaben des Originals. Damit ist der Voice-Bereich funktional vollständig portiert; im Lane-Panel fehlt nur noch der Modus-Wechsel-Knopf.

## #100 — Steam-Panel-Buttons repariert + Scam-Nachrichten werden wieder gelöscht

**Ausgangslage:** Ein systematischer Bug-Audit über alle Bots hat zwei stille Fehler gefunden. Erstens: Die Buttons im öffentlichen Steam-Panel („Steam verknüpfen", „Rang prüfen") liefen seit dem Steam-Umzug auf einen Programmfehler — wer klickte, bekam einfach keine Reaktion. Ursache war ein Lesefehler beim Weiterreichen der Klick-Daten: Der Code griff auf das Feld „values" zu, erwischte dabei aber eine eingebaute Python-Funktion gleichen Namens statt der eigentlichen Auswahl-Werte, und stürzte beim Umwandeln ab. Zweitens: Der SecurityGuard übergab beim Löschen von Scam-Nachrichten einen Begründungs-Parameter, den die Discord-Bibliothek in der eingesetzten Version gar nicht kennt — die Löschung brach mit einem Typfehler ab, der von der Fehlerbehandlung nicht abgefangen wurde. Erkannte Scam-Nachrichten blieben dadurch in bestimmten Fällen einfach stehen.

**Was wurde geändert:** Der Panel-Code liest die Auswahl-Werte jetzt korrekt aus dem Daten-Wörterbuch der Interaktion und prüft dabei den Typ. Der SecurityGuard löscht ohne den nicht unterstützten Parameter — die Begründung steht weiterhin vollständig im Moderations-Log.

**Wie es jetzt funktioniert:** Klick auf „Steam verknüpfen" oder „Rang prüfen" leitet wieder sauber an den Steam-Dienst weiter und antwortet im Chat. Erkennt der SecurityGuard eine Scam-Nachricht (egal ob Einzelfall oder Nachrichten-Serie), wird sie wieder zuverlässig entfernt.

## #100 — Rust-Neuaufbau: der Lane-Router

**Ausgangslage:** Der Router-Voice-Kanal sortiert Spieler nach ihrer gespeicherten Vorliebe (Ranked, Casual, Street Brawl) automatisch in eine passende Lane ein — bevorzugt zu bekannten Mitspielern aus dem Co-Spieler-Graphen, sonst in die erste Lane mit Platz, und wenn nichts passt, wird eine neue erstellt. Ranked verlangt eine verifizierte Rang-Rolle, sonst gibt es eine freundliche DM mit dem Verifikations-Hinweis.

**Geändert:** Komplett in Rust angeschlossen — gleiche Vorlieben-Tabelle, gleiche Panel-Knöpfe (Modus-Wahl plus Auto-Join-Schalter, Kennungen unverändert), gleiche Auswahl-Logik samt Co-Spieler-Vorrang aus dem bereits portierten Aktivitäts-Graphen, gleiche Lane-Neuerstellung über die TempVoice-Maschine (die dafür gelernt hat, Lanes auch für Nutzer zu bauen, die im Router statt im Sammel-Kanal stehen). Wer den Modus im Panel wählt, während er im Router steht, wird sofort einsortiert. Damit ist der letzte als Umstiegs-Voraussetzung markierte Baustein abgehakt; einzige dokumentierte Rest-Lücke des Voice-Bereichs ist die separate Anfänger-Einsortierung samt ihrer Spezial-Lanes.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Die Lane-Wahl ist getestet — Sammel-Kanäle und volle oder leere Lanes werden nie gewählt, Mitspieler-Lanes gewinnen vor der erstbesten. Der Router hängt als Subscriber am zentralen Ereignis-Verteiler wie alle anderen Voice-Bausteine.

## #99 — Rust-Neuaufbau: die Feedback-DMs nach den ersten Voice-Runden

**Ausgangslage:** Wer seine allererste Voice-Session (mindestens 5 Minuten, mit Mitspielern) beendet, bekommt eine freundliche DM mit Feedback-Knopf und einem 4-Fragen-Formular; nach mindestens vier verschiedenen Voice-Tagen folgt einmalig eine zweite, kürzere Nachfrage. Die Antworten landen in der Datenbank und beim Owner. Das war die letzte offene Lücke des Voice-Tracker-Ports.

**Geändert:** Komplett in Rust angeschlossen: Texte und Formular-Fragen wortgleich (das Formular kann jetzt auch im neuen System mehrzeilige Antworten — dafür wurde der Modal-Baukasten um mehrzeilige Felder erweitert), gleiche Auslöse-Regeln (Erst-Session-Erkennung VOR dem Speichern, Mitspieler-Pflicht, 5-Minuten-Grenze, Vier-Tage-Regel einmalig), gleiche Tabellen, gleicher Knopf — auch alte, vor dem Umstieg verschickte Feedback-DMs funktionieren weiter, weil der Knopf seine Anfrage über die Datenbank wiederfindet statt über den Prozess-Speicher.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Tests spielen beide Auslöser durch: Erst-Session verschickt genau eine DM (kurze oder einsame Sessions nicht), die zweite Nachfrage kommt erst ab vier Voice-Tagen und genau einmal. Namen werden ab elf Mitspielern als „+N weitere" gekappt wie im Original.

## #98 — Rust-Neuaufbau: die Min-Rang-Sperre im Panel

**Ausgangslage:** Comp/Ranked-Lane-Besitzer können eine Rang-Untergrenze setzen — Rang-Rollen unterhalb der Schwelle verlieren das Verbinden-Recht, und der Lane-Name bekommt ab Emissary den Zusatz „• ab X". Der Panel-Knopf war im Rust-Port noch gesperrt.

**Geändert:** Komplett angeschlossen: Auswahlmenü mit allen Rängen (plus „Kein Min-Rang" zum Zurücksetzen), Rollen-Sperren exakt nach der Original-Punktelogik (Haupt- und Unterrang-Rollen, Kurzformen inklusive), Aufräumen der Sperren beim Zurücksetzen, Namens-Zusatz über die bestehende, getestete Namenslogik. Nur für Comp/Ranked-Lanes — Chill-Lanes lehnen mit klarer Ansage ab.

**Wie es jetzt funktioniert:** Wie vorher — Besitzer wählt die Schwelle im Panel, niedrigere Ränge können nicht mehr verbinden. Im Panel ist damit nur noch der Lanes-Modus-Wechsel offen.

## #97 — Rust-Neuaufbau Phase 5 (Teil 3): Player-Finder portiert (bleibt aus)

**Ausgangslage:** Der Player-Finder schlägt auf eine Suche im LFG-Kanal passende Mitspieler vor — gefiltert nach typischer Spielzeit, Wochentag, Voice-Aktivität der letzten 14 Tage und Rang-Nähe (±3), sortiert nach Steam-Status (Lobby vor Match vor „im Spiel" vor Discord-online). Er steht vor einem kompletten Redesign.

**Geändert:** Der Logik-Kern ist vereinbarungsgemäß nach Rust portiert, bleibt aber per Schalter deaktiviert (Standard: aus): alle Filter, die Kandidaten-Kette mit den drei „Lebenszeichen"-Bedingungen, die Status-Beschriftungen und ihre Rangfolge, die Datenbank-Zugriffe auf Muster, Aktivität und Steam-Presence (mit 2-Minuten-Frische-Grenze) sowie die 60-Sekunden-Abklingzeit pro Nutzer. Das geplante Redesign kann damit direkt auf der Rust-Basis aufsetzen statt auf dem Alt-Code.

**Wie es jetzt funktioniert:** Gar nicht — und das ist Absicht. Der Schalter bleibt aus, bis das Redesign steht; die Logik ist mit Tests abgesichert, damit beim Redesign klar ist, was das Alt-Verhalten war.

## #96 — Rust-Neuaufbau Phase 7 (Teil 2): die Coaching-Brücke

**Ausgangslage:** Der Bot hält die Coaching-Plattform der Website synchron: Alle zehn Minuten übermittelt er das Coach-Roster (wer die Coach-Rolle trägt, mit Namen und Avatar), und jede Minute holt er fällige Termin-Benachrichtigungen ab und stellt sie als DM zu — Termin geplant, Erinnerung zwei Stunden vorher, Absage.

**Geändert:** Beide Abläufe sind in Rust portiert: gleiche Schnittstellen und Kopfzeilen, gleiche Token-Kette, die DM-Texte wortgleich inklusive Berlin-Zeitformat („Mi, 10.06. um 19:00 Uhr", sommer- wie winterzeitfest), und die zwei wichtigen Schutzregeln des Originals: Ein leeres Coach-Roster wird nie übermittelt (sonst würde ein Discord-Schluckauf alle Coaches von der Website wischen), und Nutzer mit deaktivierten DMs werden als zugestellt bestätigt statt endlos erneut versucht.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Die Datums-Formatierung und alle drei DM-Texte sind mit Tests gegen das Original-Format abgesichert (inklusive Sommer-/Winterzeit-Wechsel). Die Loops starten erst mit der Gateway-Übernahme.

## #95 — Rust-Neuaufbau Phase 6 (Teil 4): der Sicherheits-Wächter

**Ausgangslage:** Der Sicherheits-Wächter schützt den Server vor Scam-Wellen und gekaperten Accounts über drei Pfade: das deterministische Takeover-Muster (Bilder in mehreren Kanälen binnen 30 Sekunden — sofortige Quarantäne ohne KI-Urteil), Mehrkanal-Bursts junger Accounts (mit KI-Bestätigung ab 78 % Sicherheit) und Keyword-Einzeltreffer (Telegram-Werbung, Gewinnversprechen und Co.), bei denen etablierte Accounts einen reversiblen Vorschlag bekommen statt des direkten Vollzugs.

**Geändert:** Alle drei Pfade sind in Rust portiert — gleiche Schwellen, gleiche Schlagwort-Liste, gleicher KI-Prompt, gleiche Fall-Akte in derselben Tabelle, gleiche Reihenfolge im Vollzug (Benachrichtigung an den Betroffenen, dann Aktion, dann Beweise löschen, öffentliche Notiz, Mod-Alarm mit Ban/Timeout-aufheben/Unban-Knöpfen). Der Ereignis-Verteiler liefert dafür jetzt auch Anhang-Zahlen und Account-Alter mit. Eine bewusst konservative Übergangs-Einschränkung: Die Bild-Inhalts-Prüfung (lief über ein externes Kommandozeilen-Werkzeug) ist noch nicht angebunden — Fälle, die nur über Bildinhalte bestätigt würden, landen deshalb als reversibler 60-Minuten-Vorschlag statt als automatischer Vollzug. Nie schärfer als das Original, im Zweifel milder.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Neun Tests decken die Detektions-Kerne ab — Takeover zündet bei Bildern in zwei Kanälen, nicht beim Doppelpost im selben Kanal und nicht außerhalb des Zeitfensters; die Burst-Regeln und Altersgrenzen entscheiden exakt wie das Original; kaputte KI-Antworten fallen sicher auf „kein Scam" zurück.

## #94 — Rust-Neuaufbau Phase 7 (Teil 1): die Onboarding-Knöpfe

**Ausgangslage:** Neue Mitglieder durchlaufen das Onboarding über feste Knöpfe im Regelkanal — der wichtigste davon ist die Regelbestätigung, die die Zugangs-Rolle vergibt. Dazu kommen der Steam-Login-Knopf und die Hinweis-Knöpfe des DM-Assistenten.

**Geändert:** Die bestehenden Knöpfe sind mit unveränderten Kennungen in Rust angeschlossen: Regelbestätigung vergibt die Onboarding-Rolle (mit ehrlicher Rückmeldung, falls die Vergabe scheitert), der Steam-Knopf holt einen frischen Einmal-Login-Link vom Steam-Dienst, und die vier Assistenten-Hinweise (Steam, FAQ, Streamer, Beta) antworten wortgleich wie bisher. Die Schritt-Navigation des geführten Kanal-Flows sagt übergangsweise ehrlich, dass sie umgebaut wird — der volle Flow folgt.

**Wie es jetzt funktioniert:** Die Knöpfe unter den bestehenden Nachrichten im Regelkanal funktionieren nach dem Umstieg weiter — insbesondere kommt jedes neue Mitglied über die Regelbestätigung an seine Rolle.

## #93 — Rust-Neuaufbau: Server-Warnungen auch im neuen Empfänger

**Ausgangslage:** Parallel zum Umbau bekam der Changelog-Empfänger eine neue Aufgabe: Der Server-Monitor postet Speicher-Warnungen als Embed in den Admin-Kanal (mit Ping bei Warnung/Kritisch, Entwarnung ohne) — Lehre aus dem Speicher-Vorfall vom 10. Juni.

**Geändert:** Die neue Warn-Route ist im Rust-Empfänger nachgezogen — gleicher Pfad, gleiche Prüfungen (Token, Pflichtfelder, nur die drei bekannten Stufen), gleiche Embeds mit Stufen-Symbol und -Farbe, gleicher Admin-Ping bei Warnung und Kritisch. Damit bleibt der Server-Monitor auch nach dem Bot-Umstieg ohne Anpassung funktionsfähig.

**Wie es jetzt funktioniert:** Unverändert — der Monitor schickt seine Meldung an den lokalen Empfänger, der Admin-Kanal bekommt das Embed, bei ernsten Stufen klingelt der Ping.

## #92 — Rust-Neuaufbau: Aufräumdienst beim Start

**Ausgangslage:** Nach einem Bot-Neustart können verwaiste Lanes übrig bleiben — Kanäle, die in der Datenbank als Lanes geführt werden, aber leer sind oder gar nicht mehr existieren.

**Geändert:** Der Start-Aufräumlauf ist portiert: 30 Sekunden nach dem Start (wenn der Discord-Zwischenspeicher sicher gefüllt ist — sonst sähen fälschlich alle Lanes leer aus) werden bekannte Lanes geprüft und leere oder verschwundene abgebaut, inklusive Datenbank-Eintrag.

**Wie es jetzt funktioniert:** Wie das Original mit seiner Start-Verzögerung — nur dass die Schutz-Wartezeit hier großzügiger gewählt ist, weil der Rust-Prozess schneller hochkommt als sein Discord-Zwischenspeicher.

## #91 — Rust-Neuaufbau: der Lurker-Modus

**Ausgangslage:** Der 👻-Lurker-Knopf im Lane-Panel war im Rust-Port noch gesperrt. Lurker sind stille Zuhörer: Sie bekommen die Lurker-Rolle, heißen sichtbar „Lurker", und das Lane-Limit wächst um eins, damit sie keinen Spielplatz blockieren — beim Verlassen wird alles automatisch zurückgebaut.

**Geändert:** Komplett in Rust angeschlossen, mit derselben Umschalt-Logik (Knopf an = Rolle + Name + Limit hoch, Knopf aus = alles zurück inklusive des gespeicherten Original-Namens), demselben Datenbank-Merkzettel und denselben Sicherungen: Schlägt die Rollen-Vergabe fehl, wird der Datenbank-Eintrag zurückgerollt; der Namens-Wechsel ist unkritisch und bricht nichts ab. Auch das automatische Aufräumen beim Verlassen der Lane hängt jetzt am zentralen Ereignis-Verteiler.

**Wie es jetzt funktioniert:** Wie vorher — Knopf drücken macht dich zum Lurker, nochmal drücken (oder die Lane verlassen) macht es rückgängig. Damit sind von den Panel-Funktionen nur noch der Lanes-Modus-Wechsel und die Min-Rang-Auswahl offen.

## #90 — Rust-Neuaufbau: Lane-Tag-Filter angeschlossen

**Ausgangslage:** Lane-Besitzer können ihre Lane filtern („nur 25+", „Ragebaiter blockieren") — der Filter setzt Verbinden-Sperren für betroffene Nutzer und trennt sie beim Beitritt. Im Rust-Port war der Panel-Button bisher als „noch nicht freigeschaltet" markiert, weil das Tag-System fehlte.

**Geändert:** Mit dem neuen Tag-System ist der Filter jetzt vollständig angeschlossen: Speichern in derselben Tabelle, Durchsetzung beim Beitritt (Sperre plus Trennung), sofortige Anwendung beim Speichern über das Panel (zwei Auswahlmenüs: Alters-Filter, Ragebaiter-Block), und die Sofort-Reaktion, wenn die Moderation jemandem den Ragebaiter-Marker verpasst, während er in einer geschützten Lane sitzt. Die Aufhebe-Logik respektiert Besitzer-Banns: Wer zusätzlich vom Besitzer gebannt ist, behält seine Sperre auch wenn der Filter ihn freigeben würde. Zwei Original-Eigenheiten sind dokumentiert übernommen: Der gespeicherte „Ton"-Filter wurde auch bisher nie durchgesetzt (toter Zweig), und ohne Tag-Dienst bleibt der Filter wirkungslos.

**Wie es jetzt funktioniert:** Wie im Original — Besitzer stellt den Filter im Panel ein, betroffene Nutzer können nicht mehr verbinden und werden getrennt, alle Komponenten laufen über die getestete Engine und das getestete Tag-System.

## #89 — Rust-Neuaufbau Phase 6 (Teil 3): das Tag-System

**Ausgangslage:** Tags sind das Bindeglied zwischen drei Systemen: Im Onboarding wählen Nutzer ihre Alters- und Ton-Präferenz („25+", „banter_ok", „ragebaiter-free"), die Lane-Filter im Voice-Bereich werten sie aus, und die Moderation vergibt den zeitlich befristeten „Ragebaiter"-Marker (14 Tage Standard-Laufzeit), der bei wiederholtem Fehlverhalten automatisch gesetzt wird.

**Geändert:** Das Tag-System ist als zentrale Anlaufstelle in Rust portiert: gleiche Tabellen, gleiche Gültigkeits-Prüfungen (nur bekannte Schlüssel und Werte, normalisiert), gleiche Standard-Laufzeit, gleicher 5-Minuten-Aufräumlauf für abgelaufene Mod-Tags, und Änderungs-Ereignisse für die angeschlossenen Systeme — das Pendant zu den bisherigen Bot-internen Benachrichtigungen, an denen Lane-Filter und Moderation hängen.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Tests decken die Kernregeln ab: ungültige Tags werden abgewiesen, Wert-Wechsel feuern genau ein Ereignis (gleicher Wert keins), der Ragebaiter-Marker läuft nach 14 Tagen ab und wird vom Aufräumlauf entfernt, und ein Neustart stellt den kompletten Zustand aus der Datenbank wieder her. Damit ist der Unterbau für die noch offenen Lane-Tag-Filter und die Ragebaiter-Automatik der Moderation gelegt.

## #88 — Rust-Neuaufbau Phase 8 (Teil 3): die Turnier-Website

**Ausgangslage:** Die öffentliche Turnier-Seite (Anmeldung per Discord-Login, Team-Verwaltung, Turnierbaum-Vorschau) lief als Web-Server im Bot-Prozess: Einmal-Login-Token, Sitzungs-Cookies mit CSRF-Schutz und elf API-Routen über dem Turnier-Unterbau.

**Geändert:** Komplett in Rust portiert und in den Web-Prozess umgezogen: derselbe Login-Fluss (Weiterleitung zum Link-Dienst, Einmal-Token einlösen, 6-Stunden-Sitzung mit gleitender Verlängerung), dieselben Sicherheits-Kopfzeilen, derselbe Turnierbaum-Generator (Team-Schnitt aus Rang-Punkten, Setzliste Erster-gegen-Letzter, Freilose, Finale/Halbfinale-Beschriftung — inklusive der Original-Eigenheit, dass Unterrang 0 als Mitte gewertet wird) und alle Anmelde-Regeln: offener Zeitraum, verifizierter Steam-Account als Rang-Quelle, Team-Pflichten, Nur-Ersteller-Rechte bei Umbenennen und Rauswerfen. Eine dokumentierte Übergangs-Notiz: Die Turnier-Rollen-Prüfung lief bisher über den Discord-Zwischenspeicher des Bots und wurde stillschweigend übersprungen, wenn der nicht bereit war — der Web-Prozess hat keinen Discord-Zugang, also gilt vorerst genau dieses Original-Ausweichverhalten.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Die Rust-Version lief parallel zum Live-Original gegen dieselben echten Daten: die Übersichts-Antwort ist JSON-identisch, die Seite byte-identisch, abgewiesene Anfragen liefern dieselben Status-Codes. Dazu Tests für den kompletten Login-und-Anmelde-Fluss und den Turnierbaum mit Referenz-Ausgabe aus dem Python-Original.

## #87 — Rust-Neuaufbau Phase 8 (Teil 2): der Turnier-Unterbau

**Ausgangslage:** Anmeldungen, Teams, Turnier-Zeiträume und die Einmal-Anmelde-Links der Turnier-Website werden in vier Tabellen verwaltet — mit Regeln wie „Team-Namen sind 2–32 Zeichen und pro Server einmalig (Groß/Klein egal)", „Team-Anmeldung braucht ein existierendes Team" und „eine neue Turnier-Phase deaktiviert automatisch die alte".

**Geändert:** Der komplette Unterbau ist in Rust portiert — gleiche Tabellen, gleiche Prüfungen, gleiche Rückmeldungen (eingefügt/aktualisiert/unverändert, inklusive der Feinheit, dass ein fehlender Anzeigename den alten nicht überschreibt). Die Einmal-Token für den Website-Login verhalten sich identisch: einmal eingelöst oder abgelaufen heißt ungültig, alte Token werden beim Anlegen neuer weggeräumt.

**Wie es jetzt funktioniert:** Sieben Tests decken die Regeln ab — Team-Anlage ist wiederholbar statt doppelt, Anmeldungs-Wechsel von Solo zu Team, Phasen-Wechsel deaktiviert den Vorgänger, Token sind strikt einmalig. Discord-Menüs und die Turnier-Website docken als Nächstes hier an.

## #86 — Rust-Neuaufbau Phase 8 (Teil 1): der Team-Balancer

**Ausgangslage:** Für Custom Games und Turniere teilt der Balancer die anwesenden Spieler anhand ihrer Rang-Punkte in zwei gleich große Teams — er probiert alle Kombinationen durch und bewertet jede mit einer Formel aus Summen-Differenz, Durchschnitts-Differenz und Team-Varianz.

**Geändert:** Der Algorithmus ist in Rust portiert — verhaltensgleich bis in die Kombinations-Reihenfolge und mit Referenzwerten aus dem Python-Original auf neun Nachkommastellen abgesichert. Zwei Eigenheiten des Originals wurden dabei dokumentiert (und bewusst übernommen statt still „verbessert"): Bei großer Rang-Spreizung dominiert der Varianz-Anteil der Formel und der Balancer baut dann homogene statt gleich starker Teams — ob das so gewollt ist, ist eine fachliche Entscheidung für später. Und bei nur zwei oder drei Spielern liefert der Notfall-Pfad ein leeres zweites Team.

**Wie es jetzt funktioniert:** Identisch zu vorher — gleiche Eingabe, gleiche Teams. Der Discord-Ablauf drumherum (Auswahl-Menü, Team-Kanäle, Verschieben) folgt mit dem Custom-Games-Port.

## #85 — Rust-Neuaufbau Phase 6 (Teil 2): der KI-Moderations-Kern

**Ausgangslage:** Der KI-Moderator bewertet jede Nachricht im Haupt-Chat mit einem bewusst rau kalibrierten Regelwerk („Gaming-Ton ist normal, sei NICHT überempfindlich") und entscheidet dreistufig: eindeutige Fälle (explizites NSFW, Scam) werden ab 90 % Sicherheit sofort gelöscht, echte Verstöße ab 78 % als Vorschlag mit Bestätigen/Ban/Ablehnen-Buttons an die Mods gegeben, und Ragebait wird nur gezählt — vier Treffer in zwei Stunden eskalieren zu einem Mod-Vorschlag.

**Geändert:** Dieser Kern ist in Rust portiert: das Regelwerk wortgleich, die Antwort-Auswertung mit denselben Gültigkeits-Prüfungen und Grenzwert-Klemmungen, das komplette Schwellen-Routing, das Ragebait-Zeitfenster in denselben Tabellen, die Fall-Akte (wer, was, Kategorie, Sicherheit, Mod-Entscheidung) und die Review-Buttons mit unveränderten Kennungen. Pro Nutzer gilt weiter die 2-Sekunden-Bremse, Moderatoren werden nie gescannt.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Auswertung und Routing sind mit Testfällen abgesichert — auch die Eckfälle: „delete" außerhalb der Sofort-Kategorien wird trotz 95 % nur zum Vorschlag, ungültige KI-Antworten landen sicher bei „braucht Kontext" statt in einer Aktion. Bewusst noch offen: die Kontext-Nachladung bei Grenzfällen, Bild-Bewertung und die Ton-Tag-Sonderschwellen — sie folgen mit dem Tag-System.

## #84 — Rust-Neuaufbau Phase 6 (Teil 1): die KI-Anbindung — und der Streamer-Erkenner denkt wieder mit

**Ausgangslage:** Mehrere Module brauchen die MiniMax-KI: der Streamer-Erkenner für unklare Namens-Paare, die Gruppensuche für die Zweitprüfung, später Moderation und Chat. Seit dem Rust-Port des Streamer-Erkenners lief dieser im reinen Heuristik-Modus.

**Geändert:** Die KI-Anbindung ist in Rust portiert — beide Betriebsarten des Originals: der Token-Plan-Modus (Anthropic-kompatible Schnittstelle) und der Standard-Modus (klassische Chat-API), mit derselben Schlüssel-Suchreihenfolge und denselben Standard-Einstellungen. Die nicht konfigurierten OpenAI-/Gemini-Pfade wurden bewusst weggelassen. Direkt angeschlossen: Der Streamer-Erkenner bekommt seine KI-Zweitmeinung zurück — wortgleicher Prompt, Temperatur 0, knappes Antwort-Limit, JSON-Auswertung wie gehabt. Ohne konfigurierten Schlüssel fällt er automatisch auf die konservative Heuristik zurück.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Beide API-Betriebsarten sind gegen einen Mock-Server verifiziert — Kopfzeilen, Anfrage-Aufbau und Antwort-Auswertung (inklusive des Falls, dass die KI Denk-Fragmente mitliefert, die übersprungen werden). Damit ist die größte dokumentierte Lücke aus dem Brücken-Port geschlossen.

## #83 — Rust-Neuaufbau Phase 5 (Teil 2): die LFG-Erkennung

**Ausgangslage:** Die Gruppensuche erkennt im LFG-Kanal automatisch, ob eine Nachricht eine Mitspieler-Suche ist — über eine Wortmuster-Heuristik, die auf 500 echten Kanal-Nachrichten kalibriert wurde („wer bock", „suche +2", „jemand wach?", inklusive Privat-Kontakt-Ausnahme), plus Bewertungs-Formeln für Rang-Nähe und Zeit-Übereinstimmung bei den Empfehlungen.

**Geändert:** Die komplette Erkennungs-Heuristik, die Rang-Nähe-Bewertung (Unterrang-genau, drei Toleranz-Stufen), die Zeit-Übereinstimmung (typische Stunden ±2 mit Mitternachts-Übergang, typische Wochentage) und das Wunsch-Filter-Parsing („25+", „ragebaiter-free" — mit Ziffern-Grenzen, sodass „125+ hp" nicht zündet) sind in Rust portiert und mit Referenzfällen aus dem laufenden Python-Original abgesichert — 17 Beispiel-Nachrichten liefern exakt dieselbe Ja/Nein-Entscheidung.

**Wie es jetzt funktioniert:** Diese Bausteine sind die testbare Grundlage; der sichtbare Ablauf (Nachricht erkennen → in die passende Lane lotsen → Mitspieler vorschlagen) folgt, sobald die KI-Zweitprüfung mit der KI-Schicht portiert ist.

## #82 — Rust-Neuaufbau Phase 5 (Teil 1): Aktivitätsmuster und Mitspieler-Graph

**Ausgangslage:** Wer wann typischerweise im Voice ist und wer mit wem spielt, wird in zwei Tabellen gepflegt, aus denen Gruppensuche, Statistiken und Empfehlungen lesen. Zwei Schreiber füttern sie: ein 6-Stunden-Lauf für die Muster und ein 10-Minuten-Takt für die aktuellen Voice-Paarungen.

**Geändert:** Beide Schreiber sind in Rust portiert: Der Muster-Lauf wertet die letzten 14 Tage aus (typische Top-3-Stunden und -Wochentage, Sitzungs-Zähler, Minuten, letzte Aktivität — bis auf die Reihenfolge bei Gleichstand identisch zur Python-Sortierung) und überschreibt idempotent. Der 10-Minuten-Takt erfasst alle Paarungen in Voice-Kanälen mit mindestens zwei echten Mitgliedern bidirektional samt Anzeigenamen. Dabei wurde ein stiller Zähl-Fehler des Originals NICHT übernommen: Der alte 6-Stunden-Lauf addierte zusätzlich die kompletten 2-Wochen-Mitspieler-Aggregate bei jedem Lauf erneut auf — viermal am Tag dieselben Sitzungen obendrauf, zusätzlich zum 10-Minuten-Takt. Im Rust-Port schreibt nur noch der korrekte, inkrementelle Pfad.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Die Muster-Berechnung ist mit Referenzwerten aus dem Python-Original abgesichert, das Mitspieler-Tracking mit Datenbank-Tests gegen das echte Schema (beidseitige Einträge, Akkumulation über mehrere Takte, Namens-Pflege). Beide Läufe starten erst mit der Gateway-Übernahme.

## #81 — Rust-Neuaufbau Phase 4c (Teil 4): das Lane-Steuerungs-Panel

**Ausgangslage:** Das Interface-Panel im Voice-Bereich ist die Schaltzentrale für Lane-Besitzer: Region, Besitz übernehmen, Limit, Kick/Bann/Entbannen, Schnell-Vorlagen, Presets, Rang-Präferenz und Umbenennen — alles über Buttons unter einer festen Nachricht.

**Geändert:** Der Kern des Panels ist in Rust portiert, mit unveränderten Button-Kennungen — das bestehende Panel im Kanal funktioniert nach dem Umstieg ohne Neuposten weiter. Umgesetzt: Region DE/EU (sperrt bzw. entsperrt die English-Only-Rolle am Kanal und merkt sich die Wahl), Besitz-Übernahme mit den Original-Regeln (nur wenn der Besitzer weg ist, nur die drei am längsten Verbundenen, mindestens 20 Minuten im Kanal — mit denselben erklärenden Ablehnungs-Texten), Limit-Dialog mit Vorlagen-Obergrenzen (Street Brawl bleibt bei 4), Kick/Bann/Entbannen über Mitglieder-Auswahllisten (Bann gilt besitzerweit über alle Lanes), Duo/Trio/Reset-Schnellknöpfe, Preset speichern/laden, Rang-Präferenz für Chill-Lanes und Owner-Umbenennen. Dafür wurde die TempVoice-Engine um eine saubere Befehls-Fassade erweitert (Claim-Prüfung, Limit-Kappung, Region, Besitzwechsel mit Bann-Tausch).

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Jeder Button prüft zuerst „bist du in einer Lane?" und „gehört sie dir?" mit den bekannten Antworten. Workspace-weit bleiben alle Tests grün; die Claim-/Limit-/Bann-Logik sitzt in der getesteten Engine. Drei Panel-Funktionen sind bewusst noch nicht freigeschaltet und sagen das ehrlich an: Tag-Filter, Lurker-Modus und der Lanes-Modus-Wechsel — ihr Unterbau (Tag-Filter-System, Lurker-Verwaltung, Router-Lanes) folgt vor dem Voice-Umstieg.

## #80 — Rust-Neuaufbau Phase 4c (Teil 3): das Rang-Türsteher-System

**Ausgangslage:** Comp/Ranked-Lanes haben einen Rang-Anker: Der Erstbesitzer (oder das erste rangierte Mitglied) bestimmt ein Score-Fenster von ±9 Unterrang-Punkten (anderthalb Hauptränge), und nur Rang-Rollen in diesem Fenster dürfen verbinden. Der Kanal heißt nach dem Anker („Phantom 3"). Wichtigste Eigenschaft: Es wird nie jemand rausgeworfen — nur die Verbinden-Rechte werden gesteuert.

**Geändert:** Komplett in Rust portiert: die Rang-Erkennung aus Rollennamen (Unterrang-Rollen wie „Asc 3" schlagen Haupt-Rollen, Kurzformen werden aufgelöst, Unterrang-Fallback aus dem verknüpften Steam-Account, sonst Mitte), die Fenster-Berechnung, die Anker-Wahl mit Erstbesitzer-Vorrang (direkt an die neue TempVoice-Engine angebunden), die Sammel-Aktualisierung der Kanal-Rechte in einem einzigen Discord-Aufruf (Jedermann-Sperre, erlaubte Unterrang-Rollen rein, nicht mehr erlaubte Rang-Rollen raus, fremde Einstellungen unangetastet) und die Wahl der „wirklich spielenden" Gruppe über dieselbe Presence-Kohorten-Logik wie der Live-Status. Anker überleben Neustarts über dieselbe Datenbank-Tabelle.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Fenster-Berechnung, Rollen-Erkennung, Rollen-Auswahl im Fenster und Namensgebung sind mit Referenzwerten aus dem Python-Original abgesichert (acht Testfälle, inklusive der Eckfälle Eternus 6 und Initiate 1). Das System hängt als weiterer Subscriber am zentralen Ereignis-Verteiler.

## #79 — Rust-Neuaufbau Phase 4c (Teil 2): der Live-Status an den Voice-Lanes

**Ausgangslage:** Die Anzeige „Lane 1 - im Match Min 17 (4/6)" an den Voice-Kanälen kommt von einem Minuten-Worker, der Steam-Presence-Daten mit den Kanal-Mitgliedern abgleicht und daraus die größte zusammen spielende Gruppe ermittelt — inklusive Party-Aufstockung für Mitglieder ohne verknüpften Steam-Account.

**Geändert:** Komplett in Rust portiert: Presence-Auswertung (mit Frische-Grenze von 3 Minuten, Minuten-Erkennung auch aus dem lokalisierten Steam-Text), Kohorten-Wahl (Match schlägt Lobby, bei Gleichstand die größere Gruppe, Server-Zuordnung vor Unbekannt), Party-Abgleich (beste Party nach Überlappung mit der Gruppe, gemeldeter Größe und Frische; fehlende unverknüpfte Mitspieler werden bis Partygröße aufgestockt) und die komplette Rename-Disziplin: 6 Minuten Abstand zwischen Umbenennungen (10 ab Match-Minute 25), Match-Ende und Status-Löschung dürfen den Abstand umgehen, und reine Mitgliederzahl-Änderungen ohne Spielstatus benennen NIE um. Auch die Standort-Tabelle, die anderen Diensten sagt, in welchem Kanal eine Steam-ID gerade sitzt, wird identisch gepflegt.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Die gesamte Auswertungs-Kette ist mit Referenzwerten aus dem Python-Original abgesichert — dieselben Presence-Zeilen ergeben dieselbe Einstufung, dieselben Gruppen, dieselben Suffixe, dieselbe Umbenennen/Warten-Entscheidung in allen sechs Regelfällen. Der Worker läuft im selben 60-Sekunden-Takt gegen dieselben Tabellen.

## #78 — Rust-Neuaufbau Phase 4c (Teil 1): die Steam-Verknüpfungs-Erinnerung

**Ausgangslage:** Wer ohne verknüpften Steam-Account regelmäßig im Voice ist, bekommt einmalig eine freundliche DM mit dem Verknüpfungs-Link — aber bewusst erst am zweiten Voice-Tag und erst nach 30 Minuten am Stück, damit niemand beim ersten Reinschnuppern angeschrieben wird.

**Geändert:** Komplett in Rust portiert, als weiterer Subscriber des Ereignis-Verteilers: gleiche Ausnahmen (Datenschutz-Widerspruch, ausgenommene Rollen, bereits verknüpft, bereits erinnert), gleiche Merkzettel in denselben Datenbank-Feldern (Erst-Sichtung, Erledigt-Status, DM-Referenz), gleicher DM-Inhalt samt Datenschutz-Erklärung, frischem Steam-Login-Link vom Steam-Dienst und Schließen-Button — dessen Kennung unverändert bleibt, damit auch die Schließen-Buttons ALTER, vor dem Umstieg verschickter DMs weiter funktionieren.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Tests decken die Kernregeln ab: Tag eins wird nur vorgemerkt (keine DM), ab Tag zwei startet die 30-Minuten-Beobachtung, wer verknüpft oder erledigt ist wird übersprungen, und der Versand schreibt Erledigt-Status und DM-Referenz korrekt in die Datenbank. Nebenbefund dieser Etappe: Der „voice_reaction_dm"-Baustein stellte sich als falsch einsortierter Twitch-Verkaufs-Melder heraus (liest die Twitch-Bot-Datenbank, standardmäßig aus) — der gehört in den Twitch-Bot-Umbau und wird dort übernommen, nicht hier.

## #77 — Rust-Neuaufbau Phase 4b: TempVoice-Kern

**Ausgangslage:** TempVoice ist mit Abstand das größte Einzelstück des Bots (~6.700 Zeilen): Beitritt in einen Sammel-Kanal erstellt automatisch eine eigene Voice-Lane, mit Besitzer-Logik, Bann-Listen, Rang-Namen und Aufräumen.

**Geändert:** Der Verhaltens-Kern ist in Rust: Join-to-create aus allen drei Sammel-Kanälen (Chill mit Rang-Namen aus Präferenz oder Rollen, Street Brawl mit festen 4er-Lanes, Comp/Ranked), Besitzer-Lebenszyklus (Auto-Übergabe an das am längsten verbundene Mitglied beim Verlassen, Besitzer-Nachtrag bei herrenlosen Lanes), Owner-Bann-Listen als Kanal-Rechte, Löschen leerer Lanes, Namens-Schutzregeln (45-Sekunden-Fenster, nie bei Live-Match-Anzeige) und die Rang-Mathematik (Haupt- und Unterränge, Kurzformen wie „Asc 3", Durchschnittsrang) — alles mit Referenzwerten aus dem Python-Original abgesichert. Datenbank-Verträge unverändert: Lanes, Bann-Listen, Voreinstellungen und Rang-Präferenzen nutzen dieselben Tabellen, Erstbesitzer und Quell-Kanal bleiben bei Updates erhalten wie bisher.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** 21 Tests decken die Kernpfade ab — Lane-Erstellung mit Rang-Namen, Präferenz schlägt Rollen, Besitzer-Übergabe an den Ältesten, Löschung beim letzten Verlassen, Bann-Rechte beim Erstellen — gespielt gegen einen Discord-Mock und die echten Tabellen-Schemata. Bewusst noch offen (vor dem Voice-Umstieg): das Steuerungs-Panel mit seinen Buttons, Tag-Filter/Lurker-Sonderlogik und die Rang-Berechtigungs-Kopplung — sie folgen mit dem Rang-Lane-Manager.

## #76 — Rust-Neuaufbau Phase 4a: das Voice-Tracking-Fundament

**Ausgangslage:** Das Voice-Session-Tracking ist die Datenbasis für die halbe Community-Statistik — Bestenlisten, Heatmaps, Mitspieler-Netzwerk und Gruppensuche lesen alle aus den Tabellen, die es schreibt. Im Original ist es einer von fünf Lauschern, die sich unkoordiniert dasselbe Voice-Ereignis teilen.

**Geändert:** Der Tracking-Kern ist als erster Subscriber des zentralen Ereignis-Verteilers in Rust portiert: Sessions starten ab zwei aktiven (ungestummten) Nicht-Bots im Kanal, Stummschalten beendet die Aktivität — außer für Mitglieder mit der Schonfrist-Rolle, die drei Minuten Karenz bekommen. Beim Ende werden Punkte berechnet (1 pro Minute plus Bonus bei vollen Kanälen), Gesamtwerte hochgezählt und die Session mit Mitspielern, Nutzerverlauf und Spitzenbelegung in die Historie geschrieben — feldgenau in dieselben Tabellen und Formate wie bisher, inklusive der Datenschutz-Regel: Wer widersprochen hat, wird nie erfasst. Die drei Wartungs-Schleifen (Keep-Alive, Schonfrist-Ablauf, Aufräumen verwaister Sessions) laufen wie im Original.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Acht Tests spielen den Lebenszyklus komplett durch — Beitritt zu zweit startet Sessions, allein nicht, Stummschalten ohne Rolle beendet, mit Rolle hält die Karenz, Opt-out wird nie erfasst, verwaiste Sessions räumt der Wächter auf — und prüfen die Datenbank-Schreibvorgänge gegen das echte Tabellen-Schema; die Punkteformel ist mit Referenzwerten aus dem Python-Original abgesichert. Bewusst noch offen für 4b: das Feedback-Nachrichten-System nach der ersten Session und die Statistik-Befehle — beides kommt, bevor der Voice-Bereich umgeschaltet wird.

## #75 — Rust-Neuaufbau Phase 3 komplett: der Streamer-Erkenner

**Ausgangslage:** Das letzte Stück der Brücken-Phase: Der Abgleich, der alle 6 Stunden neue Twitch-Streamer gegen die Discord-Mitgliederliste hält — mit Namens-Normalisierung (Akzente raus, Leetspeak übersetzt, Anhängsel wie „TTV" und „live" entfernt), Ähnlichkeits-Berechnung und der Drei-Wege-Entscheidung: automatisch verknüpfen, als Vorschlag mit Bestätigen/Ablehnen-Buttons an die Mods geben, oder verwerfen.

**Geändert:** Komplett in Rust portiert, inklusive des Ähnlichkeits-Algorithmus aus Pythons Standardbibliothek (difflib), der Zeichen für Zeichen nachgebaut wurde. Damit das beweisbar stimmt, wurden Referenzwerte direkt aus dem laufenden Python-Original gezogen und als Tests eingebacken — „N4ni" wird zu „nani", „drag_skope | TTV" zu „dragskope", und die Ähnlichkeit von „dragskope" zu „dragscope" ist auf zwölf Nachkommastellen identisch. Der Merkzettel bewerteter Streamer nutzt dieselbe Datei wie bisher — offene Vorschläge überleben also auch den Umstieg. Die Review-Buttons laufen über das neue Klick-Routing samt Mod-Rechte-Prüfung.

**Wie es jetzt funktioniert:** Wie bisher — alle 6 Stunden automatisch (der erste Lauf nach einem Neustart wird übersprungen, den Voll-Abgleich startet ein Admin bewusst per Kommando), nur eindeutige, exakte Namens-Treffer werden automatisch verknüpft, alles im Graubereich geht an die Mods. Eine ehrliche Übergangs-Einschränkung: Die KI-Zweitmeinung bei unklaren Fällen kommt erst mit der KI-Schicht in Phase 6 — bis dahin gilt die konservative Heuristik, exakt so, wie sich das Original verhält, wenn seine KI nicht verfügbar ist. Damit ist Phase 3 abgeschlossen; live geht das gesammelt mit dem Bot-Umstieg.

## #74 — Rust-Neuaufbau Phase 3b: Twitch-Klick-Tracking und das letzte Glied der Klick-Kette

**Ausgangslage:** #73 hatte das Klick-Routing gebaut, aber zwei Lücken gelassen: Die Twitch-Live-Buttons („Auf Twitch ansehen" unter Live-Ankündigungen) wurden noch nicht verarbeitet, und es fehlte die Übergabe von der echten Discord-Verbindung an das Routing — Klicks und Slash-Befehle kamen also noch nirgends an.

**Geändert:** Beide Lücken sind zu. (1) Die Twitch-Live-Brücke: Ein Klick auf den Live-Button wird beim Twitch-Bot als Zählung verbucht (mit Einmal-Schlüssel pro Klick — doppelt zählt nicht) und der Nutzer bekommt seinen persönlichen Twitch-Link als nur für ihn sichtbare Antwort. Beim Start holt sich der Bot alle aktiven Live-Ankündigungen, damit auch Buttons unter älteren Nachrichten funktionieren — und wenn ein Klick auf eine unbekannte Ankündigung trifft, lädt er die Liste einmal frisch nach, statt ins Leere zu laufen (das konnte das Original nicht). (2) Die Discord-Verbindung reicht jetzt alle Klicks, Eingabefenster und Slash-Befehle ans Routing weiter — mit derselben 2-Sekunden-Regel wie bisher: Dauert eine Antwort länger, erscheint erst „Bot denkt nach", dann die echte Antwort.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Gegen einen Mock-Twitch-Bot getestet: Klick-Zählung mit korrektem Einmal-Schlüssel und feldgenauem Inhalt, Standard-Beschriftung bei leerem Button-Text, Nachlade-Verhalten bei unbekannten Klicks, Hinweis bei wirklich abgelaufenen Ankündigungen. Damit ist die Abhängigkeit aus #72 aufgelöst: Vermittler und Discord-Verbindung können beim Umschalten gemeinsam wandern, weil die Klick-Verarbeitung jetzt komplett in Rust existiert. Es fehlt aus Phase 3 noch der Streamer-Erkennungs-Scan (läuft alle 6 Stunden) — danach ist die Brücken-Phase komplett.

## #73 — Rust-Neuaufbau Phase 3a: Klick-Verarbeitung und Steam-Brücke

**Ausgangslage:** Phase 2 hatte die offene Flanke benannt: Wer die Discord-Verbindung besitzt, muss auch alle Button-Klicks verarbeiten — sonst posten wir tote Knöpfe. Außerdem ist die Steam-Brücke (der „dünne Arm", über den /betainvite, /steam, Rang-Checks und die Link-Panels mit dem Rust-Steam-Bot reden) bisher Python.

**Geändert:** Zwei Bausteine in Rust: (1) Ein zentrales Klick- und Befehls-Routing — Buttons werden über exakte Kennungen oder Präfixe (z. B. alle `betainvite:`-Schritte mit einem Eintrag) an ihre Verarbeiter geleitet, Slash-Befehle kommen aus einer Registry, die gleichzeitig die Discord-Definitionen fürs Synchronisieren liefert. (2) Die komplette Steam-Brücke: alle 14 Slash-Befehle, beide Panels samt Alt-Kennungen früher geposteter Panels, der gesamte Einladungs-Funnel, das Freundescode-Eingabefenster (einzige lokale UI — alles andere wird 1:1 durchgereicht) und die `!steam_*`-Admin-Kommandos.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Für die Tests wurde ein Mock-Steam-Bot hochgefahren, der jede Anfrage aufzeichnet: Das Leitungsformat (Ereignis-Art, Nutzer/Server/Kanal, Befehlsname samt Argumenten, Freundescode als Werteliste) ist feldgenau identisch zum Python-Original, ebenso die Rückrichtung — Text, Embed, Sichtbarkeit, Link-Button und interaktive Buttons aus der Steam-Bot-Antwort. Auch der Ausfall-Fall („Steam-Bot nicht erreichbar") antwortet wortgleich. Live geht davon noch nichts — es fehlt bewusst das letzte Stück (die Übergabe der Klicks von der echten Discord-Verbindung an dieses Routing), das zusammen mit der Twitch-Brücke in Phase 3b kommt.

## #72 — Rust-Neuaufbau Phase 2: Vermittler, Changelog-Dienst und Event-Fundament

**Ausgangslage:** Andere Bots (z. B. der Twitch-Bot) führen Discord-Aktionen über den internen „Master-Broker" aus — eine lokale Schnittstelle mit Doppel-Absicherung (nur localhost + Token) und Schutz gegen versehentliche Doppel-Ausführung: Jede Aktion trägt einen Einmal-Schlüssel; Wiederholungen liefern das gespeicherte Ergebnis statt z. B. eine Nachricht zweimal zu senden. Dazu kommt der Changelog-Empfänger, über den diese Ankündigungen hier gepostet werden. Beides hing bisher am Python-Prozess.

**Geändert:** Beide Dienste sind vollständig in Rust nachgebaut (gehen noch NICHT live), dazu zwei Fundament-Stücke für alles Weitere: ein zentraler Event-Verteiler — künftig hören alle Bereiche auf EINE normalisierte Ereignis-Quelle statt wie bisher fünf Voice- und sechs Nachrichten-Lauscher nebeneinander — und die Discord-Anbindung selbst, sauber getrennt in Sofort-Aktionen (funktionieren ohne Live-Verbindung) und Live-Daten (brauchen die Gateway-Session, die bis zum Umschalten beim Python-Bot bleibt).

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Der Rust-Vermittler wurde auf einem Testport neben den laufenden Python-Vermittler gestellt: Authentifizierungs-Fehler, Antwort-Format und Fehlertexte sind deckungsgleich; die Doppel-Ausführungs-Sperre ist mit eigenen Tests abgedeckt (Wiederholung liefert Cache, anderer Inhalt zum selben Schlüssel wird abgelehnt, parallele Wiederholung wartet aufs Ergebnis). Wichtige Erkenntnis für den Umschaltplan: Interaktive Buttons, die der Vermittler postet, werden vom Besitzer der Live-Verbindung verarbeitet — Vermittler und Live-Verbindung müssen deshalb ZUSAMMEN umgeschaltet werden, nicht einzeln. Das ist dokumentiert und eingeplant.

## #71 — Rust-Neuaufbau: Aktivitäts-Statistiken portiert — Phase 1 komplett

**Ausgangslage:** Die öffentliche Statistik-Seite (Voice-Heatmaps, Rang-Verteilung, Bestenlisten, persönliche Statistiken mit Discord-Login) war mit 22 Endpunkten der zweite große Webdienst im Python-Bot-Prozess. Pikantes Detail aus der Analyse: Die „Rang-Schätzung über Mitspieler" für Spieler ohne verknüpften Steam-Account war seit jeher wirkungslos — sie lieferte Kategorien, die an jeder einzelnen Verwendungsstelle wieder herausgefiltert wurden. Ein stiller Bug, den nie jemand bemerkt hat, weil das Ergebnis „funktionierte".

**Geändert:** Alle 22 Endpunkte sind nach Rust portiert (gehen noch NICHT live): die acht Analyse-Ansichten, beide Bestenlisten, die sechs persönlichen Me-Endpunkte samt Discord-Login-Flow und die Sicherheits-Schicht (CORS-Allowlist, Cache-Regeln, signierte Session-Cookies). Die wirkungslose Mitspieler-Heuristik wurde nicht mitgenommen — verhaltensgleich, aber ehrlich. Nebenbei wurde aus „eine Datenbank-Abfrage pro Sitzungszeile" (das Original fragte den Rang desselben Spielers hunderte Male pro Anfrage neu ab) ein Zwischenspeicher pro Anfrage.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Wieder Testport gegen laufende Python-Version, gleiche Datenbank: **18 von 18 vergleichbaren Endpunkten liefern identisches JSON** — inklusive Heatmap-Bucketing, Wochentrends, Rundungen und Fehlerfällen; die ausgelieferte HTML-Seite ist byte-identisch. Die login-pflichtigen Me-Endpunkte wurden mit selbst signierten Test-Sessions gegen eine Datenbank-Kopie durchgespielt (echte Logins bleiben beim Umschalten gültig, weil die Cookie-Signierung nachweislich byte-gleich ist). Damit ist Phase 1 des Neuaufbaus komplett: Tierlist + Statistiken warten fertig verifiziert auf die Umschalt-Freigabe — die Checkliste dafür liegt in `rust/docs/02-cutover-phase1.md`.

## #70 — Rust-Neuaufbau: Tierlist komplett portiert und auf echten Daten bewiesen

**Ausgangslage:** Die öffentliche Tierlist (Hero-Winrates, Build-Votes, Admin-Pflege) lief als einer von sechs Webdiensten im Python-Bot-Prozess. Ihre Admin-Anmeldung griff dabei direkt in die internen Session-Daten des Dashboards — das funktioniert nur, solange alles in einem Prozess steckt, und genau diese Verquickung soll weg.

**Geändert:** Die komplette Tierlist ist jetzt in Rust nachgebaut (geht noch NICHT live, läuft parallel zur Python-Version): alle öffentlichen Endpunkte (Heldenliste, Tierlist in drei Rang-Buckets, Verlauf), das Build-Voting mit 5-Sekunden-Sperre pro Absender, die Admin-Endpunkte und der automatische Daten-Abruf von der Deadlock-API. Zwei Dinge wurden dabei sauberer gelöst: Die Admin-Anmeldung fragt jetzt über eine offizielle interne Schnittstelle beim Dashboard nach statt heimlich in dessen Speicher zu greifen, und die Session-Cookies werden byte-identisch zum Original signiert — ein Nutzer bleibt beim späteren Umschalten eingeloggt.

**Wie es jetzt funktioniert (und wie das bewiesen ist):** Die Rust-Version wurde auf einem Testport gegen die laufende Python-Version gestellt — gleiche Datenbank, gleiche Anfragen. Ergebnis: alle sieben Lese-Endpunkte liefern **identisches JSON** bis aufs letzte Feld, alle Fehlerfälle (ungültige Build-ID, unbekannter Build, fehlende Anmeldung) antworten mit denselben Statuscodes und Texten, und das Voting wurde gegen eine Datenbank-Kopie durchgespielt (Stimme zählt, Sperre greift). Dafür musste sogar Pythons Rundungsverhalten exakt nachgebaut werden — die naive Variante rundete 50.365 in die falsche Richtung. Umgeschaltet wird erst nach Freigabe; bis dahin bedient weiterhin Python den Live-Betrieb.

## #69 — Rust-Neuaufbau gestartet: das Fundament steht

**Ausgangslage:** Der Bot ist über die Jahre zu einem 71.000-Zeilen-Python-Prozess gewachsen, der neben Discord auch sechs Webdienste gleichzeitig betreibt. Vieles ist doppelt (drei identische Server-Hüllen, zwei parallele Turnier-APIs, doppeltes Einladungs-Tracking), die Konfiguration ist über 38 Dateien verstreut — und ein Bot-Neustart reißt alle Websites mit, weil alles in einem Prozess steckt.

**Geändert:** Unter `rust/` beginnt der komplette Neuaufbau in Rust — nach dem bewährten Muster von Steam- und Twitch-Bot: Der Python-Bot läuft unverändert weiter, fertige Teile übernehmen einzeln und erst nach Freigabe. Phase 0 legt das Fundament:

- **Zwei getrennte Prozesse** statt einem: Bot (Discord + interne Schnittstellen) und Web (alle Websites + Dashboard). Künftige Bot-Neustarts treffen die Websites damit nicht mehr.
- **Eine zentrale Konfiguration** statt 166 verstreuter Umgebungs-Zugriffe — beim Start einmal gelesen und geprüft, mit denselben Variablennamen wie bisher.
- **Eine Datenbank-Schicht** auf die bestehende gemeinsame Datenbank: ein serialisierter Schreibkanal, beliebig viele parallele Leser, und sie weigert sich, bei falschem Pfad stillschweigend eine leere Datenbank anzulegen (klassische Fehlerquelle).

**Wie es jetzt funktioniert:** Beide Rust-Prozesse starten, lesen die echte Bot-Datenbank (116 Tabellen erkannt, rein lesend geprüft) und warten sauber auf ihr Stopp-Signal — sie binden noch keinen Port und übernehmen noch keine Funktion. Die Datenbank bleibt der Vertrag zwischen Alt und Neu: Rust ändert kein Schema, bevor ein Bereich offiziell übernommen wird. Der vollständige Plan mit Phasen, Architektur-Entscheidungen und Schema-Snapshot liegt in `rust/docs/`; 14 automatische Tests plus Format- und Lint-Prüfung sichern jede weitere Phase ab.

## #68 — Steam-Brücke rendert jetzt echte Buttons

**Ausgangslage:** Die Antworten des Steam-Dienstes (Einladungs-Flow, Link-Panel) kamen in Discord ohne sichtbare Schaltflächen an — die Brücke registrierte nur unsichtbare Platzhalter-Buttons, und die Texte verwiesen auf Schaltflächen, die niemand sehen konnte.

**Geändert:** Der Steam-Dienst liefert seine Buttons jetzt mit Beschriftung und Stil mit, und die Brücke baut daraus echte Discord-Schaltflächen an jeder Antwort. Die Panel-Buttons („🎟️ Einladung starten", „🔗 Steam verknüpfen", „🔢 Freundescode eingeben", „📊 Rang prüfen") haben sichtbare Beschriftungen bekommen.

**Betroffen:** Alle, die den Einladungs-Flow oder das Steam-Panel nutzen — der Ablauf ist jetzt klickbar statt rätselhaft. Bereits gepostete Panels zeigen die neuen Beschriftungen erst nach einem Neu-Posten der Panels.

## #67 — Steam-Austritts-Ban wieder entfernt + Reparatur-Befehl durchgereicht

**Ausgangslage:** Mit #66 hatte der interne Vermittler eine Ban-Route bekommen, damit der Steam-Dienst beim Server-Verlassen nach einem Playtest-Invite automatisch bannen kann. Diese Automatik ist auf Community-Entscheidung wieder gestrichen — den Server zu verlassen ist kein Bann-Grund.

**Geändert:** Die Ban-Route ist komplett ausgebaut (der Vermittler kann wieder nur Rollen, Nachrichten, DMs, Kanäle und Invites). Zusätzlich reicht die Steam-Brücke den neuen Admin-Befehl zum erneuten Senden einer Steam-Freundschaftsanfrage an den Steam-Dienst durch — das Werkzeug, mit dem versehentlich gekündigte Freundschaften (siehe Steam-Bot #22) wieder angeknüpft werden.

**Betroffen:** Niemand verliert etwas — gebannt wegen Austritt wird nicht mehr, und Admins haben einen Reparatur-Befehl mehr.

## #66 — Steam-Austritts-Ban wieder möglich (Broker-Ban-Route)

**Ausgangslage:** Der neue Steam-Dienst sollte beim Verlassen des Servers nach einem Playtest-Invite wieder bannen können (Anti-Missbrauch gegen Invite-Farming) — aber der interne Vermittler, über den der Dienst Discord-Aktionen ausführt, kannte bisher nur Rollen vergeben/entziehen und DMs, kein Bannen.

**Geändert:** Der interne Vermittler bekommt eine Ban-Route. Sie bannt per User-ID und funktioniert deshalb auch dann, wenn die Person den Server schon verlassen hat. Auth, Idempotenz und Server-Allowlist laufen wie bei den bestehenden Rollen-Routen.

**Betroffen:** Server-Moderation — der automatische Ban beim Verlassen nach einem Invite greift wieder (die eigentliche Entscheidung trifft der Steam-Dienst).

## #65 — Steam-Slash-Commands wieder verfügbar

**Ausgangslage:** Seit der Steam-Umstellung (Eintrag #62) fehlten die Slash-Commands rund um Steam — die alten Bausteine, die sie bereitstellten, sind abgeschaltet. Die Verknüpfungs-Buttons im Panel liefen weiter, aber Befehle wie `/account_verknüpfen`, `/steam links` oder `/checkrank` waren weg.

**Geändert:** Der dünne Steam-Vermittler im Bot registriert die Commands jetzt selbst und leitet sie an den neuen Steam-Dienst weiter, der die eigentliche Arbeit macht. Wieder da: `/account_verknüpfen`, `/steam links`, `/steam whoami`, `/steam setprimary`, `/steam unlink`, `/steam_rank`, `/checkrank` sowie die Admin-Befehle `/steam_rank_sync`, `/subrank_sync`, `/sync_steam_friends` und `/publish_steam_panel`.

**Wie's funktioniert:** Jeder Command schickt Name und Argumente in einem festen Format an den Steam-Dienst und rendert dessen Antwort (Text, Embed oder Login-Button). Befehle mit Eingabe (z. B. eine SteamID bei `/steam whoami`) übergeben diese mit; `/checkrank` löst die @-Mention vorher zum Discord-User auf. Die langlaufenden Admin-Syncs über die ganze Freundesliste bekommen ein größeres Zeitlimit und melden sich erst, wenn der Dienst fertig ist, statt vorzeitig abzubrechen.

**Betroffen:** Alle, die ihren Steam-Account verwalten oder ihren Rang prüfen, sowie Admins, die einen Sofort-Sync auslösen.

## #64 — Voice-Nudge-DM repariert: Steam-Link-Button funktioniert wieder

**Ausgangslage:** Seit der Steam-Umstellung (Eintrag #62) holte sich die freundliche Erinnerungs-DM ("verknüpf doch mal deinen Steam-Account"), die nach 30 Minuten im Voice verschickt wird, ihre Login-URL noch über den alten Weg: Sie suchte sich zur Laufzeit das passende Steam-Cog im Bot zusammen. Genau dieses Cog war aber abgeschaltet. Die Suche fand stattdessen den Nudge-Baustein selbst und lief dann beim Erzeugen der URL in einen Fehler — die DM kam entweder ohne funktionierenden Button oder gar nicht.

**Geändert:** Die Erinnerungs-DM und der zugehörige Health-Check fragen jetzt direkt den neuen Steam-Dienst nach einer frischen Login-URL, statt im Bot nach einem Cog zu suchen.

**Wie's funktioniert:**
- Es gibt jetzt eine zentrale Stelle, die beim Steam-Dienst einen Einmal-Link anfordert (15 Minuten gültig) — denselben, den auch der Steam-Verknüpfen-Button im Server-Panel erzeugt. Die Nudge-DM nutzt genau diese Stelle, dadurch gibt es keinen zweiten, abweichenden Weg mehr, der kaputtgehen kann.
- Der stündliche Selbsttest des Bots prüfte bislang, ob das alte (jetzt abgeschaltete) Steam-Modul geladen ist, und meldete deshalb dauerhaft einen Fehlalarm. Er prüft jetzt stattdessen, ob der neue Steam-Dienst auf seinem Health-Endpunkt antwortet.
- Der nicht mehr benötigte Watchdog der alten Steam-Bridge wurde abgeschaltet.

**Betroffen:** Alle, die nach längerer Zeit im Voice die Steam-Verknüpfungs-Erinnerung bekommen — der Button im DM führt wieder zuverlässig zum Login.

## #63 — Coaching: Coach-Roster-Sync zur Website + Termin-DMs

**Ausgangslage:** Die Coaching-Website wusste nicht, wer eigentlich Coach ist — sie kannte nur die Coaches, die zufällig schon einmal eine Session gespiegelt hatten. Und für die neuen Coaching-Termine der Website gab es keinen Weg, Spieler in Discord zu benachrichtigen.

**Geändert:** Ein neuer Baustein hält Discord und Website synchron: Er meldet alle Träger der Coach-Rolle an die Website und stellt Termin-Benachrichtigungen als DM zu.

**Wie's funktioniert:**
- **Rollen-Sync:** Beim Start, alle 10 Minuten und sofort bei jeder Rollenänderung (mit 5-Sekunden-Sammelfenster, damit mehrere Änderungen kurz hintereinander nur einen Sync auslösen) wird die komplette Coach-Liste mit Namen und Avataren übertragen. Leere Listen werden nie gesendet — Schutz davor, das Website-Roster versehentlich zu leeren. Ist der Mitglieder-Cache direkt nach dem Start noch leer, wird er einmal explizit nachgeladen.
- **Termin-DMs:** Der Baustein fragt die Website im Minutentakt nach fälligen Benachrichtigungen und schickt dem Spieler je nach Typ eine DM — Einladung bei Terminanlage, Erinnerung unter 2 Stunden vor Start, Absage-Info. Zeiten werden in deutscher Zeit formatiert. Hat ein Spieler DMs deaktiviert, wird die Benachrichtigung als erledigt markiert statt endlos neu versucht; bei Netzwerkfehlern wird sie beim nächsten Durchlauf erneut zugestellt.

## #62 — Steam-Umschaltung: alte Steam-Bausteine deaktiviert

Die Umschaltung auf den Rust-Steam-Dienst ist vollzogen: Über die Cog-Blockliste sind alle neun alten Steam-Bausteine (Verknüpfung, Freundes-Abgleich, Rang-Rollen, Aufräumer, Playtest-Trichter, Guard-Automatik, Token-Verwaltung) deaktiviert — der Code bleibt unangetastet liegen und kann im Notfall mit einer Zeile reaktiviert werden. Discord-seitig übernimmt der schlanke Brücken-Cog (#60/#61); die gesamte Logik läuft im Rust-Dienst.

Der erste Live-Abgleich unter neuer Führung: 284 Steam-Freunde geprüft, 281 Verknüpfungen bestätigt, keine fälschlich entfernt. Der stündliche Verified-Rollen-Abgleich und die Voice-Erinnerung laufen unverändert weiter — sie kollidieren nicht mit dem neuen Dienst.

## #61 — Steam-Brücke: Playtest-Funnel-UI + einheitliches Event-Format

Nachtrag zu #60: Der Brücken-Cog kann jetzt auch den Playtest-Einladungs-Ablauf bedienen — drei Slash-Befehle (/betainvite für alle, Panel-Veröffentlichung und Statistik für Admins) und alle Funnel-Buttons werden als Ereignisse an den Rust-Dienst weitergereicht, der sämtliche Texte und Entscheidungen liefert. Das Panel hängt seine Buttons weiterhin lokal an, damit sie nach Bot-Neustarts klickbar bleiben.

Außerdem wurde das Übertragungsformat zwischen Brücke und Rust-Dienst vereinheitlicht: Alle Ereignis-Arten (Klick, Server-Verlassen, Admin-Befehl, Slash-Befehl) senden ihre Daten jetzt in derselben verschachtelten Struktur. Vorher nutzten drei der vier Arten ein abweichendes flaches Format — die Gegenseite hätte sie kommentarlos abgelehnt.

## #60 — Discord-Arm für den Rust-Steam-Bot: Broker erweitert + neuer Brücken-Cog

Der Steam-Bot zieht nach Rust um — Discord bleibt aber beim Haupt-Bot. Damit der Rust-Dienst alle nötigen Discord-Aktionen auslösen kann, wurde der interne Vermittler (Master-Broker) um zwei Operationen erweitert: Rolle entfernen und Direktnachricht senden — beide mit derselben Absicherung wie die bestehenden Operationen (Token-Pflicht, Wiederholungsschutz über Idempotenz-Schlüssel, Guild-/Rollen-Whitelists).

Neu dazu kommt ein bewusst dünner Brücken-Cog: Er rendert die Steam-Verknüpfungs-Panels (auch ältere, bereits gepostete bleiben klickbar), öffnet das Freundescode-Eingabefenster, leitet jeden Klick, jedes Server-Verlassen und die Steam-Admin-Befehle als Ereignis an den Rust-Dienst weiter und zeigt dessen Antwort an. Er enthält selbst keinerlei Steam-Logik — fällt der Rust-Dienst aus, antwortet er mit einem freundlichen Hinweis statt zu crashen.

Aktiv wird das Ganze erst beim Umschalten: Dann werden die neun alten Steam-Bausteine über die Blockliste deaktiviert und der Rust-Dienst übernimmt.

## #59 — Coaching-Modal: Interaction-Timeout-Fix (Defer vor DB-Calls)

**Ausgangslage:** Wenn ein User das Coaching-Formular ausgefüllt und auf „Absenden" gedrückt hat, kam manchmal die Fehlermeldung „Beim Absenden der Anfrage ist ein Fehler aufgetreten". Das passierte weil der Bot nach dem Modal-Submit zunächst eine synchrone DB-Operation (`INSERT INTO coaching_requests`) auf dem Event-Loop ausgeführt hat — ohne die Discord-Interaction vorher zu bestätigen. Discord erwartet eine Antwort innerhalb von 3 Sekunden; wenn der DB-Call (z.B. durch Lock-Contention mit dem Background-Analyse-Task) auch nur kurz blockiert, läuft der Interaction-Token ab und jede nachfolgende `send_message`-Antwort schlägt mit 404 fehl.

**Geändert:** Der `on_submit`-Handler ruft jetzt sofort `interaction.response.defer(ephemeral=True)` auf, bevor irgendeine DB-Arbeit beginnt. Damit ist das 3-Sekunden-Fenster gesichert und der Bot hat anschließend bis zu 15 Minuten Zeit für die eigentliche Verarbeitung. Die Erfolgs- und Fehlermeldungen werden nun via `interaction.followup.send()` geschickt statt `interaction.response.send_message()`.

**Ergebnis:** Das Formular nimmt Anfragen zuverlässig entgegen, egal ob die DB kurz ausgelastet ist oder ein paralleler Analyse-Task läuft.

## #58 — Admin-Session-Validierung: Sliding TTL beim Cross-Dashboard-Check

**Ausgangslage:** Der interne `validate-session`-Endpoint (den der Twitch-Bot nutzt, um zu prüfen ob eine aktive Admin-Session im Discord-Bot existiert) griff direkt auf das Session-Dict zu und machte die Ablauf-Prüfung manuell. Dabei wurde die Session-Laufzeit weder verlängert noch der zentrale Cleanup-Pfad durchlaufen — jeder Zugriff lief am eigentlichen Session-Management vorbei.

**Geändert:** Der Handler nutzt jetzt `validate_discord_session()` statt des direkten Dict-Lookups. Das ist dieselbe Methode, die auch alle anderen Session-Zugriffspfade (Browser-Request, CSRF-Check etc.) verwenden.

**Wie es jetzt funktioniert:** Jede erfolgreiche Cross-Dashboard-Validierung verlängert die Session-Laufzeit wie ein normaler Zugriff. Gleichzeitig werden abgelaufene Einträge über den einheitlichen Cleanup-Pfad entfernt, statt still im Dict zu bleiben.

## #57 — Lobby-Finder: Channel-Name "unbekannt" gefixt + Trigger-Rauschen reduziert

**Ausgangslage:** Der Lobby-Finder zeigte in seinen Antworten statt der echten "Neue Spieler Lane" den Platzhalter "# unbekannt". Außerdem feuerte er auf Nachrichten wie "suche leute zum zocken, schreib mir bitte priv" — eine Ankündigung, keine Lobby-Anfrage.

**Geändert:** Die Channel-ID für die Neue-Spieler-Lane war in `lfg.py` noch auf den alten Wert von vor der adaptiven-Lane-Einführung gesetzt (`1465839460485697556`), während der tatsächliche Anchor-Channel eine andere ID hat (`1470126503252721845`). ID korrigiert. Zusätzlich wurde ein Negativ-Signal ganz an den Anfang der Intent-Erkennung gelegt: Nachrichten, die gleichzeitig "schreib/schreibt/meldet" und "priv/privat" enthalten, werden jetzt sofort übersprungen — der User will selbst koordinieren, nicht vom Bot weitergeleitet werden.

**Jetzt:** Channel-Mention zeigt den richtigen Namen. Nachrichten mit explizitem "schreib priv"-Muster lösen keine Lobby-Vorschläge mehr aus.

## #56 — Dependency-Updates: aiohttp 3.14.0 + protobufjs-Lücken geschlossen

**Ausgangslage:** Dependabot meldete 11 offene Sicherheitslücken: 4× aiohttp (medium, Cross-Origin-Redirect + Deserialisierung unsicherer Daten) und 7× protobufjs (bis high, u.a. Code-Injection via Byte-Felder, Prototype-Pollution, DoS durch rekursive Expansion).

**Geändert:** aiohttp von 3.13.5 auf 3.14.0 angehoben (`requirements.txt` + `.venv`). protobufjs im steam_presence-Modul via `npm install` + `npm audit fix` aktualisiert: Top-Level-Package von 8.0.2 auf 8.4.2, zusätzlich die verschachtelten Kopien in den transitiven Abhängigkeiten (`steam-user`, `steam-session`, `steam-appticket`) auf gepatchte Versionen gebracht.

**Ergebnis:** `npm audit` meldet 0 Vulnerabilities. Bot läuft weiter ohne Verhaltensänderung.

## #55 — Coaching: Freigeben-Button-Fix + echte Umlaute

**Ausgangslage:** Der „Freigeben"-Button im Coaching-Request-Embed hatte zwei Bugs. Erstens: Wenn eine Anfrage bereits für alle offen war (kein reservierter Coach), bekamen Nicht-Server-Owner die Meldung „Nur der reservierte Coach oder ein Admin kann freigeben" — statt der korrekten Info „bereits für alle offen". Die eigentliche Info-Meldung war für alle außer dem Guild-Owner toter Code. Zweitens: Als „Admin" galten ausschließlich der Guild-Owner und eine hardcoded User-ID — Discord-User mit `Administrator`-Permission wurden geblockt, obwohl die Fehlermeldung selbst von „ein Admin" spricht.

**Geändert:** Logik-Reihenfolge korrigiert: Der „bereits offen"-Check läuft jetzt zuerst, bevor der Permission-Check greift. Außerdem wurde `guild_permissions.administrator` in den Admin-Check aufgenommen, konsistent mit dem restlichen Codebase-Pattern (`website_invite_cog._is_owner_or_admin`). Zusätzlich: Alle 14 Pseudo-Umlaute (`fuer`, `ue`, `oe` etc.) in der Datei durch echte Umlaute ersetzt.

**Wie's funktioniert:** Klickt jemand Freigeben auf einer bereits offenen Anfrage, sieht er jetzt die korrekte Info — unabhängig von seiner Rolle. Klickt ein Discord-Admin (mit `Administrator`-Flag) auf eine reservierte Anfrage, kann er sie freigeben. Der assigned Coach kann weiterhin immer freigeben.

## #54 — Coaching: Session-Abschluss wird jetzt an die Website gespiegelt

**Ausgangslage:** Wenn ein Admin mit `/coaching-session-beenden` eine Session beendete, landete das nur in der Bot-DB. Die Coaching-Website zeigte weiterhin die Session als aktiv — weil der Bot nie den `/platform/sync`-Endpunkt aufrief.

**Geändert:** Der `coaching_survey`-Cog sendet nach dem Session-Update jetzt automatisch einen HTTP-Call an das Website-Backend (`/platform/sync` auf Port 8772). Payload enthält Request-ID, Discord-IDs, Coach-Name und `session_status: "completed"` + die Bot-Session-UUID.

**Wie's funktioniert:** Der interne Token (derselbe wie im restlichen Stack) authentifiziert den Call. Das Backend macht ein Upsert: Session wird auf `completed` gesetzt, der Timestamp landet in `completed_at`. Damit sehen Spieler auf `/coaching/me` ihre Session-Historie korrekt — inklusive dem Abschluss. Der Coaching-Flow in Discord bleibt unverändert.

## #53 — Master-Broker: Discord-Invites auf Anfrage erstellen

**Hintergrund:** Der Twitch-Bot läuft als separater Prozess und hat keinen eigenen Discord-Guild-Mitgliedsstatus — er kann daher keine Invites direkt über die Discord-API erstellen. Gelöst über den Master-Broker, der bereits für andere Discord-Aktionen (Nachrichten senden, Channels anlegen, Rollen vergeben) als interner Proxy fungiert.

**Geändert:** Neuer Endpunkt `POST /internal/master/v1/discord/create-invite` im Master-Broker. Nimmt `channel_id` und optionalen `reason`, löst den Channel über den Haupt-Bot auf und ruft `channel.create_invite(max_age=0, max_uses=0, unique=True)` auf. Antwort enthält `invite_url`, `code`, `channel_id` und `guild_id`. Auth und Idempotency-Handling laufen identisch zu den anderen Broker-Endpunkten.

## #52 — Fix: TempVoice-Interface lädt wieder korrekt

**Problem:** Durch die neuen Router-Buttons (Umbenennen + Modus wechseln) kam es beim Bot-Start zu einem Fehler, der das komplette TempVoice-Modul am Laden hinderte — alle Lane-Funktionen waren damit ausgefallen.

**Ursache:** Der Lurker-Button hat intern row 3 als Standard, was die Reihe bereits auf 4 Items brachte. Die zwei neuen Buttons auf row 3 machten 6 Items — Discord erlaubt maximal 5 pro Reihe.

**Geändert:** Die neuen Router-Buttons landen jetzt auf row 4, die in der Standard-Lane-Ansicht bisher leer war.

## #51 — Router: Smart-Routing bevorzugt bekannte Mitspieler

**Problem:** Der Router hat beim Suchen einer freien Lane einfach die erste passende genommen — ohne Rücksicht darauf, ob da jemand drin sitzt, mit dem man schon oft gespielt hat.

**Geändert:** Beim Smart-Routing (Auto-Join an) wird jetzt zuerst geprüft, ob in einer der passenden Lanes ein bekannter Mitspieler sitzt. Erst wenn keine solche Lane gefunden wird, greift der bisherige Fallback (erste freie Lane mit Platz).

**Wie's funktioniert:** Beim Join holt der Router die Top-20-Co-Player aus der bestehenden `user_co_players`-Tabelle (die trackt, wie viele gemeinsame Voice-Sessions zwei User hatten). Dann werden alle passenden Lanes durchsucht: Liegt die User-ID eines Mitglieds in dieser Co-Player-Liste, wird diese Lane priorisiert. Findet sich kein Co-Player, landet man wie gewohnt in der nächsten freien Lane.

## #50 — TempVoice: Neues Router-System mit Spielmodus-Wahl

**Problem:** Wer in einen Sprachkanal wollte, landete in einem von drei fixen Staging-Kanälen (Casual, Ranked, Street Brawl) — Modus-Wahl durch das Betreten des richtigen Kanals. Das war unflexibel: keine Möglichkeit den Modus zu ändern, keine smarte Verteilung in laufende Lanes, keine einheitliche Einstiegsstelle.

**Geändert:** Ein einzelner Router-Sprachkanal ersetzt die drei Eingänge für alle, die über diesen neuen Weg kommen wollen. Die alten Staging-Kanäle laufen unverändert weiter. Dazu gibt es einen Interface-Textkanal mit einer persistenten UI aus drei Modus-Buttons (Casual, Ranked, Street Brawl) und einem Auto-Join-Toggle.

**Wie's funktioniert:**

- **Modus wählen:** User klickt im Textkanal auf einen der drei Buttons — wird als Standard gespeichert und gilt für alle folgenden Joins.
- **Auto-Join aus (grau):** Beim Betreten des Router-VC wird sofort eine eigene private Lane im passenden Bereich erstellt.
- **Auto-Join an (grün):** Smart Routing — der Bot sucht eine bestehende Lane mit weniger als 6 Personen und demselben Modus. Findet er keine, wird eine neue erstellt.
- **Ranked-Gate:** Wer Ranked wählt, braucht einen verifizierten Rang (Steam-Verknüpfung). Ohne Rang kommt eine DM mit Link zum Info-Kanal, kein Move.
- **Neue Spieler:** Werden weiterhin automatisch in die New-Player-Lane umgeleitet, bevor das Modus-Routing greift — bestehende Logik unverändert.
- **Modus-Wechsel einer laufenden Lane:** Lane-Owner können ihren Kanal nachträglich auf einen anderen Modus (inkl. Off Topic) umstellen — der Kanal zieht in die passende Discord-Kategorie um. Dazu gibt es neue Buttons im Lane-Control-Interface.
- **Off-Topic-Lanes** landen in der Router-Kategorie, genau wie der bestehende permanente Off-Topic-Kanal.

## #49 — Coaching wird jetzt fair auf alle Coaches verteilt

**Problem:** Eingehende Coaching-Anfragen liefen nach dem „Wer zuerst klickt"-Prinzip — der erste Coach, der auf „Claimen" drückte, bekam die Session. In der Praxis griff dadurch meist immer derselbe Coach zu, während andere kaum drankamen; eine echte Verteilung gab es nicht. Zusätzlich lief im Hintergrund noch ein altes, längst totes Zweit-System mit fest eingetragener Coach-Liste mit, das die Lage nur unübersichtlicher machte.

**Geändert:** Neue Anfragen werden jetzt automatisch **fair** einem Coach zugewiesen und **24 Stunden exklusiv für ihn reserviert**. Es gibt einen **Freigeben-Button**, und nach Ablauf der 24 Stunden öffnet sich die Anfrage automatisch für alle. Wer Coach ist, ergibt sich dynamisch aus der Coach-Rolle — keine fest hinterlegte Namensliste mehr. Das alte parallele Claim-System wurde komplett entfernt.

**Wie's funktioniert:** Sobald eine Anfrage analysiert und gepostet wird, wählt der Bot aus allen Trägern der Coach-Rolle (der Server-Inhaber ist bewusst ausgenommen) den aus, der **am längsten nicht mehr an der Reihe war** — bei Gleichstand den mit den **wenigsten laufenden Sessions**. Dieser Coach steht sichtbar im Embed und hat 24 Stunden exklusiv Zeit zu übernehmen; andere Coaches sehen die Anfrage, können sie in diesem Fenster aber nicht claimen. Drückt der reservierte Coach (oder ein Admin) auf „Freigeben", oder laufen die 24 Stunden ab, wird die Anfrage für alle Coaches geöffnet — ein Hintergrund-Check im Minutentakt erledigt das Ablaufen automatisch und aktualisiert die Nachricht. Lässt sich gerade kein passender Coach ermitteln, ist die Anfrage sofort für alle offen, genau wie vorher.

**Betroffen:** Vor allem die Coaches (faire Reihenfolge statt Windhundprinzip). Für anfragende Spieler bleibt der Ablauf gleich — nur ist schneller klar, wer sich kümmert.

## #48 — Security Guard: MiniMax-Label erscheint jetzt nachträglich im Mod-Embed

**Problem:** Das MiniMax-Bild-Urteil lief blockierend *vor* dem Mod-Alert — wenn MiniMax länger brauchte (oder per Timeout abbrach bei 8 s), stand im ersten Post „nicht verfügbar" und der Bot wartete die ganze Zeit, bevor er überhaupt Timeout und Löschung ausführte.

**Geändert:** Alert wird sofort gepostet (mit „⏳ wird ermittelt…" als Platzhalter), Timeout + Löschung laufen direkt danach. MiniMax läuft als Hintergrund-Task mit 45 s Spielraum und aktualisiert das Embed per Edit, sobald das Urteil vorliegt.

**Wie's funktioniert:** `asyncio.create_task` startet den AI-Check entkoppelt vom Hauptpfad. Sobald MiniMax antwortet, sucht die Task das Feld „MiniMax-Einschätzung" im bereits geposteten Embed und überschreibt es. Schlägt das Edit fehl (Nachricht gelöscht, Bot-Rechte weg), wird das still ignoriert.

## #47 — Security Guard: Timeout-Bug gefixt, AI-Scam-Erkennung repariert, öffentliche Scam-Meldung

**Problem:** Der Security Guard hat Scam-Accounts zwar erkannt und den Alert im Mod-Channel gepostet, aber keine einzige Aktion ausgeführt — kein Timeout, keine Nachrichtenlöschung. Der Bot hat den Mod-Alert sogar doppelt gepostet (der Case erschien zweimal), weil der Crash den State zurückgesetzt und beim nächsten Trigger erneut ausgelöst hat. Dazu hat die AI immer "kein Scam" zurückgegeben, obwohl MiniMax intern bereits 100 % Scam erkannt hatte.

**Was war kaputt:**
- py-cord 2.7.1 kennt den Parameter `timed_out_until`, nicht `communication_disabled_until` (discord.py-mainline-Name). Jeder Timeout-Aufruf ist mit einem `TypeError` abgestürzt — ungebated, bis zu `discord.client` hochgereicht, wo er stillschweigend weggeloggt wurde. Weder Timeout noch Nachrichtenlöschung (die danach kamen) liefen je.
- `max_output_tokens=120` beim Scam-Text-Check war zu niedrig für Thinking-Modelle: MiniMax M3 hat alle 120 Token für den internen Denkprozess verbraucht, kein JSON-Response überlebt — die AI hat daher immer `False` zurückgegeben, egal wie eindeutig der Scam war.

**Geändert:**
- Timeout-Aufruf auf `timed_out_until` umgestellt (überall, inkl. Timeout-Aufhebung per Button).
- `max_output_tokens` für Scam-Text-Check von 120 auf 1500 angehoben, damit Thinking + JSON beide reinpassen.
- Nach jeder automatischen Scam-Aktion (Ban oder Timeout) postet der Bot jetzt kurz in den betroffenen Channel(s): `🔒 Scam erkannt — Account wurde automatisch gebannt/gesperrt.` — so sehen User, die den Scam gesehen haben, dass das Mod-System reagiert hat.

**Wie's jetzt funktioniert:** Scam erkannt → Beweis in Mod-Channel spiegeln → Timeout setzen (funktioniert jetzt) → Nachrichten löschen → öffentliche Kurzmeldung im betroffenen Channel → DM an User. Der AI-Text-Check liefert jetzt bei eindeutigen Scams (wie Fake-MrBeast-Crypto oder USDT-Withdrawal-Screenshots) korrekt `is_scam: true` mit hoher Confidence.

## #46 — Master-Dashboard: Login hält jetzt 2 Wochen statt 6 Stunden

**Problem:** Die Anmeldung am Master-Dashboard galt nur 6 Stunden. Da der Twitch-Admin-Bereich auf dieser zentralen Sitzung aufsetzt, fiel man dort regelmäßig nach wenigen Stunden raus — das Dashboard wirkte „tot": Oberfläche lädt, aber jede Aktion läuft ins Leere, weil die Sitzung im Hintergrund abgelaufen war.

**Geändert:** Die Standard-Lebensdauer der Master-Dashboard-Sitzung von 6 Stunden auf 14 Tage angehoben.

**Wie's funktioniert:** Die Sitzung gilt jetzt zwei Wochen und verlängert sich bei jeder Nutzung automatisch (Sliding-Refresh) — wer regelmäßig reinschaut, bleibt praktisch dauerhaft angemeldet. Der Wert ist weiterhin per Umgebungsvariable überschreibbar; der neue Standard greift, solange nichts anderes gesetzt ist. Einmal anmelden reicht damit für zwei Wochen statt mehrmals täglich neu einloggen zu müssen.

**Betroffen:** Login-Komfort im Admin-/Dashboard-Bereich; für normale Discord-Nutzer nichts sichtbar.

## #45 — Crypto-Scam: Jetzt automatisch gelöscht + Ban-Button für Mods

**Problem:** Ein Einzel-Scam (eine Nachricht, ein Kanal) wurde vom Bot erkannt (Confidence 0,97), aber weder automatisch gelöscht noch direkt gesperrt — weil er in keine der Auto-Delete-Kategorien fiel. Der Moderator konnte anschließend auch nicht direkt über den Bot bannen, da "Accept" nur einen 24h-Timeout ausgelöst und keine Ban-Option angeboten hat.

**Geändert:** `scam` als vollständige Kategorie im KI-Moderator eingeführt: im System-Prompt beschrieben, in die erlaubten Kategorien aufgenommen und in die Auto-Delete-Liste eingetragen. Zusätzlich ist ein neuer **Ban-Button** in jede Moderations-Review-Nachricht eingebaut worden.

**Wie's jetzt funktioniert:** Stuft die KI eine Nachricht mit `category=scam` und `verdict=delete` bei Confidence ≥ 0,90 ein, wird sie sofort gelöscht und der User automatisch 24h stummgeschaltet — ohne Mod-Interaktion. Liegt die Confidence zwischen 0,78 und 0,90, landet ein Vorschlag im Review-Kanal mit drei Buttons: **Accept** (Nachricht löschen + Timeout), **Ban** (Nachricht löschen + permanenter Serverausschluss) und **Deny** (Fall ablehnen). Der Ban-Pfad greift auch bei allen anderen Kategorien — nicht nur bei Scam.

## #44 — Gekaperte Stamm-Accounts: Scam-Bilder werden gestoppt

**Problem:** Ein langjähriges, etabliertes Mitglied wurde gehackt und hat in Sekunden Krypto-/Casino-Scam-Screenshots (gefälschte Auszahlungs-„Beweise", ein Fake-Promi-Giveaway, eine Casino-Bonusseite) über fünf, sechs Kanäle gestreut. Der Bot hat nicht reagiert — aus zwei Gründen. Erstens: Das harte Durchgreifen aus #43 galt nur für neue Accounts unter 30 Tagen; ein gekapertes Alt-Mitglied fiel komplett durch dieses Raster. Zweitens: Der Bild-Check hing an einer KI, die die Bilder technisch gar nicht „sehen" kann — das hier eingesetzte Modell ist reiner Text. Die in #43 erwähnte Bild-Mitlesung lief faktisch ins Leere, das Bild-Urteil stand deshalb immer auf „0 %".

**Geändert:** Ein neuer Schnellpfad speziell gegen Account-Übernahmen, der für **alle** Mitglieder gilt — ausdrücklich auch alte, etablierte (nur Mods sind ausgenommen). Und die Bilder werden jetzt von einem echten Bild-Verstehen-Dienst gelesen statt vom blinden Text-Modell.

**Wie's funktioniert:** Postet ein Account innerhalb von **30 Sekunden Bilder in mindestens zwei verschiedenen Kanälen** — der typische Fingerabdruck einer übernommenen Identität, die Werbung streut — greift der Bot sofort durch, ohne auf ein KI-Urteil zu warten (deterministisch, deshalb auch unabhängig davon, ob die KI etwas erkennt). Reihenfolge: erst die Bilder als Beweis in den Mod-Kanal kopieren (solange die Links noch leben), dann den Account **24 Stunden stummschalten** und sämtliche dieser Nachrichten kanalübergreifend löschen. Der Betroffene bekommt eine DM mit dem Hinweis, sein Account sei womöglich gehackt — Passwort ändern, 2FA aktivieren, beim Mod-Team melden. Die Mods sehen eine Übersicht mit den Bildern und zwei Buttons: **Bann** (endgültig) oder **Stummschaltung aufheben** (Fehlalarm). Die 24 Stunden sind bewusst umkehrbar gehalten.

**Warum die KI jetzt wirklich mitliest:** Zusätzlich schickt der Bot ein repräsentatives Bild durch ein echtes Bild-Verstehen. Das liefert den Mods jetzt ein konkretes Urteil im Alarm — z. B. „Scam, 100 % — gefälschtes Promi-Konto bewirbt ein Krypto-Casino-Giveaway" — statt der bisherigen stummen „0 %", weil das alte Modell die Bilder nie sehen konnte. Wichtig: Dieses Urteil ist nur noch ein Hinweis fürs Mod-Team, kein Auslöser — die Quarantäne läuft auch dann, wenn die KI gerade nichts sagt.

## #43 — Spam-Schutz greift jetzt selbst durch

**Problem:** Wenn ein neuer Account (jünger als 30 Tage) in kurzer Zeit dieselbe Werbung oder dieselben Bilder über mehrere Kanäle streut — das klassische Spam-Wellen-Muster — hat der Bot das zwar erkannt, aber nur den Mods gemeldet und dann gewartet. Die Spam-Nachrichten blieben stehen, bis jemand von Hand eingriff. Erkennung lief, Reaktion nicht.

**Geändert:** Der Bot greift jetzt beim Muster selbst durch, statt auf eine KI-Bestätigung zu warten — die kam vorher nämlich oft gar nicht (siehe unten).

**Wie's funktioniert:** Schlägt das Muster an (ab 3 Nachrichten in 3 Kanälen, oder schon ab 2 Kanälen mit Bildern/Scam-Wörtern; Mods und etablierte Accounts ausgenommen), läuft der Reihe nach: erst die Bilder als Beweis in den Mod-Kanal kopieren (bevor die Links durchs Löschen tot sind), dann die Nachrichten löschen und den Account **1 Stunde stummschalten**. Die Mods bekommen eine Übersicht mit Texten, Bildern und zwei Buttons — **Bann** oder **Timeout aufheben**. Die Stunde ist bewusst kurz und umkehrbar, falls es doch ein harmloser Neuling war. Die KI prüft Text und Bilder weiterhin, aber nur noch als Hinweis fürs Mod-Team, nicht als Auslöser.

**Warum vorher „0 %" dastand:** Die Text-Prüfung lief gegen einen KI-Dienst, der auf diesem Bot gar nicht eingerichtet ist (Zugangsschlüssel fehlt) — es kam nie eine Antwort zurück, und „keine Antwort" wurde als 0 % angezeigt. Jetzt läuft die Prüfung über den Dienst, der hier wirklich konfiguriert ist und auch die Bilder mitliest. Dessen interne „Denk"-Abschnitte werden vor der Auswertung herausgefiltert, weil das Ergebnis sonst beim Einlesen zerbrach.

## #42 — Direkte Channel-ID für interne API-Posts

- Interne Bot-Posts können jetzt einen beliebigen Discord-Kanal direkt ansprechen (statt nur "all"/"twitch")
- Wird genutzt für Moderations-Alerts aus dem Twitch-Bot

## #41 — Changelog-Posts direkt in Discord

- Neue Changelogs landen jetzt automatisch als Discord-Embed im Dev-Update-Kanal
- Twitch-Bot-Änderungen können zusätzlich gezielt im Twitch-Bot-Kanal gepostet werden
- Admin-interne Änderungen werden dabei bewusst weggelassen – nur was User sehen sollen kommt rein
- Admins können Einträge auch per `/changelog post` manuell auslösen

## #40 — Steam-Verifikation: Watchdog gegen stille Disconnects

- Ein neuer automatischer Watchdog überwacht jetzt dauerhaft ob der Steam-Bot eingeloggt ist
- Fällt die Steam-Verbindung lautlos weg (wie heute ~16 Uhr), erkennt der Watchdog das innerhalb von 30 Sekunden und startet den Bot nach 2 Minuten automatisch neu
- Vorher konnte eine solche Verbindungsunterbrechung unbemerkt über eine Stunde anhalten, weshalb Steam-Verifikationen in diesem Zeitraum fehlschlugen
- Betroffene User können die Verknüpfung jetzt erneut starten – sie wird wieder funktionieren

## #39 — FAQ-Bot kennt jetzt fast alle Server-Themen

- Der FAQ-Bot kann jetzt deutlich mehr Fragen beantworten: TempVoice und Lanes, Coaching, Onboarding & Beta-Invite, Steam-Verknüpfung, Twitch-Dashboard inkl. Tarife, Turniere, Patchnotes-Bot und unsere Websites
- Antworten zu Preisen und kostenlosen vs. bezahlten Features sind jetzt verlässlich, weil die zugrundeliegende Doku konkret und aktuell ist
- Veraltete, sich überschneidende Doku wurde archiviert, damit der Bot keine widersprüchlichen Aussagen mehr mischt

## #38 — Coaching-Infos überarbeitet und bereinigt

- Coaching ist jetzt überall klar als kostenlos und ohne Limit beschrieben
- Hinweis auf Feedback nach dem Coaching eingefügt – User werden gebeten ehrlich zu antworten
- Technische Details (Rollen-Zuweisung etc.) aus den Nutzer-Infos entfernt
- KI-Analyse erstellt keine Priorität mehr, sondern direkt nutzbare Fokuspunkte
- Alle alten Referenzen auf das nicht mehr existierende Coaching-Modul aus den Docs entfernt

## #37 — Coaching-Dokumentation und bessere KI-Antworten

- Der FAQ-Bot weiß jetzt zuverlässig wo und wie Coaching beantragt wird und verweist direkt auf den richtigen Channel
- Neue Coaching-Dokumentation hinterlegt, damit der Bot konkrete Antworten geben kann
- Die KI-Analyse bei Coaching-Anfragen listet jetzt direkt die Fokuspunkte für den Coach – ohne Wiederholung der User-Angaben
- Veraltete Referenz auf den alten Coaching-Bot entfernt

## #36 — Rang-Präferenz für Chill-Lanes

- Neuer Button „🎯 Mein Rang" im TempVoice-Interface (für alle Lane-Typen sichtbar)
- Über ein Dropdown-Menü kannst du Haupt-Rang und Sub-Rang auswählen und speichern
- Wenn du eine Chill-Lane erstellst, wird der gewählte Rang automatisch als Kanalname verwendet
- Dein Sub-Rang bestimmt außerdem die Sortierposition deiner Lane in der Kategorie

## #35 — Leere Sprachkanäle werden jetzt zuverlässig gelöscht

- Leere Lanes in der Ranked-Kategorie werden jetzt korrekt automatisch entfernt
- Channels bleiben nach einem Bot-Neustart nicht mehr dauerhaft erhalten
- Namen von laufenden Lanes werden nach einem Neustart nicht mehr vom Bot überschrieben

## #34 — Tag-Filter-Bestätigung nur noch für dich sichtbar

- Nach dem Speichern des Tag-Filters wird die Bestätigung nur noch dir angezeigt, nicht mehr im Channel

## #33 — Bestimmter Channel bleibt immer ganz unten in der Kategorie

- Ein festgelegter Voice-Channel wird nach jedem Neu-Sortieren automatisch ans Ende der Kategorie verschoben
- Er bleibt immer unter allen anderen Lanes, egal welche Ränge neu erstellt werden

## #32 — Channel aus TempVoice-Verwaltung ausgenommen

- Ein bestimmter Voice-Channel wird vom Bot nicht mehr umbenannt, gelöscht oder verschoben
- Der Channel bleibt weiterhin über die Verwaltungsinterfaces anpassbar

## #31 — Austritts-Umfrage: verstehen, warum Leute gehen

- Wenn jemand den Server verlässt, bekommt er automatisch eine freundliche Nachricht mit einer kurzen Umfrage — die Fragen passen sich an, je nachdem ob jemand neu war oder schon länger aktiv dabei
- Nach der ersten Antwort kommt eine gezielte Nachfrage, damit klarer wird, was genau das Problem war
- Wer ausführlicher Feedback geben will (auch mit Bildern), bekommt einen Link zu einer Feedback-Seite auf der Website
- Neue Auswertung im Admin-Dashboard zeigt, aus welchen Gründen Leute gehen und wie oft geantwortet wird
- Gebannte Mitglieder werden von der Umfrage ausgenommen

## #30 — Lobby-Finder treffsicherer und übersichtlicher

- Passt wirklich eine Lobby zu deinem Rang, wird dir gezielt die eine vorgeschlagen statt einer langen Liste
- Rang-Schnitt einer Lobby zeigt jetzt den echten Rang-Namen (z. B. Oracle) statt kryptischer Kürzel
- Aufgeräumteres Layout mit mehr Abstand, klarem Beitritts-Hinweis und Warnung wenn eine Lobby fast voll ist
- Falsche "über deinem Rang"-Warnung bei Lobbys unter deinem Rang behoben
- Erkennt mehr Nachrichten wie "jemand am start?" oder "noch wer wach?" und sortiert auch unverifizierte Rang-Rollen sauber ein

## #29 — Ehrliche Member-Herkunft im Admin-Dashboard

- Kreis-Diagramm "Wo kommen unsere Member her?" zeigt jetzt einen eigenen Website-Bereich statt Website-Joins in "Persönlich" zu verstecken
- Historische Joins ohne klare Quelle werden nicht mehr als "Twitch" geschätzt, sondern ehrlich als "Unbekannt" markiert
- Pro Website-Subseite gibt es jetzt einen eigenen Discord-Invite-Code (Landing, Streamer, Mitspieler, Coaching, Helden, Guides)
- Hover über Twitch- oder Website-Stück zeigt die Aufschlüsselung pro Streamer bzw. Subseite
- `/website-invite-recreate` rotiert jetzt gezielt nur eine ausgewählte Subseite

## #28 — Alle GitHub Code-Scanning-Alerts behoben

- Sicherheitslücke in der Node.js-Abhängigkeit protobufjs durch Paket-Override geschlossen
- XSS-Gefahr in der Aktivitätsstatistik-Seite behoben (HTML-Escaping für API-Daten)
- Integrity-Attribut für externes Chart.js-CDN-Skript hinzugefügt
- YAML-Syntaxfehler im Auto-Merge-Workflow behoben
- Log-Injection-Schwachstelle im Master-Broker gefixt (sanitisierte Log-Werte)
- Über 50 Code-Quality-Hinweise bereinigt (leere Except-Blöcke, ungenutzte Variablen, Import-Stil)
- 23 bestätigte False Positives (URL-Redirection, Cookie-Injection, SQL-Queries) als solche markiert und geschlossen

## #27 — Dependabot Auto-Merge wieder voll funktionsfähig

- Fehlende Hilfsdatei ergänzt, die den Python-Security-Scanner blockiert hat
- Syntaxfehler in der Auto-Merge-Workflow-Datei behoben (Actionlint-Fehler)
- Alle drei Ursachen, die Dependabot-PRs vom automatischen Merge abgehalten haben, sind beseitigt

## #26 — GitHub Actions Minutenverbrauch stark reduziert

- Neun tägliche Workflows auf wöchentliche oder event-basierte Trigger umgestellt
- Dashboard DAST und Auth-Guardrails laufen jetzt wöchentlich statt täglich
- Security-Scans, Secret-Scanning und Incident-Automation brauchen keinen täglichen Lauf mehr
- Semgrep meldet Findings als Artifact statt den Build zu blockieren

## #25 — SecurityGuard: Kein Auto-Ban ohne AI-Bestätigung

- Jeder Burst-Trigger wird jetzt zuerst von der KI geprüft — kein Ban ohne AI-Bestätigung
- Wenn die KI nicht sicher genug ist, landet der Fall nur als Vorschlag im Mod-Channel (inkl. Ban-Button für manuelle Entscheidung)
- Verhindert Fehlbans bei neuen Accounts die zufällig in mehreren Channels aktiv sind
- Bilder werden ebenfalls AI-geprüft wenn Text alleine nicht ausreicht

## #24 — SecurityGuard: Ban-Logs persistent in DB gespeichert + Review-Channel aktiv

- Jeder automatische Ban/Timeout wird jetzt dauerhaft in der Datenbank gespeichert — Fälle bleiben auch nach Bot-Neustart nachvollziehbar
- Die gespeicherten Daten umfassen Grund, betroffene Channels, Nachrichten-Snippets und Anhang-Anzahl
- Auch KI-erkannte Scam-Fälle bei etablierten Accounts (Timeout-Proposals) werden persistiert
- Ein dedizierter Review-Channel zeigt ab sofort alle Incidents als Embed an

## #23 — KI-Moderationskontext deutlich verbessert

- Kontext-Nachrichten zeigen jetzt relative Zeitstempel (z.B. "[2min ago]") — die KI erkennt ob der Kontext frisch oder veraltet ist
- Nachrichten vom selben User der bewertet wird, sind mit ">>>" markiert — so sieht die KI das Verhaltensmuster des Users klar
- Reply-Chain: wenn jemand auf eine Nachricht antwortet, bekommt die KI die Original-Nachricht direkt mit — Reaktionen werden im richtigen Kontext bewertet
- System-Prompt erklärt jetzt explizit: Reaktion auf eine Provokation wird milder bewertet als die Provokation selbst

## #22 — KI-Moderationsfilter entschärft: weniger False Positives

- Schwellenwert für Moderationsvorschläge von 55% auf 78% Konfidenz angehoben
- Die KI holt jetzt nur noch 12 statt 25 Nachrichten Kontext — verhindert, dass Kontext-Rauschen aus dem Gaming-Channel die Bewertung verfälscht
- System-Prompt präzisiert: Gaming-Klischees (Nationalitäten), kurze Ein-Wort-Antworten und einmalige Beleidigungen werden nicht mehr gemeldet
- Wichtig: Wer schlechtes Verhalten meldet oder kommentiert, wird nicht mehr selbst geflaggt

## #21 — Bild-Scam-Erkennung: KI analysiert Screenshots in mehreren Channels

- Accounts die nur Bilder senden (z.B. gefälschte X-Posts mit Investitions-Gewinnen) werden jetzt erkannt
- Sobald jemand Bilder in 2+ verschiedenen Channels schickt, prüft die KI automatisch ob es Scam ist
- Junge Accounts (<30 Tage) werden bei bestätigtem Bild-Scam direkt gebannt
- Ältere Accounts bekommen einen 24h-Timeout + DM und Mods sehen den Fall im Mod-Channel
- Message-Verlauf wird jetzt für alle Accounts getrackt, nicht nur für neue

## #20 — Klare Grenze: jung = bis 30 Tage, etabliert = älter als 30 Tage

- Accounts bis 30 Tage alt werden bei erkanntem Scam direkt gebannt
- Accounts älter als 30 Tage (möglicherweise gehackt) bekommen stattdessen einen 24h-Timeout und eine DM
- Der KI-Scam-Check läuft jetzt für alle Accounts, nicht mehr nur für neue
- Mehrkanal-Burst-Erkennung bleibt weiterhin auf Accounts unter 30 Tagen begrenzt

## #19 — Etablierte Scam-Accounts werden sofort getimeoutet und per DM informiert

- Accounts die älter sind und möglicherweise gehackt wurden, werden nicht mehr nur gemeldet, sondern sofort für 24h stummgeschaltet
- Der betroffene User bekommt automatisch eine DM: Grund, Dauer, und der Hinweis sich beim Mod-Team zu melden sobald der Account wieder sicher ist
- Mods sehen im Mod-Channel trotzdem ein Info-Embed mit zwei Buttons: "Ban" (eskalieren) oder "Timeout aufheben" (falls False Positive)
- Kein manuelles Bestätigen mehr nötig — der Bot handelt sofort

## #18 — Scam-Erkennung mit KI und erweiterter Account-Prüfung

- Scam-Nachrichten werden jetzt auch von Accounts erkannt, die bis zu einem Monat alt sind (vorher nur 24 Stunden)
- Einzelne Nachrichten mit verdächtigen Inhalten werden per KI automatisch auf Scam geprüft — kein Multi-Channel-Spam mehr nötig
- Neue Accounts werden bei KI-bestätigtem Scam automatisch gebannt
- Ältere, etablierte Accounts (möglicherweise gehackt) werden nicht automatisch gebannt — stattdessen erscheint ein Mod-Vorschlag mit Ban-Button im Mod-Channel
- Das bisherige Join-Zeitfenster als Bedingung wurde entfernt

## #17 — TempVoice greift nicht mehr in fremde Voice-Kategorien ein

- Channels außerhalb der TempVoice-Kategorien (Chill, Comp, Street Brawl) werden nicht mehr umbenannt oder gelöscht
- Betrifft z.B. Custom Games oder andere manuelle Voice-Channels
- Verhindert, dass Channels mit Namen wie "Lane 1" in falschen Kategorien fälschlicherweise als TempVoice-Lane erkannt werden

## #16 — FAQ-Bot antwortet automatisch in neuen Tickets

- Wenn in der Support-Kategorie ein neues Ticket aufgemacht wird, analysiert der FAQ-Bot die erste Nachricht des Users
- Hat der Bot eine passende Antwort aus der Dokumentation, antwortet er direkt im Ticket
- Kann der Bot das Problem nicht lösen, schreibt er gar nichts – der Mensch übernimmt dann wie gewohnt
- Gilt für alle Channels die mit "ticket-" beginnen in der ❓Support-Kategorie

## #15 — FAQ-Bot erkennt Onboarding- und Invite-Probleme automatisch

- FAQ-Bot gibt jetzt bei "kein Invite" oder "kann nicht herunterladen" sofort eine Schritt-für-Schritt-Anleitung aus
- Checkliste: Onboarding abgeschlossen? Richtige Option gewählt? Rollen gesetzt? /betainvite verwendet? Steam-Kauf vorhanden?
- Bot weist freundlich aber klar darauf hin wenn das Onboarding-Lesen das Problem gelöst hätte
- Neue Dokumentationsdatei mit dem vollständigen Invite-Ablauf für die Bot-Wissensbasis

## #14 — Tracking-Invite und Auswertung für Website-Joins

- Bot legt automatisch einen permanenten Tracking-Invite an und merkt sich den Code in der DB
- Neuer Slash-Befehl `/website-invite` zeigt Status, Code und bisherige Nutzungen
- `/website-invite-recreate` erzeugt bei Bedarf einen neuen Code (z.B. wenn jemand den alten löscht)
- `/join-quellen [tage]` aggregiert die Member-Joins der letzten N Tage nach Quelle (Website, Vanity, Twitch-Streamer, persönliche Einladungen)
- Channel-Default ist der Welcome-Channel, kann via `WEBSITE_INVITE_CHANNEL_ID` Env-Var überschrieben werden

## #13 — TempVoice Sweep löscht keine Staging-Channels mehr

- Staging-Channels sind jetzt gegen den automatischen Sweep geschützt
- Hintergrund: Der Bot hatte den alten Chill Lanes Staging-Channel selbst gelöscht, weil dessen Name einem Lane-Muster entsprach

## #12 — Chill Lanes Staging Channel auf neuen Channel aktualisiert

- Der Staging-Channel für Chill Lanes wurde nach dem Löschen des alten Channels auf den neuen Channel umgestellt
- Alle betroffenen Stellen im Code wurden aktualisiert: TempVoice, LFG und User-Retention-Links

## #11 — Steam-Verknüpfung: Link wird jetzt immer frisch beim Klick erstellt

- Der „Via Steam verknüpfen"-Button im Onboarding generiert den Login-Link jetzt erst beim Klicken – nicht mehr beim Laden der Nachricht
- Dadurch können Links nicht mehr ablaufen, bevor jemand draufklickt
- Das Problem „invalid Launch" bei der Steam-Verifizierung ist damit behoben

## #10 — Vollständige Security-Fortress hinzugefügt

- Neuer Security-Scan läuft täglich: prüft Workflow-Integrität, findet Secrets im Code, scannt Python auf Sicherheitslücken und bekannte CVEs in Dependencies
- Alle Workflow-Dateien sind jetzt auf genaue Commit-Hashes gepinnt — kein Supply-Chain-Angriff über gemutete Action-Tags möglich
- JavaScript-Abhängigkeiten werden auf bekannte Schwachstellen geprüft
- Security-Fortress ergänzt den bestehenden Deep-Scan sinnvoll, ohne Laufzeit zu verschwenden

## #9 — Dependabot-PRs werden jetzt automatisch gemerged + CI-Laufzeiten halbiert

- Dependabot-PRs werden ab jetzt automatisch approved und direkt gemerged (nicht mehr blockiert durch Lint oder DAST)
- Lint und DAST-Scans überspringen Dependabot-PRs, da sie nur Dependency-Dateien ändern — kein Sicherheitsverlust
- Security-Scans laufen weiterhin täglich; Container/IaC/Supply-Chain scannen jetzt wöchentlich statt täglich
- Security-Incident-Automation läuft jetzt täglich statt alle 6 Stunden — 75 % weniger Runs
- Dependency-Review hat keinen sinnlosen Tages-Schedule mehr (läuft weiterhin auf jedem PR)

## #8 — CI-Artifacts werden nach 30 Tagen automatisch gelöscht

- Alle automatisch erzeugten CI-Berichte (Security-Scans, Performance-Reports, Logs) werden ab jetzt nach 30 Tagen automatisch von GitHub entfernt
- Verhindert, dass sich der GitHub-Actions-Speicher dauerhaft volläuft

## #7 — Steam-Bot startet wieder und updated Ränge

- Steam-Bot lief seit dem 27. April nicht mehr — FAs annehmen/senden und Rang-Updates funktionierten nicht
- Ursache: Drei kombinierte Bugs verhinderten den Start (falscher Pfad auf Linux, fehlende Env-Variablen für den Node-Prozess, veraltete native Module)
- Der Bot läuft jetzt stabil und verarbeitet wieder Rang-Checks und Freundschaftsanfragen

## #6 — DB-Pfad fest im Code, kein Datenverlust mehr bei Neustarts

- Der Bot nutzt jetzt immer `data/deadlock.sqlite3` direkt im Repo — egal welche Umgebungsvariablen gesetzt sind oder nicht
- Davor: Nach der Token-Rotation fehlte die `DEADLOCK_DB_PATH`-Variable → Bot startete mit einer leeren Fallback-DB → alle User wirkten wie Neulinge (kein Voice-Verlauf, keine Steam-Links)
- Die fehlenden 2.5 Tage Daten (276 Voice-Sessions, Steam-Links, Nudge-Status etc.) wurden in die Haupt-DB zurückgespielt

## #5 — Steam-Nudge-DM geht nicht mehr mehrfach an denselben User

- Wenn die ursprüngliche Nudge-DM gelöscht wurde (z. B. vom User selbst), schickt der Bot keine zweite DM mehr — die Nachricht ist weg, die Benachrichtigung bleibt trotzdem gesetzt
- Fehlschläge beim Speichern des „bereits benachrichtigt"-Flags werden jetzt im Log sichtbar, statt still ignoriert zu werden

## #4 — Tag-System: bessere Sortierung in Voice-Lanes und LFG

- Du kannst dir jetzt selbst zwei Tags setzen: deinen Lieblings-Ton (Banter-OK oder Ragebaiter-Free) und optional eine Altersangabe (25+ oder U25)
- Setzen geht entweder direkt im Onboarding nach dem Server-Join oder jederzeit per `/meine-tags`
- Voice-Lane-Owner können in ihrer Lane einen 🛡️ Tag-Filter setzen, damit nur Leute mit passendem Ton oder Alter joinen können
- Wer wiederholt Ragebait fährt, bekommt automatisch einen Ragebaiter-Mod-Tag (14 Tage), der ihn aus Ragebaiter-Free-Lanes raushält — Mods können den Tag jederzeit anpassen
- LFG-Suche kann jetzt auch nach Tags filtern, damit Mitspieler besser zur eigenen Stimmung passen

## #3 — Tierlist-Backend: WR-Daten alle 8 Stunden automatisch

- Neuer Service liefert die Hero-Tierliste der Website mit Live-Winrates pro Skill-Bucket
- Daten werden alle 8 Stunden automatisch aktualisiert, immer auf Basis des aktuellen Patches
- Drei Skill-Buckets verfügbar: All, Phantom+, Eternus
- Admin-Endpunkte für Beschreibungen, Streamer-Listen und Schwellen — Login über bestehenden Discord-Flow
- Build-Voting (👍 / 👎) mit Rate-Limit pro Browser

## #2 — Voice Feedback geht nicht mehr an bestehende User nach Bot-Neustart

- Nutzer, die beim Neustart bereits im Voice-Call saßen, bekommen kein fälschliches „erstes Mal"-Feedback mehr
- Prüfung erweitert: beide Tabellen (voice_stats und voice_session_log) werden gecheckt, nicht nur eine
- Feedback-Retry bei DMs-deaktiviert läuft jetzt nicht mehr ewig: der ursprüngliche Zeitstempel bleibt erhalten und fällt nach 72 Stunden aus dem Fenster

## #1 — Sicherheitslücke: Bot-API nicht mehr von außen erreichbar

- Der interne Statistik-Server (Port 8768) war versehentlich von außen direkt erreichbar
- Jetzt lauscht er nur noch auf localhost — externer Zugriff ohne Caddy nicht mehr möglich
