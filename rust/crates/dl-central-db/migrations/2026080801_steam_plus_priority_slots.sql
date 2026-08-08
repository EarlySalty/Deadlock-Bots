-- Plus-Tier des Steam-Bots: priorisierte Steam-Accounts ("Prio-Slots").
--
-- Ein Supporter markiert bis zu drei seiner verknuepften Steam-Accounts. Das
-- Verknuepfen selbst bleibt unbegrenzt (core.steam_links kennt kein Limit und
-- bekommt hier auch keines); gedeckelt ist ausschliesslich die Zahl der Slots.
-- Deshalb eine eigene Tabelle statt einer Spalte auf core.steam_links: die
-- Verknuepfung ist Bestandsdaten, der Slot ist Abo-Semantik, und beides in
-- einer Tabelle zu fuehren waere genau die Vermischung, die die Grenze
-- versehentlich auf das Verknuepfen ausdehnen wuerde.
--
-- WIRKUNG HEUTE: keine ausserhalb dieser Zeile. Es gibt im gesamten Oekosystem
-- keinen belegten Endpunkt der Deadlock API, an den eine Priorisierung
-- gemeldet werden koennte (RESEARCH-TECHNIK.md §3: kein Endpoint, keine Auth,
-- keine Doku). Der Slot ist vorerst reine Markierung in der eigenen Datenbank;
-- der Meldeweg wird angehaengt, sobald er extern erfragt ist. Bis dahin darf
-- kein Text den Perk als sofort verfuegbar darstellen.
--
-- Kein Fremdschluessel auf core.steam_links: der Leave-/Reconcile-Pfad
-- archiviert Links und loescht sie aus core.steam_links
-- (steam-persistence/src/links.rs, restore_reconcile_links holt sie zurueck).
-- Ein ON DELETE CASCADE wuerde die Slots dabei still mitnehmen, waehrend die
-- Verknuepfung selbst wiederkommt. Slot-Zeilen ueberleben deshalb bewusst;
-- sichtbar wird ein Slot erst ueber den Join im Lesepfad.
--
-- Die Zahl 3 steht NICHT in dieser Migration. Sie gehoert in genau eine
-- Pruefung (steam-flows/src/plus/slots.rs, MAX_PRIORITY_SLOTS); eine zweite
-- Formulierung als CHECK-Constraint waere ein Zwilling, der beim Aendern
-- auseinanderlaeuft.

CREATE TABLE IF NOT EXISTS steam.plus_priority_slots (
    discord_id BIGINT NOT NULL,
    -- Gleiche Schreibweise wie core.steam_links.steam_id (TEXT, SteamID64).
    steam_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Der Primaerschluessel ist zugleich die Idempotenz-Schranke: derselbe
    -- Account zweimal markiert bleibt ein Slot.
    PRIMARY KEY (discord_id, steam_id)
);

-- Rueckweg vom Account zum Besitzer (Diagnose, spaeterer Meldeweg).
CREATE INDEX IF NOT EXISTS plus_priority_slots_steam_id_idx
    ON steam.plus_priority_slots (steam_id);
