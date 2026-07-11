-- Jeder Steam-Task gehört genau einem Bot-Account.
--
-- Bisher claimte `next_pending_task` den ältesten PENDING-Task, ohne zu fragen,
-- WER ihn ausführen soll. Das ging gut, solange es genau einen Steam-Client gab.
-- Mit einem zweiten Bot-Account wäre es ein Datenverlust-Bug: `FOR UPDATE SKIP
-- LOCKED` verhindert nur, dass zwei Instanzen DENSELBEN Task greifen — nicht,
-- dass die FALSCHE Instanz ihn greift. Bot 2 würde die Playtest-Einladungen von
-- Bot 1 mit seinem eigenen Account verschicken (der als frischer Account gar
-- nicht einladen darf), und niemand sähe den Grund.
--
-- Default 1 = alles Bestehende gehört Bot 1. Im Ein-Bot-Betrieb ändert sich
-- nichts; der Filter ist dann eine Tautologie.
ALTER TABLE steam.steam_tasks
    ADD COLUMN IF NOT EXISTS bot_account_id SMALLINT NOT NULL DEFAULT 1;

-- Der Claim sucht "ältester PENDING-Task MEINES Bots" — ohne diesen Index
-- degeneriert er bei wachsender Task-Historie zum Full Scan.
CREATE INDEX IF NOT EXISTS steam_tasks_pending_by_bot_idx
    ON steam.steam_tasks (bot_account_id, id)
    WHERE status = 'PENDING';
