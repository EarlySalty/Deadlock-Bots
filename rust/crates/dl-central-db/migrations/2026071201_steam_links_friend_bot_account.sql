-- Welcher Bot-Account ist mit diesem verknüpften Nutzer befreundet?
--
-- Bisher war `is_steam_friend` ein einzelnes Boolean: "irgendein Bot ist mit
-- diesem Nutzer befreundet". Das genügte, solange es genau einen Bot gab. Mit
-- einem zweiten Bot-Account ist die Frage "WELCHER Bot?" nicht mehr ableitbar —
-- und ohne die Antwort würde Bot 1 die Freunde von Bot 2 als "unbekannt" purgen,
-- den Rang über den falschen (nicht befreundeten) Bot lesen und neue Anfragen
-- blind an einen vollen Account schicken.
--
-- `friend_bot_account_id` trägt genau diese Zuordnung. NULL heißt "mit keinem
-- Bot befreundet" (Link existiert, aber is_steam_friend=false). Bewusst NICHT
-- DEFAULT 1: ein nicht befreundeter Link gehört keinem Bot, und ein Default
-- würde diese Lüge in jede neue Zeile schreiben.
ALTER TABLE core.steam_links
    ADD COLUMN IF NOT EXISTS friend_bot_account_id SMALLINT;

-- Backfill: jede HEUTE bestehende Freundschaft gehört historisch Bot 1 — der
-- war bis zu dieser Migration der einzige Bot, der Anfragen versenden konnte.
UPDATE core.steam_links
    SET friend_bot_account_id = 1
    WHERE is_steam_friend = true
      AND friend_bot_account_id IS NULL;

-- "Wie viele Freunde hat Bot N?" (Lastverteilung) und "alle Freunde von Bot N"
-- (Sync/Purge/Rang pro Bot) sind die beiden heißen Pfade — beide filtern auf
-- friend_bot_account_id und leben nur für aktive Freundschaften.
CREATE INDEX IF NOT EXISTS steam_links_friend_bot_idx
    ON core.steam_links (friend_bot_account_id)
    WHERE is_steam_friend = true;
