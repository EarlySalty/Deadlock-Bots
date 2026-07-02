-- Blockt konkurrierende Writer fuer die Dauer der Migrations-TX, damit zwischen Dedupe und Index-Erstellung kein neues Duplikat entstehen kann; SHARE ROW EXCLUSIVE konfligiert mit ROW EXCLUSIVE und erlaubt reine SELECTs.
LOCK TABLE core.steam_links IN SHARE ROW EXCLUSIVE MODE;

WITH ranked_primary_accounts AS (
    SELECT discord_id,
           steam_id,
           ROW_NUMBER() OVER (
               PARTITION BY discord_id
               ORDER BY verified DESC,
                        updated_at DESC NULLS LAST,
                        linked_at DESC NULLS LAST,
                        steam_id ASC
           ) AS rn
      FROM core.steam_links
     WHERE primary_account
       AND discord_id <> 0
)
UPDATE core.steam_links AS steam_links
   SET primary_account = false
  FROM ranked_primary_accounts
 WHERE steam_links.discord_id = ranked_primary_accounts.discord_id
   AND steam_links.steam_id = ranked_primary_accounts.steam_id
   AND ranked_primary_accounts.rn > 1;

CREATE UNIQUE INDEX IF NOT EXISTS uq_steam_links_one_primary
    ON core.steam_links (discord_id)
    WHERE primary_account AND discord_id <> 0;
